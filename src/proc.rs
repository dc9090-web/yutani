//! Bounding a child process in wall-clock time, which `std::process` does
//! not offer.
//!
//! Needed because `busctl --timeout=N` bounds only the *method-call reply*
//! phase: a unix socket that accepts a connection but never speaks D-Bus
//! leaves `busctl` stuck in connect/authentication for 90 s or more, and
//! `busctl status` ignores `--timeout` entirely. Anything the UI thread (or
//! a game launch) waits on therefore needs a timeout around the process, not
//! inside the protocol.

use std::io;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How often `wait_or_kill` asks whether the child has finished. Small
/// enough that a fast command is not noticeably delayed, large enough that
/// waiting out a multi-second timeout costs nothing measurable.
const POLL: Duration = Duration::from_millis(25);

/// Wait for `child` for at most `timeout`; `SIGKILL` it if it outlives that.
/// Returns whether it was killed. The child is left un-reaped either way —
/// the caller still has to `wait`/`wait_with_output` it.
fn wait_or_kill(child: &mut Child, timeout: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            return Ok(true);
        }
        std::thread::sleep(POLL);
    }
}

/// `Command::output`, bounded: `Ok(None)` means the child was still running
/// after `timeout` and has been killed and reaped.
///
/// The child's pipes are only drained after it exits, so a command that
/// writes more than a pipe buffer (64 KiB) without exiting blocks and is
/// then killed by the timeout — which is the same outcome the caller wants
/// from a command that will not finish. Every caller here produces a few
/// hundred bytes at most.
pub fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> io::Result<Option<Output>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let killed = wait_or_kill(&mut child, timeout)?;
    let output = child.wait_with_output()?;
    Ok((!killed).then_some(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_that_outlives_the_timeout_is_killed_and_reaped() {
        let mut child = Command::new("sleep").arg("10").spawn().unwrap();
        let started = Instant::now();
        assert!(wait_or_kill(&mut child, Duration::from_millis(100)).unwrap(), "must report the kill");
        assert!(started.elapsed() < Duration::from_secs(2), "must not wait for the child's own 10 s");
        // `kill` only *sends* SIGKILL, so give the kernel a moment; the point
        // is that the child really is gone, not that it died instantly.
        let exit = (0..200).find_map(|_| match child.try_wait().unwrap() {
            Some(status) => Some(status),
            None => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        });
        assert!(exit.is_some(), "the timed-out child must be gone");
    }

    #[test]
    fn a_command_that_finishes_in_time_yields_its_output() {
        let out = output_with_timeout(Command::new("echo").arg("hi"), Duration::from_secs(5)).unwrap().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn a_command_that_hangs_times_out_instead_of_blocking_the_caller() {
        let started = Instant::now();
        let out = output_with_timeout(Command::new("sleep").arg("10"), Duration::from_millis(100)).unwrap();
        assert!(out.is_none(), "a child that outlived the timeout must not look like a successful run");
        assert!(started.elapsed() < Duration::from_secs(2), "the caller must not wait for the child");
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error_not_a_timeout() {
        let err = output_with_timeout(&mut Command::new("/nonexistent-yutani-binary"), Duration::from_secs(1));
        assert!(err.is_err(), "a spawn failure must stay distinguishable from a timeout");
    }
}
