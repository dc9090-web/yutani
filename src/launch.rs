//! `yutani launch -- <command…>`: run a command (Steam's `%command%`) inside
//! a scope under `yutani-eve.slice`, so every socket it and its children
//! open is routed through the tunnel from the first packet.
//!
//! `yutani launch` must never prevent the game from starting, and must never
//! run it twice. A missing `systemd-run` binary is not the only way wrapping
//! can fail: the user's D-Bus/systemd manager can also be unreachable (wrong
//! `XDG_RUNTIME_DIR`, no session bus) or wedged, and in either case
//! `systemd-run --scope` would exit non-zero *without ever starting the
//! game* — indistinguishable, from the exit code alone, from the game
//! itself failing. So before wrapping we preflight the manager with a short
//! `busctl --user --timeout=2 status`, run under [`yutani::proc`]'s
//! process-level timeout.
//!
//! The two bounds are not the same thing. `busctl`'s own `--timeout` bounds
//! only the *method-call reply* phase, and `busctl status` ignores it
//! altogether: a socket that accepts a connection but never speaks D-Bus
//! keeps `busctl` in connect/authentication for 90 s or more. What actually
//! bounds this call is [`PREFLIGHT_TIMEOUT`], after which the process is
//! killed. If the preflight fails (timeout, non-zero exit, spawn error, or
//! no stdout), we skip `systemd-run` entirely and run the command directly,
//! warning on stderr, so the game still starts exactly once.
//!
//! Once the preflight has passed, `systemd-run`'s exit status is propagated
//! as-is: `systemd-run --scope --wait`-like invocations exec the scope and
//! wait for it, so a non-zero status is the *game's own* exit, not a
//! wrapping failure, and is not retried directly (that would run the game
//! twice). One sharp edge is inherent to this: if the game binary itself is
//! missing, `systemd-run` cannot exec it and exits with its own "command not
//! found" code (1), which then looks exactly like a game that exited 1 — we
//! cannot tell those apart from here.

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitCode, ExitStatus, Output};
use std::time::Duration;

use crate::tunnel::SLICE;

/// Wall-clock bound on the preflight. The game is waiting behind it, so it
/// is deliberately short: a manager that cannot answer in two seconds is
/// treated as unreachable.
pub const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(2);

pub fn systemd_run_argv(command: &[String]) -> Vec<String> {
    let mut v: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect", &format!("--slice={SLICE}"), "--"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend(command.iter().cloned());
    v
}

/// argv for the preflight: is the user's systemd/D-Bus manager reachable at
/// all? `--timeout=2` only bounds a method call's reply (and `status`
/// ignores it); [`PREFLIGHT_TIMEOUT`] is what bounds this call.
pub fn preflight_argv() -> Vec<String> {
    ["busctl", "--user", "--timeout=2", "status"].iter().map(|s| s.to_string()).collect()
}

