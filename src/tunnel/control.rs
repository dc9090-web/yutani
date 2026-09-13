//! Unprivileged side: start/stop the unit (polkit-authorised) and read the
//! tunnel's state without root.

use anyhow::{Context as _, ensure};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::status::{TunnelFile, TunnelStatus, assemble};
use super::{IFACE, SLICE, STATUS_PATH, UNIT_NAME, UNIT_PATH};

pub fn installed() -> bool {
    std::path::Path::new(UNIT_PATH).exists()
}

/// `--no-ask-password`: without the polkit rule in place, systemctl would
/// otherwise hand the request to an authentication agent and block for as
/// long as that agent cares to wait. With it, the call fails fast and says
/// why — which the caller can show, whereas an unbounded wait it cannot.
pub fn systemctl_argv(verb: &str) -> Vec<String> {
    ["systemctl", "--no-ask-password", verb, UNIT_NAME].iter().map(|s| s.to_string()).collect()
}

/// Start `yutani-eve.slice` on the user's manager, which creates its
/// cgroup. The worker's nft ruleset names that cgroup by path, and nft
/// resolves the path against the live cgroup tree when the ruleset is
/// loaded ("Could not parse cgroupsv2 path" otherwise) — so the slice has
/// to exist before the unit starts. After a reboot nothing has created it
/// yet: the first `yutani launch` or adoption would, but the tunnel is
/// normally connected before EVE is launched. Starting a slice that is
/// already active is a no-op, and an active slice stays active (and its
/// cgroup stays put) after every scope under it has exited.
///
/// The worker does the same from root (`rules::slice_start_command`) and
/// is the authority: a failure here is only warned about, so that a
/// caller whose environment cannot reach the user manager still gets the
/// tunnel, with the worker's own message if the slice truly cannot exist.
pub fn slice_start_argv() -> Vec<String> {
    ["systemctl", "--user", "--no-ask-password", "start", SLICE].iter().map(|s| s.to_string()).collect()
}

