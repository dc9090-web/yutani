//! Is an EVE process running? A `/proc` walk by executable name, for the
//! Launch EVE log (the EVE Launcher's presence) and the Characters page
//! (never copy settings files under a running client).

/// The executables that count as EVE: the game client and the launcher.
pub const EVE_PROCESSES: [&str; 3] = ["exefile.exe", "eve-online.exe", "evelauncher.exe"];

/// The EVE Launcher alone.
pub const LAUNCHER: &str = "evelauncher.exe";

/// `exe_name` is `/proc/<pid>/comm` or a cmdline argv[0] (Windows or Unix
/// path); matches when its basename equals one of `patterns`, ignoring case.
pub fn is_eve_process(exe_name: &str, patterns: &[&str]) -> bool {
    let base = exe_name.rsplit(['/', '\\']).next().unwrap_or(exe_name);
    patterns.iter().any(|p| p.eq_ignore_ascii_case(base))
}

/// Does a process with this `comm` and `argv0` match `patterns`? `comm` is
/// truncated to 15 bytes, so argv[0] is the fallback.
fn process_name_matches(comm: &str, argv0: &str, patterns: &[&str]) -> bool {
    is_eve_process(comm.trim(), patterns) || is_eve_process(argv0, patterns)
}

/// The kernel keeps `comm` to `TASK_COMM_LEN - 1` = 15 bytes, cutting a
/// longer name there. So only a `comm` of exactly 15 bytes may be a cut
/// one; a shorter `comm` is the whole name, and argv[0] cannot add a match
/// to it. (`evelauncher.exe` is itself 15 characters and matches through
/// `comm` directly.)
fn needs_argv0(comm: &str) -> bool {
    comm.trim_end_matches('\n').len() == 15
}

/// Whether the process at `dir` (`/proc/<pid>`) matches. argv[0]
/// (`cmdline`) is read only when `comm` does not match and may be
/// truncated ([`needs_argv0`]): a few hundred processes per walk otherwise.
fn matches(dir: &std::path::Path, comm: &str, patterns: &[&str]) -> bool {
    if is_eve_process(comm.trim(), patterns) {
        return true;
    }
    if !needs_argv0(comm) {
        return false;
    }
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    let argv0 = cmdline.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    process_name_matches(comm, &argv0, patterns)
}

/// Is a process owned by `uid` and matching `patterns` alive right now?
/// Stops at the first match.
pub fn process_running(patterns: &[&str], uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    dir.flatten().any(|entry| {
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            return false;
        }
        let Ok(meta) = entry.metadata() else { return false };
        if meta.uid() != uid {
            return false;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        matches(&entry.path(), &comm, patterns)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATS: [&str; 2] = ["exefile.exe", "eve-online.exe"];

    #[test]
    fn matches_eve_executables_case_insensitively() {
        assert!(is_eve_process("exefile.exe", &PATS));
        assert!(is_eve_process("EXEFILE.EXE", &PATS));
        assert!(is_eve_process("C:\\CCP\\EVE\\tq\\bin64\\exefile.exe", &PATS));
        assert!(is_eve_process("/some/pfx/drive_c/EVE/eve-online.exe", &PATS));
        assert!(!is_eve_process("wineserver", &PATS));
        assert!(!is_eve_process("exefile.exe.old", &PATS));
    }

    #[test]
    fn argv0_is_read_only_for_a_comm_the_kernel_may_have_cut() {
        assert!(needs_argv0("evelauncher.exe"), "15 bytes: may be cut");
        assert!(needs_argv0("exefile-long-na\n"), "the trailing newline does not count");
        assert!(!needs_argv0("wineserver\n"), "shorter: the whole name");
        assert!(!needs_argv0("exefile.exe"));
        assert!(!needs_argv0(""));
    }

    #[test]
    fn process_name_matches_comm_or_argv0() {
        assert!(process_name_matches("evelauncher.exe", "", &[LAUNCHER]));
        assert!(process_name_matches("wineserver", r"C:\EVE\Launcher\evelauncher.exe", &[LAUNCHER]));
        assert!(!process_name_matches("bash", "/usr/bin/bash", &[LAUNCHER]));
    }

    #[test]
    fn no_eve_process_runs_under_an_unused_uid() {
        assert!(!process_running(&EVE_PROCESSES, u32::MAX - 1));
    }
}
