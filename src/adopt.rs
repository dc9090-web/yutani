//! Auto-adoption: move running EVE processes into `yutani-eve.slice` so the
//! tunnel rules apply to them even when they weren't started through
//! `yutani launch`. Uses the user's own systemd manager (no root).

use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::tunnel::SLICE;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub pid: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct AdoptEvent {
    pub pid: u32,
    pub name: String,
    pub result: Result<(), String>,
}

/// `exe_name` is `/proc/<pid>/comm` or a cmdline argv[0] (Windows or Unix
/// path); matches when its basename equals one of `patterns`, ignoring case.
pub fn is_eve_process(exe_name: &str, patterns: &[String]) -> bool {
    let base = exe_name.rsplit(['/', '\\']).next().unwrap_or(exe_name);
    patterns.iter().any(|p| p.eq_ignore_ascii_case(base))
}

/// `/proc/<pid>/cgroup` (cgroup v2: one `0::/path` line) is under our slice.
/// cgroup-v2-only: a hybrid or v1 host also has a `name=systemd` line for the
/// same process, which this deliberately ignores — the `0::` prefix is what
/// `.split("::").nth(1)` requires, so any v1 line just fails to match.
pub fn in_slice(cgroup_file: &str) -> bool {
    cgroup_file
        .lines()
        .any(|l| l.split("::").nth(1).is_some_and(|p| p.contains(&format!("/{SLICE}/")) || p.ends_with(&format!("/{SLICE}"))))
}