fn run(argv: &[String], what: &str) -> anyhow::Result<()> {
    let out = Command::new(&argv[0]).args(&argv[1..]).output().context("systemctl")?;
    ensure!(out.status.success(), "{what}: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

fn systemctl(verb: &str) -> anyhow::Result<()> {
    ensure!(installed(), "tunnel is not installed; run `yutani tunnel install <conf>`");
    run(&systemctl_argv(verb), &format!("systemctl {verb} {UNIT_NAME}"))
}

pub fn connect() -> anyhow::Result<()> {
    ensure!(installed(), "tunnel is not installed; run `yutani tunnel install <conf>`");
    if let Err(e) = run(&slice_start_argv(), &format!("systemctl --user start {SLICE}")) {
        tracing::warn!("{e:#}; leaving it to the tunnel worker to start the slice");
    }
    systemctl("start")
}

pub fn disconnect() -> anyhow::Result<()> {
    systemctl("stop")
}

/// What `quit` has to do before the daemon may exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitPlan {
    /// The tunnel is up: stop the unit first, then exit. Leaving it running
    /// would strand EVE's traffic in a tunnel with nothing left to manage
    /// it — and the next launch would find a link it never brought up.
    DisconnectThenExit,
    /// Nothing to wind down.
    ExitNow,
}

/// Pure decision for IPC `quit`. Only a tunnel that is both installed and
/// connected is worth a `systemctl stop`: an uninstalled unit has nothing
/// to stop, and a disconnected one is already where we want it — and each
/// avoided call is ~10 s the daemon does not linger for.
pub fn quit_plan(installed: bool, connected: bool) -> QuitPlan {
    if installed && connected { QuitPlan::DisconnectThenExit } else { QuitPlan::ExitNow }
}

pub fn read_tunnel_file() -> Option<TunnelFile> {
    serde_json::from_slice(&std::fs::read(STATUS_PATH).ok()?).ok()
}

pub fn iface_present() -> bool {
    std::path::Path::new(&format!("/sys/class/net/{IFACE}")).exists()
}

pub fn sysfs_counters() -> Option<(u64, u64)> {
    let read = |n: &str| std::fs::read_to_string(format!("/sys/class/net/{IFACE}/statistics/{n}")).ok()?.trim().parse::<u64>().ok();
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

/// How long `systemctl is-failed` may take before its answer is discarded.
/// `current_tunnel_status` runs on every IPC `status` request — once a
/// second while the applet's popup is open — so a systemd or dbus that
/// accepts the call and then stalls must not stall the daemon with it.
pub const UNIT_FAILED_TIMEOUT: Duration = Duration::from_secs(1);

/// Read `systemctl is-failed`'s verdict off its exit status.
///
/// `is-failed` exits **0 only for a unit in the failed state**; 3 for an
/// inactive one, 4 for a unit that does not exist, non-zero for everything
/// else. So nothing but a clean exit counts — and `None` (the call timed
/// out and was killed) is not evidence of anything, least of all failure.
fn failed_from(out: Option<&Output>) -> bool {
    out.is_some_and(|o| o.status.success())
}

/// Whether systemd calls the unit failed. Read-only and unprivileged: no
/// polkit prompt, no state change.
pub fn unit_failed() -> bool {
    let argv = systemctl_argv("is-failed");
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    let out = crate::proc::output_with_timeout(&mut cmd, UNIT_FAILED_TIMEOUT).ok().flatten();
    failed_from(out.as_ref())
}

pub fn current_tunnel_status(location: &str) -> TunnelStatus {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let installed = installed();
    let iface = iface_present();
    // Spec §3: a unit that is installed and simply stopped is the ordinary
    // disconnected state — only systemd calling it *failed* is the attention
    // state. Short-circuited so the common paths (nothing installed, or the
    // link happily up) never spawn `systemctl` at all.
    let failed = installed && !iface && unit_failed();
    assemble(read_tunnel_file().as_ref(), iface, sysfs_counters(), installed, failed, location, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// nft resolves the `socket cgroupv2` path against the live cgroup
    /// tree when the ruleset is loaded, so the slice's cgroup must exist
    /// *before* the unit starts. After a reboot nothing has created it yet
    /// (the first `yutani launch` or adoption would), so connect creates
    /// it itself, on the user manager, without an authentication agent.
    #[test]
    fn connect_starts_the_eve_slice_on_the_user_manager_first() {
        assert_eq!(slice_start_argv(), vec!["systemctl", "--user", "--no-ask-password", "start", "yutani-eve.slice"]);
    }

    #[test]
    fn systemctl_never_waits_on_an_authentication_agent() {
        assert_eq!(systemctl_argv("start"), vec!["systemctl", "--no-ask-password", "start", "yutani-tunnel.service"]);
        assert_eq!(systemctl_argv("stop"), vec!["systemctl", "--no-ask-password", "stop", "yutani-tunnel.service"]);
        assert_eq!(systemctl_argv("is-failed"), vec!["systemctl", "--no-ask-password", "is-failed", "yutani-tunnel.service"]);
    }

    /// Only a clean exit 0 is the failed state, and a timed-out `systemctl`
    /// (`None`) says nothing at all — it must not be read as "failed", which
    /// would park the attention badge in the panel whenever systemd is slow.
    #[test]
    fn only_a_zero_exit_from_is_failed_counts_as_failed() {
        let run = |program: &str| {
            crate::proc::output_with_timeout(&mut Command::new(program), UNIT_FAILED_TIMEOUT).unwrap()
        };
        let zero = run("true");
        let nonzero = run("false");
        assert!(failed_from(zero.as_ref()), "exit 0 is the failed state");
        assert!(!failed_from(nonzero.as_ref()), "any non-zero exit is not");
        assert!(!failed_from(None), "a timeout is not evidence of failure");
    }

    #[test]
    fn quit_stops_the_tunnel_only_when_there_is_a_live_one_to_stop() {
        assert_eq!(quit_plan(true, true), QuitPlan::DisconnectThenExit);
        assert_eq!(quit_plan(true, false), QuitPlan::ExitNow);
        assert_eq!(quit_plan(false, false), QuitPlan::ExitNow);
        // Nonsense in practice, but "not installed" wins: there is no unit
        // to hand `systemctl stop`, so waiting on one would just fail slowly.
        assert_eq!(quit_plan(false, true), QuitPlan::ExitNow);
    }

    /// Read-only and unprivileged, so the real command may run here. With
    /// no unit installed `systemctl is-failed` prints `inactive` and exits
    /// 4, which is emphatically not the failed state.
    #[test]
    fn a_unit_that_is_not_installed_is_not_failed() {
        if installed() {
            return; // a live install could legitimately be in any state
        }
        assert!(!unit_failed());
    }
}
