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
pub fn in_slice(cgroup_file: &str) -> bool {
    cgroup_file
        .lines()
        .any(|l| l.split("::").nth(1).is_some_and(|p| p.contains(&format!("/{SLICE}/")) || p.ends_with(&format!("/{SLICE}"))))
}

pub fn busctl_adopt_argv(pid: u32) -> Vec<String> {
    [
        "busctl", "--user", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
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

pub fn adopt(c: &Candidate) -> anyhow::Result<()> {
    let argv = busctl_adopt_argv(c.pid);
    let out = Command::new(&argv[0]).args(&argv[1..]).output()?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

/// Scan every 2 s; adopt what's new; report each attempt once per pid.
pub fn subscription(patterns: Vec<String>) -> Subscription<AdoptEvent> {
    Subscription::run_with(patterns, run)
}

fn run(patterns: &Vec<String>) -> iced::futures::stream::BoxStream<'static, AdoptEvent> {
    let patterns = patterns.clone();
    let uid = crate::ipc::uid();
    let (mut tx, rx) = mpsc::channel::<AdoptEvent>(16);
    let worker = async move {
        let mut seen: HashSet<u32> = HashSet::new();
        loop {
            for c in scan(&patterns, uid) {
                if !seen.insert(c.pid) {
                    continue;
                }
                let result = adopt(&c).map_err(|e| format!("{e:#}"));
                let _ = tx.send(AdoptEvent { pid: c.pid, name: c.name.clone(), result }).await;
            }
            // Forget pids that are gone so a reused pid can be adopted again.
            seen.retain(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists());
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

    #[test]
    fn busctl_argv_moves_one_pid_into_a_scope_under_the_slice() {
        let a = busctl_adopt_argv(4242);
        assert_eq!(
            a,
            vec![
                "busctl", "--user", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
                "org.freedesktop.systemd1.Manager", "StartTransientUnit", "ssa(sv)a(sa(sv))",
                "yutani-eve-adopt-4242.scope", "fail", "3",
                "PIDs", "au", "1", "4242",
                "Slice", "s", "yutani-eve.slice",
                "CollectMode", "s", "inactive-or-failed",
                "0",
            ]
        );
    }
}