/// `--timeout=5` bounds only the *method-call reply* phase; a socket that
/// accepts but never speaks D-Bus would still hang `busctl` in connect and
/// authentication for far longer. [`ADOPT_TIMEOUT`] is what actually bounds
/// the call, by killing the process.
pub fn busctl_adopt_argv(pid: u32) -> Vec<String> {
    [
        "busctl", "--user", "--timeout=5", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager", "StartTransientUnit", "ssa(sv)a(sa(sv))",
        &format!("yutani-eve-adopt-{pid}.scope"), "fail", "3",
        "PIDs", "au", "1", &pid.to_string(),
        "Slice", "s", SLICE,
        "CollectMode", "s", "inactive-or-failed",
        "0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// EVE processes owned by `uid` that are not yet in the slice, and every
/// pid that was alive during the walk (all users: it feeds
/// [`Tracker::retain_live`], and a second `/proc` walk per tick just for
/// that would double the syscalls).
pub fn scan(patterns: &[String], uid: u32) -> (Vec<Candidate>, HashSet<u32>) {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir("/proc") else { return (Vec::new(), HashSet::new()) };
    let mut out = Vec::new();
    let mut live = HashSet::new();
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        live.insert(pid);
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != uid {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        let name = if is_eve_process(comm.trim(), patterns) {
            comm.trim().to_string()
        } else {
            // `comm` is truncated to 15 bytes; only then is argv[0] worth
            // the extra read (a few hundred processes per tick otherwise).
            let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
            let argv0 = cmdline.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            if !is_eve_process(&argv0, patterns) {
                continue;
            }
            argv0.rsplit(['/', '\\']).next().unwrap_or(&argv0).to_string()
        };
        let cgroup = std::fs::read_to_string(entry.path().join("cgroup")).unwrap_or_default();
        if in_slice(&cgroup) {
            continue;
        }
        out.push(Candidate { pid, name });
    }
    (out, live)
}

/// Wall-clock bound on one adoption call. A timeout is a failure like any
/// other, so it feeds the same per-pid backoff.
pub const ADOPT_TIMEOUT: Duration = Duration::from_secs(5);

/// Moves only `c.pid` into the slice — not any descendants it may already
/// have spawned (e.g. helper processes forked before adoption ran). Those
/// stay wherever they are unless they are themselves scanned and matched.
pub fn adopt(c: &Candidate) -> anyhow::Result<()> {
    let argv = busctl_adopt_argv(c.pid);
    let out = yutani::proc::output_with_timeout(Command::new(&argv[0]).args(&argv[1..]), ADOPT_TIMEOUT)?
        .ok_or_else(|| anyhow::anyhow!("busctl did not answer within {ADOPT_TIMEOUT:?}"))?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

/// When a pid that has failed `failures` times may be attempted again.
#[derive(Clone, Copy, Debug)]
struct Backoff {
    failures: u32,
    next: Instant,
}

/// Bookkeeping for repeated adoption attempts across scan ticks. A pid only
/// joins `seen` (and is never attempted again) once it is actually adopted;
/// a failing pid is retried, because the failure may be transient (the user
/// bus hiccuping), but with an exponential backoff — retrying every 2 s
/// forever means a wedged bus costs a bounded-but-real amount of work per
/// pid every tick, and each attempt can take up to [`ADOPT_TIMEOUT`]. It is
/// warned about only once, via `warned`, so a persistent failure doesn't
/// spam one `AdoptEvent` per tick.
#[derive(Default)]
pub struct Tracker {
    seen: HashSet<u32>,
    warned: HashSet<u32>,
    backoff: HashMap<u32, Backoff>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// How long to wait after the `failures`-th consecutive failure:
    /// 2 s, 4 s, 8 s, … capped at 60 s.
    pub fn backoff_after(failures: u32) -> Duration {
        Duration::from_secs(2u64.saturating_pow(failures.min(16)).min(60))
    }

    /// Should `pid` be attempted at `now`? False once it has been adopted,
    /// and false while it is backing off from a recent failure.
    pub fn should_attempt_at(&self, pid: u32, now: Instant) -> bool {
        !self.seen.contains(&pid) && self.backoff.get(&pid).is_none_or(|b| now >= b.next)
    }

    /// [`Self::should_attempt_at`] with the real clock.
    pub fn should_attempt(&self, pid: u32) -> bool {
        self.should_attempt_at(pid, Instant::now())
    }

    /// Record the outcome of attempting `pid` at `now`. Returns whether an
    /// `AdoptEvent` should be emitted: always on success; on failure, only
    /// the first time (subsequent retries of the same still-failing pid
    /// stay silent until it either succeeds or dies).
    pub fn note_at(&mut self, pid: u32, ok: bool, now: Instant) -> bool {
        if ok {
            self.seen.insert(pid);
            self.warned.remove(&pid);
            self.backoff.remove(&pid);
            true
        } else {
            let failures = self.backoff.get(&pid).map_or(0, |b| b.failures) + 1;
            self.backoff.insert(pid, Backoff { failures, next: now + Self::backoff_after(failures) });
            self.warned.insert(pid)
        }
    }

    /// [`Self::note_at`] with the real clock.
    pub fn note(&mut self, pid: u32, ok: bool) -> bool {
        self.note_at(pid, ok, Instant::now())
    }

    /// Drop bookkeeping for pids that are no longer alive, so a reused pid
    /// starts fresh and a since-fixed pid can be retried without limit.
    pub fn retain_live(&mut self, live: &HashSet<u32>) {
        self.seen.retain(|pid| live.contains(pid));
        self.warned.retain(|pid| live.contains(pid));
        self.backoff.retain(|pid, _| live.contains(pid));
    }
}


/// Scan every 2 s; adopt what's new; report each pid's outcome once
/// (successes always, failures only until warned — see [`Tracker`]).
pub fn subscription(patterns: Vec<String>) -> Subscription<AdoptEvent> {
    Subscription::run_with(patterns, run)
}

/// Run `f` on tokio's blocking pool. The subscription's future shares the
/// executor thread with the whole UI (`cosmic::executor::Default` is a
/// one-worker tokio runtime), so the `/proc` walk and every `busctl` call
/// have to leave it — otherwise a slow bus freezes the thumbnails.
async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("adoption task failed: {e}"))
}

#[allow(clippy::ptr_arg)] // signature dictated by `Subscription::run_with`
fn run(patterns: &Vec<String>) -> iced::futures::stream::BoxStream<'static, AdoptEvent> {
    let patterns = std::sync::Arc::new(patterns.clone());
    let uid = crate::ipc::uid();
    let (mut tx, rx) = mpsc::channel::<AdoptEvent>(16);
    let worker = async move {
        let mut tracker = Tracker::new();
        loop {
            let p = patterns.clone();
            let (candidates, live) = off_thread(move || scan(&p, uid)).await.unwrap_or_default();
            for c in candidates {
                if !tracker.should_attempt(c.pid) {
                    continue;
                }
                let target = c.clone();
                let result = off_thread(move || adopt(&target).map_err(|e| format!("{e:#}"))).await.unwrap_or_else(Err);
                let ok = result.is_ok();
                if tracker.note(c.pid, ok) {
                    let _ = tx.send(AdoptEvent { pid: c.pid, name: c.name.clone(), result }).await;
                }
            }
            // Forget pids that are gone so a reused pid can be adopted again.
            tracker.retain_live(&live);
            futures_timer::Delay::new(Duration::from_secs(2)).await;
        }
    };
    Box::pin(iced::futures::stream::select(
        iced::futures::stream::once(worker).filter_map(|()| async { None }),
        rx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pats() -> Vec<String> {
        vec!["exefile.exe".into(), "eve-online.exe".into()]
    }

    #[test]
    fn matches_eve_executables_case_insensitively() {
        assert!(is_eve_process("exefile.exe", &pats()));
        assert!(is_eve_process("EXEFILE.EXE", &pats()));
        assert!(is_eve_process("C:\\CCP\\EVE\\tq\\bin64\\exefile.exe", &pats()));
        assert!(is_eve_process("/some/pfx/drive_c/EVE/eve-online.exe", &pats()));
        assert!(!is_eve_process("wineserver", &pats()));
        assert!(!is_eve_process("exefile.exe.old", &pats()));
    }

    #[test]
    fn detects_slice_membership_from_proc_cgroup() {
        assert!(in_slice("0::/user.slice/user-1000.slice/user@1000.service/yutani-eve.slice/yutani-eve-123.scope\n"));
        assert!(in_slice("0::/user.slice/user-1000.slice/user@1000.service/yutani-eve.slice/yutani-eve-adopt-9.scope\n"));
        assert!(!in_slice("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-cosmic-x.scope\n"));
    }

    /// systemd nests `yutani-eve.slice` under `yutani.slice` (the dash makes
    /// it a child slice), so the real `/proc/<pid>/cgroup` line has five
    /// path components, not four — verified with `systemd-run --user
    /// --scope --slice=yutani-eve.slice -- cat /proc/self/cgroup`. A sibling
    /// scope directly under the parent `yutani.slice` must not count.
    #[test]
    fn detects_slice_membership_under_the_real_nested_parent_slice() {
        assert!(in_slice(
            "0::/user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice/yutani-eve-adopt-9.scope\n"
        ));
        assert!(!in_slice(
            "0::/user.slice/user-1000.slice/user@1000.service/yutani.slice/other.scope\n"
        ));
    }

    #[test]
    fn busctl_argv_moves_one_pid_into_a_scope_under_the_slice() {
        let a = busctl_adopt_argv(4242);
        assert_eq!(
            a,
            vec![
                "busctl", "--user", "--timeout=5", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
                "org.freedesktop.systemd1.Manager", "StartTransientUnit", "ssa(sv)a(sa(sv))",
                "yutani-eve-adopt-4242.scope", "fail", "3",
                "PIDs", "au", "1", "4242",
                "Slice", "s", "yutani-eve.slice",
                "CollectMode", "s", "inactive-or-failed",
                "0",
            ]
        );
    }

    #[test]
    fn scan_reports_every_live_pid_in_the_same_walk() {
        let (_, live) = scan(&pats(), crate::ipc::uid());
        assert!(live.contains(&std::process::id()), "the live set must cover the caller itself");
        assert!(live.contains(&1), "and pids of other users (init), for retain_live");
    }

    #[test]
    fn tracker_stops_attempting_a_pid_once_adopted() {
        let mut t = Tracker::new();
        assert!(t.should_attempt(7));
        assert!(t.note(7, true), "success must emit an event");
        assert!(!t.should_attempt(7));
    }

    #[test]
    fn tracker_retries_a_failing_pid_but_warns_only_once() {
        let t0 = std::time::Instant::now();
        let mut t = Tracker::new();
        assert!(t.note_at(7, false, t0), "first failure must emit an event");
        assert!(t.should_attempt_at(7, at(t0, 2)), "a failed pid must still be retried, once it has backed off");
        assert!(!t.note_at(7, false, at(t0, 2)), "repeated failure must stay silent");
        assert!(t.should_attempt_at(7, at(t0, 6)));
        // It eventually succeeds: emits again, and stops being attempted.
        assert!(t.note_at(7, true, at(t0, 6)));
        assert!(!t.should_attempt_at(7, at(t0, 60)));
    }

    #[test]
    fn tracker_warns_again_after_a_success_then_a_fresh_failure() {
        let mut t = Tracker::new();
        assert!(t.note(7, false));
        assert!(!t.note(7, false));
        assert!(t.note(7, true));
        // Simulate the pid dying and a new process reusing it.
        t.retain_live(&HashSet::new());
        assert!(t.note(7, false), "a fresh failure after the pid's bookkeeping was pruned must emit again");
    }

    #[test]
    fn tracker_retain_live_drops_dead_pids_from_both_sets() {
        let mut t = Tracker::new();
        t.note(1, true); // seen
        t.note(2, false); // warned
        t.retain_live(&HashSet::from([2]));
        assert!(t.should_attempt(1), "pid 1 died, so it should be attempted again if it reappears");
        // pid 2 is still alive and still warned; a repeat failure stays silent.
        assert!(!t.note(2, false));
    }

    /// `Instant` has no constructor, so tests anchor on one `now` and move
    /// forward from it.
    fn at(base: std::time::Instant, secs: u64) -> std::time::Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn backoff_doubles_from_two_seconds_and_stops_at_a_minute() {
        assert_eq!(Tracker::backoff_after(1), Duration::from_secs(2));
        assert_eq!(Tracker::backoff_after(2), Duration::from_secs(4));
        assert_eq!(Tracker::backoff_after(3), Duration::from_secs(8));
        assert_eq!(Tracker::backoff_after(4), Duration::from_secs(16));
        assert_eq!(Tracker::backoff_after(5), Duration::from_secs(32));
        assert_eq!(Tracker::backoff_after(6), Duration::from_secs(60), "capped, not 64");
        assert_eq!(Tracker::backoff_after(40), Duration::from_secs(60), "and it stays capped");
    }

    #[test]
    fn a_failing_pid_is_not_retried_until_its_backoff_has_passed() {
        let t0 = std::time::Instant::now();
        let mut t = Tracker::new();
        assert!(t.should_attempt_at(7, t0));
        t.note_at(7, false, t0);
        assert!(!t.should_attempt_at(7, at(t0, 1)), "2 s backoff: no retry after 1 s");
        assert!(t.should_attempt_at(7, at(t0, 2)), "retry once the 2 s have passed");
        // Second failure: 4 s, measured from when it happened.
        t.note_at(7, false, at(t0, 2));
        assert!(!t.should_attempt_at(7, at(t0, 5)));
        assert!(t.should_attempt_at(7, at(t0, 6)));
    }

    #[test]
    fn a_success_clears_the_backoff_so_a_reused_pid_is_attempted_at_once() {
        let t0 = std::time::Instant::now();
        let mut t = Tracker::new();
        t.note_at(7, false, t0);
        t.note_at(7, false, at(t0, 2));
        t.note_at(7, true, at(t0, 6));
        assert!(!t.should_attempt_at(7, at(t0, 6)), "adopted pids are never attempted again");
        t.retain_live(&HashSet::new()); // the pid died
        assert!(t.should_attempt_at(7, at(t0, 6)), "a fresh pid starts with no backoff");
    }

    #[test]
    fn retain_live_drops_the_backoff_of_dead_pids() {
        let t0 = std::time::Instant::now();
        let mut t = Tracker::new();
        t.note_at(9, false, t0);
        assert!(!t.should_attempt_at(9, t0));
        t.retain_live(&HashSet::from([1]));
        assert!(t.should_attempt_at(9, t0), "pid 9 is gone; its backoff must go with it");
    }
}
