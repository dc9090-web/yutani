//! Auto-adoption: move running EVE processes into `yutani-eve.slice` so the
//! tunnel rules apply to them even when they weren't started through
//! `yutani launch`. Uses the user's own systemd manager (no root).

use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use std::collections::HashSet;
use std::process::Command;
use std::time::Duration;

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

/// `--timeout=5` bounds how long a wedged user manager can stall the
/// executor thread that runs this (the scan loop otherwise ticks every 2 s).
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

/// EVE processes owned by `uid` that are not yet in the slice.
pub fn scan(patterns: &[String], uid: u32) -> Vec<Candidate> {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != uid {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let argv0 = cmdline.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        let name = if is_eve_process(comm.trim(), patterns) {
            comm.trim().to_string()
        } else if is_eve_process(&argv0, patterns) {
            argv0.rsplit(['/', '\\']).next().unwrap_or(&argv0).to_string()
        } else {
            continue;
        };
        let cgroup = std::fs::read_to_string(entry.path().join("cgroup")).unwrap_or_default();
        if in_slice(&cgroup) {
            continue;
        }
        out.push(Candidate { pid, name });
    }
    out
}

/// Moves only `c.pid` into the slice — not any descendants it may already
/// have spawned (e.g. helper processes forked before adoption ran). Those
/// stay wherever they are unless they are themselves scanned and matched.
pub fn adopt(c: &Candidate) -> anyhow::Result<()> {
    let argv = busctl_adopt_argv(c.pid);
    let out = Command::new(&argv[0]).args(&argv[1..]).output()?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

/// Bookkeeping for repeated adoption attempts across scan ticks. A pid only
/// joins `seen` (and is never attempted again) once it is actually adopted;
/// a failing pid is retried on every tick — the failure may be transient
/// (e.g. the user bus hiccuping) — but only warned about once, via `warned`,
/// so a persistent failure doesn't spam one `AdoptEvent` per tick.
#[derive(Default)]
pub struct Tracker {
    seen: HashSet<u32>,
    warned: HashSet<u32>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Should `pid` be attempted this tick? False once it has been adopted.
    pub fn should_attempt(&self, pid: u32) -> bool {
        !self.seen.contains(&pid)
    }

    /// Record the outcome of attempting `pid`. Returns whether an
    /// `AdoptEvent` should be emitted: always on success; on failure, only
    /// the first time (subsequent retries of the same still-failing pid
    /// stay silent until it either succeeds or dies).
    pub fn note(&mut self, pid: u32, ok: bool) -> bool {
        if ok {
            self.seen.insert(pid);
            self.warned.remove(&pid);
            true
        } else {
            self.warned.insert(pid)
        }
    }

    /// Drop bookkeeping for pids that are no longer alive, so a reused pid
    /// starts fresh and a since-fixed pid can be retried without limit.
    pub fn retain_live(&mut self, live: &HashSet<u32>) {
        self.seen.retain(|pid| live.contains(pid));
        self.warned.retain(|pid| live.contains(pid));
    }
}

fn live_pids() -> HashSet<u32> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return HashSet::new() };
    dir.flatten().filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok()).collect()
}

/// Scan every 2 s; adopt what's new; report each pid's outcome once
/// (successes always, failures only until warned — see [`Tracker`]).
pub fn subscription(patterns: Vec<String>) -> Subscription<AdoptEvent> {
    Subscription::run_with(patterns, run)
}

fn run(patterns: &Vec<String>) -> iced::futures::stream::BoxStream<'static, AdoptEvent> {
    let patterns = patterns.clone();
    let uid = crate::ipc::uid();
    let (mut tx, rx) = mpsc::channel::<AdoptEvent>(16);
    let worker = async move {
        let mut tracker = Tracker::new();
        loop {
            for c in scan(&patterns, uid) {
                if !tracker.should_attempt(c.pid) {
                    continue;
                }
                let result = adopt(&c).map_err(|e| format!("{e:#}"));
                let ok = result.is_ok();
                if tracker.note(c.pid, ok) {
                    let _ = tx.send(AdoptEvent { pid: c.pid, name: c.name.clone(), result }).await;
                }
            }
            // Forget pids that are gone so a reused pid can be adopted again.
            tracker.retain_live(&live_pids());
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
    fn tracker_stops_attempting_a_pid_once_adopted() {
        let mut t = Tracker::new();
        assert!(t.should_attempt(7));
        assert!(t.note(7, true), "success must emit an event");
        assert!(!t.should_attempt(7));
    }

    #[test]
    fn tracker_retries_a_failing_pid_but_warns_only_once() {
        let mut t = Tracker::new();
        assert!(t.note(7, false), "first failure must emit an event");
        assert!(t.should_attempt(7), "a failed pid must still be retried");
        assert!(!t.note(7, false), "repeated failure must stay silent");
        assert!(t.should_attempt(7));
        // It eventually succeeds: emits again, and stops being attempted.
        assert!(t.note(7, true));
        assert!(!t.should_attempt(7));
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
}
