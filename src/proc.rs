//! Bounding a child process in wall-clock time, which `std::process` does
//! not offer.
//!
//! Needed because `busctl --timeout=N` bounds only the *method-call reply*
//! phase: a unix socket that accepts a connection but never speaks D-Bus
//! leaves `busctl` stuck in connect/authentication for 90 s or more, and
//! `busctl status` ignores `--timeout` entirely. Anything the UI thread (or
//! a game launch) waits on therefore needs a timeout around the process, not
//! inside the protocol.

use std::io::{self, Read as _};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How often `wait_or_kill` asks whether the child has finished. Small
/// enough that a fast command is not noticeably delayed, large enough that
/// waiting out a multi-second timeout costs nothing measurable.
const POLL: Duration = Duration::from_millis(25);

/// Wait for `child` for at most `timeout`; `SIGKILL` it if it outlives that.
/// Returns whether it was killed. The child is left un-reaped either way —
/// the caller still has to `wait` it (not `wait_with_output`: that also
/// tries to drain the pipes, which is exactly what `output_with_timeout`
/// must not do blindly on the timeout path — see its doc comment).
///
/// `try_wait` is polled rather than watched, so there is a boundary race: a
/// child that happens to exit in the instant between one poll and the
/// deadline can still be reported as killed (`Ok(true)`) even though it was
/// already on its way out. Harmless — `kill` on an already-exited pid is a
/// no-op as far as the caller is concerned, since the child is dead either
/// way.
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
/// after `timeout` and has been killed.
///
/// Stdout and stderr are drained on two background threads started right
/// after spawn, not after the child exits — reading only after exit is
/// `std::process::Command::output`'s own well-known pipe-buffer trap: a
/// child that writes more than one pipe buffer (64 KiB on Linux) before
/// exiting would block on the write, never exit, and just run out the
/// timeout instead of completing.
///
/// The two paths out of this function treat those reader threads
/// differently, and that difference is the point:
///
/// - On the **success** path, both threads are `join`ed. The pipe's write
///   end closes when the child (and anything that inherited the fd) exits,
///   so the reads reach EOF and the joins return promptly in the ordinary
///   case. A child that forks a descendant which keeps the inherited fd
///   open past the child's own exit can still delay this join — that is
///   inherent to inherited file descriptors and invisible from here.
/// - On the **timeout** path, the threads are *not* joined: after `kill`,
///   a descendant that inherited the pipe can still hold it open
///   indefinitely, and a reader blocked reading from it would never return
///   — joining it would turn a bounded timeout into an unbounded wait,
///   defeating the entire point of this function. Instead we reap the
///   child with a plain `wait()` (no pipe I/O) and return `Ok(None)` at
///   once, leaving the reader threads to finish whenever their pipe end
///   finally closes; they hold nothing but that fd, so an abandoned one
///   costs nothing.
pub fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> io::Result<Option<Output>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut stdout = child.stdout.take().expect("stdout was requested as piped");
    let mut stderr = child.stderr.take().expect("stderr was requested as piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let killed = wait_or_kill(&mut child, timeout)?;
    let status = child.wait()?;
    if killed {
        // Do not join: a surviving descendant could hold the pipe open
        // forever. Let the readers finish on their own time.
        return Ok(None);
    }
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Some(Output { status, stdout, stderr }))
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

    #[test]
    fn a_child_writing_more_than_one_pipe_buffer_does_not_deadlock() {
        // 200 KiB comfortably exceeds a 64 KiB pipe buffer: if stdout were
        // only drained after `wait_or_kill` returns, the child would block on
        // the write, never exit, and this would time out instead of
        // finishing well within it.
        let started = Instant::now();
        let out = output_with_timeout(Command::new("head").args(["-c", "200000", "/dev/zero"]), Duration::from_secs(10))
            .unwrap()
            .expect("must complete, not look like a timeout");
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 200_000);
        assert!(started.elapsed() < Duration::from_secs(5), "concurrent draining must not wait out the timeout");
    }
}