/// From the preflight's outcome (`None` if it timed out or could not even
/// be spawned — e.g. `busctl` itself is missing), decide whether it is safe
/// to wrap the game in `systemd-run`. A timeout, a non-zero exit, a spawn
/// error, or empty stdout (the call returned but said nothing sane) all
/// mean "no, run directly".
pub fn should_wrap(preflight: Option<&Output>) -> bool {
    preflight.is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// `Err` means `busctl` itself could not even be spawned (missing binary,
/// permission denied, ...); `Ok(None)` means it ran but did not answer
/// within [`PREFLIGHT_TIMEOUT`]; `Ok(Some(_))` means it exited within the
/// timeout (whether or not [`should_wrap`] then likes the result). Kept
/// distinct from a plain `Option` so [`preflight_warning`] can tell a
/// missing tool from a wedged manager instead of blurring both into one
/// message.
fn run_preflight() -> io::Result<Option<Output>> {
    let argv = preflight_argv();
    yutani::proc::output_with_timeout(Command::new(&argv[0]).args(&argv[1..]), PREFLIGHT_TIMEOUT)
}

/// What to print when [`should_wrap`] said no. The three causes look
/// different to someone staring at this after a game failed to launch, so
/// they get different words: `busctl` missing/unspawnable is a local setup
/// problem, a timeout is a wedged or unreachable manager, and an answer
/// `should_wrap` still rejected (non-zero exit or empty stdout) is neither.
fn preflight_warning(preflight: &io::Result<Option<Output>>) -> String {
    match preflight {
        Err(e) => format!("yutani launch: could not run busctl ({e}); running without the tunnel cgroup"),
        Ok(None) => format!(
            "yutani launch: user systemd/D-Bus manager not reachable within {PREFLIGHT_TIMEOUT:?}; running without the tunnel cgroup"
        ),
        Ok(Some(_)) => {
            "yutani launch: user systemd/D-Bus manager did not answer busctl status; running without the tunnel cgroup"
                .to_string()
        }
    }
}

/// Exit code for a finished child, POSIX-shell style: the process's own
/// exit code, or `128 + signal` if it was killed by a signal (it then has
/// no exit code at all), or `1` if somehow neither is set.
pub fn exit_code(status: ExitStatus) -> u8 {
    match (status.code(), status.signal()) {
        (Some(code), _) => code as u8,
        (None, Some(sig)) => 128u8.wrapping_add(sig as u8),
        (None, None) => 1,
    }
}

/// Never stops the game from starting: if the preflight fails, or
/// `systemd-run` is missing, run the command directly and say so on
/// stderr. See the module doc comment for the full rationale.
pub fn run(command: Vec<String>) -> ExitCode {
    if command.is_empty() {
        eprintln!("yutani launch: nothing to run (usage: yutani launch -- <command…>)");
        return ExitCode::from(2);
    }
    let preflight = run_preflight();
    let answer = preflight.as_ref().ok().and_then(|o| o.as_ref());
    if should_wrap(answer) {
        let argv = systemd_run_argv(&command);
        match Command::new(&argv[0]).args(&argv[1..]).status() {
            Ok(status) => return ExitCode::from(exit_code(status)),
            Err(err) => {
                eprintln!("yutani launch: systemd-run unavailable ({err}); running without the tunnel cgroup");
            }
        }
    } else {
        eprintln!("{}", preflight_warning(&preflight));
    }
    match Command::new(&command[0]).args(&command[1..]).status() {
        Ok(status) => ExitCode::from(exit_code(status)),
        Err(err) => {
            eprintln!("yutani launch: cannot run {}: {err}", command[0]);
            ExitCode::from(127)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(code: i32, stdout: &[u8]) -> Output {
        Output { status: ExitStatus::from_raw(code << 8), stdout: stdout.to_vec(), stderr: Vec::new() }
    }

    #[test]
    fn wraps_the_command_in_a_scope_under_the_slice() {
        let argv = systemd_run_argv(&["/path/proton".into(), "run".into(), "exefile.exe".into()]);
        assert_eq!(&argv[..7], &["systemd-run", "--user", "--scope", "--quiet", "--collect", "--slice=yutani-eve.slice", "--"]);
        assert_eq!(&argv[7..], &["/path/proton", "run", "exefile.exe"]);
    }

    #[test]
    fn preflight_checks_the_user_bus_with_a_short_timeout() {
        assert_eq!(preflight_argv(), vec!["busctl", "--user", "--timeout=2", "status"]);
        // The real bound is the process-level one: `busctl status` ignores
        // `--timeout` entirely.
        assert!(PREFLIGHT_TIMEOUT <= Duration::from_secs(2), "the game waits behind this");
    }

    #[test]
    fn a_preflight_that_times_out_is_treated_as_unreachable() {
        // `output_with_timeout` reports a timeout as `Ok(None)`, which
        // `run_preflight` flattens to `None` — the same "run directly" path
        // as a missing `busctl`.
        let timed_out: Option<Output> = yutani::proc::output_with_timeout(
            Command::new("sleep").arg("10"),
            Duration::from_millis(100),
        )
        .ok()
        .flatten();
        assert!(!should_wrap(timed_out.as_ref()), "a wedged manager must not wrap the game");
    }

    #[test]
    fn the_preflight_warning_distinguishes_a_missing_busctl_from_a_timeout_from_a_bad_answer() {
        let spawn_failed = preflight_warning(&Err(io::Error::new(io::ErrorKind::NotFound, "No such file or directory")));
        assert!(spawn_failed.contains("could not run busctl"), "got {spawn_failed}");

        let timed_out = preflight_warning(&Ok(None));
        assert!(timed_out.contains("not reachable within"), "got {timed_out}");

        let bad_answer = preflight_warning(&Ok(Some(output(1, b"nope"))));
        assert!(bad_answer.contains("did not answer"), "got {bad_answer}");

        assert_ne!(spawn_failed, timed_out);
        assert_ne!(timed_out, bad_answer);
        assert_ne!(spawn_failed, bad_answer);
    }

    #[test]
    fn should_wrap_only_when_preflight_succeeded_with_output() {
        assert!(!should_wrap(None), "spawn error must not wrap");
        assert!(!should_wrap(Some(&output(1, b"nope"))), "non-zero exit must not wrap");
        assert!(!should_wrap(Some(&output(0, b""))), "no output must not wrap");
        assert!(should_wrap(Some(&output(0, b"BusAddress=unix:path=/run/user/1000/bus\n"))));
    }

    #[test]
    fn exit_code_passes_through_a_normal_exit() {
        assert_eq!(exit_code(ExitStatus::from_raw(0 << 8)), 0);
        assert_eq!(exit_code(ExitStatus::from_raw(7 << 8)), 7);
    }

    #[test]
    fn exit_code_is_128_plus_signal_for_a_signal_kill() {
        // Raw wait status for "killed by signal 9" (SIGKILL): low 7 bits are
        // the signal number, no exit-code bits set.
        assert_eq!(exit_code(ExitStatus::from_raw(9)), 137);
        assert_eq!(exit_code(ExitStatus::from_raw(15)), 143);
    }
}
