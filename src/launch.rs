//! `yutani launch -- <command…>`: run a command (Steam's `%command%`) in
//! this process's place, with the game's environment set first.
//!
//! The environment is what
//! [`LaunchConfig`](yutani::model::config::LaunchConfig) asks for: present
//! mode and frame-rate cap (see [`game_env`]). A fullscreen client covered
//! by another gets no frame callbacks from the compositor, and a vsynced
//! swapchain blocks on them, so without this the covered client stops
//! drawing and its thumbnail freezes.
//!
//! The game is `exec`'d, so it ends up *being* the `yutani launch` pid:
//! whatever tracks that pid (Steam does, for `%command%`) sees the game's
//! own exit status, and a signal sent to it reaches the game. `yutani
//! launch` must never prevent the game from starting, and never run it
//! twice: `exec` only returns when nothing was started.
//!
//! The user-manager preflight ([`preflight_argv`], [`should_wrap`]) lives
//! here for `launch_steam`, which wraps Steam itself in a scope.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Output};
use std::time::Duration;

use cosmic::cctk::sctk::{
    delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::client::{Connection, QueueHandle, globals::registry_queue_init, protocol::wl_output},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use yutani::model::config::{Config, LaunchConfig};

/// `MESA_VK_WSI_PRESENT_MODE=mailbox`: Mesa's Vulkan WSI presents without
/// waiting for the compositor's frame callback (which never comes for a
/// covered surface). The compositor still vsyncs the screen, so this does
/// not tear; it only stops the client blocking. Not `immediate`: Mesa's
/// Wayland WSI offers that mode only when the compositor has
/// `wp_tearing_control_v1` (COSMIC does not), rejects the override
/// otherwise, and with any override set stops honouring the per-present
/// mode switch vkd3d-proton uses, so the client ends up pinned to FIFO.
pub const PRESENT_MODE_VAR: &str = "MESA_VK_WSI_PRESENT_MODE";
/// The value: the one unthrottled mode every Wayland compositor supports.
pub const PRESENT_MODE: &str = "mailbox";
/// DXVK's own frame limiter, so "no vsync" does not mean "as fast as the
/// GPU can go" for every client at once.
pub const FRAME_RATE_VAR: &str = "DXVK_FRAME_RATE";
/// The cap when neither the config nor the compositor names one.
pub const FALLBACK_FRAME_RATE: u32 = 60;

/// The variables to add to the game's environment, given the config, the
/// fastest connected display's refresh rate (if it could be read) and
/// whether the launch line already sets a variable (then it wins: the user
/// wrote it on purpose). Pure, so the rules are testable without a
/// compositor or a game.
pub fn game_env(config: &LaunchConfig, display_hz: Option<u32>, already_set: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    if config.unlocked_present && !already_set(PRESENT_MODE_VAR) {
        vars.push((PRESENT_MODE_VAR.to_string(), PRESENT_MODE.to_string()));
    }
    // A cap only matters once vsync no longer paces the client, unless the
    // user asked for one outright.
    if (config.unlocked_present || config.frame_rate.is_some()) && !already_set(FRAME_RATE_VAR) {
        let cap = config.frame_rate.or(display_hz).unwrap_or(FALLBACK_FRAME_RATE);
        vars.push((FRAME_RATE_VAR.to_string(), cap.to_string()));
    }
    vars
}

/// The highest current refresh rate among the given outputs' modes, in
/// whole Hz (`59_997` mHz → 60), or `None` if nothing reports one.
pub fn fastest_refresh_hz(current_modes_mhz: impl IntoIterator<Item = i32>) -> Option<u32> {
    current_modes_mhz
        .into_iter()
        .filter(|&mhz| mhz > 0)
        .map(|mhz| ((mhz as u32) + 500) / 1000)
        .filter(|&hz| hz > 0)
        .max()
}

struct DisplayProbe {
    registry: RegistryState,
    outputs: OutputState,
}

impl OutputHandler for DisplayProbe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for DisplayProbe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers!(OutputState);
}

delegate_output!(DisplayProbe);
delegate_registry!(DisplayProbe);

/// Ask the compositor for every output's current mode: two round trips on
/// a throw-away connection, milliseconds on a live compositor. `None` when
/// there is no Wayland display to ask (or it answers nothing useful); the
/// game must start either way.
fn display_refresh_hz() -> Option<u32> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<DisplayProbe>(&conn).ok()?;
    let qh = queue.handle();
    let mut probe = DisplayProbe { registry: RegistryState::new(&globals), outputs: OutputState::new(&globals, &qh) };
    // One round trip binds the outputs, the next collects their modes.
    queue.roundtrip(&mut probe).ok()?;
    queue.roundtrip(&mut probe).ok()?;
    let modes: Vec<i32> = probe
        .outputs
        .outputs()
        .filter_map(|o| probe.outputs.info(&o))
        .flat_map(|info| info.modes.into_iter().filter(|m| m.current).map(|m| m.refresh_rate))
        .collect();
    fastest_refresh_hz(modes)
}

/// Put [`game_env`]'s variables into this process's environment, which both
/// exec paths hand to the game, and say so once on stderr (Steam keeps it).
fn prepare_game_env(config: &LaunchConfig) {
    let display_hz = if config.unlocked_present && config.frame_rate.is_none() { display_refresh_hz() } else { None };
    let vars = game_env(config, display_hz, |name| std::env::var_os(name).is_some());
    if vars.is_empty() {
        return;
    }
    let shown: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
    eprintln!("yutani launch: {}", shown.join(" "));
    for (k, v) in vars {
        // Single-threaded here, before anything is spawned.
        unsafe { std::env::set_var(k, v) };
    }
}

/// Wall-clock bound on the preflight. Something is waiting behind it, so
/// it is deliberately short: a manager that cannot answer in two seconds
/// is treated as unreachable.
pub const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(2);

/// argv for the preflight: is the user's systemd/D-Bus manager reachable at
/// all? `--timeout=2` only bounds a method call's reply (and `status`
/// ignores it); [`PREFLIGHT_TIMEOUT`] is what bounds this call.
pub fn preflight_argv() -> Vec<String> {
    ["busctl", "--user", "--timeout=2", "status"].iter().map(|s| s.to_string()).collect()
}

/// From the preflight's outcome (`None` if it timed out or could not even
/// be spawned — e.g. `busctl` itself is missing), decide whether it is safe
/// to wrap a command in `systemd-run`. A timeout, a non-zero exit, a spawn
/// error, or empty stdout (the call returned but said nothing sane) all
/// mean "no, run directly".
pub fn should_wrap(preflight: Option<&Output>) -> bool {
    preflight.is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// Replace this process with `argv`. A successful `execvp` never comes
/// back, so this only ever returns — with the reason — when nothing was
/// started, which is exactly when the caller may still try something else.
fn exec(argv: &[String]) -> io::Error {
    Command::new(&argv[0]).args(&argv[1..]).exec()
}

/// Run `command` in this process's place. Returns only if the exec failed,
/// as the shell's "command not found" code (127) after saying why on
/// stderr.
fn run_directly(command: &[String]) -> u8 {
    run_directly_with(command, exec)
}

/// [`run_directly`] with the exec step handed in. The tests use a stub:
/// a real `CommandExt::exec` that fails has already run std's pre-exec
/// reset (SIGPIPE back to `SIG_DFL`, the signal mask cleared), which is
/// process-global and would stay with the test binary for every test
/// after it.
fn run_directly_with(command: &[String], exec: impl FnOnce(&[String]) -> io::Error) -> u8 {
    let err = exec(command);
    eprintln!("yutani launch: cannot run {}: {err}", command[0]);
    127
}

/// Set the game's environment, then become the game. Returns only when the
/// command could not be started at all.
pub fn run(command: Vec<String>) -> ExitCode {
    if command.is_empty() {
        eprintln!("yutani launch: nothing to run (usage: yutani launch -- <command…>)");
        return ExitCode::from(2);
    }
    prepare_game_env(&Config::load().launch);
    ExitCode::from(run_directly(&command))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    fn output(code: i32, stdout: &[u8]) -> Output {
        Output { status: ExitStatus::from_raw(code << 8), stdout: stdout.to_vec(), stderr: Vec::new() }
    }

    /// Daniel, 2026-09-16: two fullscreen clients on one display, the one
    /// underneath drew nothing (0 GPU time) until raised, so its thumbnail
    /// was a still image. A present mode that never waits for the frame
    /// callback keeps it drawing; the cap keeps that from meaning "flat
    /// out"; a variable on the launch line wins.
    ///
    /// Same day, later: `immediate` was the wrong mode. Mesa only offers it
    /// when the compositor has `wp_tearing_control_v1`, which cosmic-comp
    /// lacks, so Mesa rejected the override ("Unsupported
    /// MESA_VK_WSI_PRESENT_MODE value!" in Steam's console log) and, with
    /// an override set, also stopped honouring vkd3d-proton's per-present
    /// mode switch: the swapchain sat in FIFO and the covered client hung in
    /// `vkQueuePresentKHR` waiting for a frame callback. `mailbox` is always
    /// offered on Wayland, never tears and never waits on the callback.
    #[test]
    fn the_game_gets_mailbox_present_and_a_cap_from_the_display() {
        let cfg = LaunchConfig::default();
        assert_eq!(
            game_env(&cfg, Some(170), |_| false),
            vec![("MESA_VK_WSI_PRESENT_MODE".to_string(), "mailbox".to_string()), ("DXVK_FRAME_RATE".to_string(), "170".to_string())]
        );
        // No display to ask: a sane fixed cap rather than none.
        assert_eq!(game_env(&cfg, None, |_| false)[1].1, "60");
        // A configured cap beats the display.
        let cfg = LaunchConfig { unlocked_present: true, frame_rate: Some(90) };
        assert_eq!(game_env(&cfg, Some(170), |_| false)[1].1, "90");
    }

    #[test]
    fn the_launch_line_wins_and_vsync_users_get_nothing_they_did_not_ask_for() {
        let cfg = LaunchConfig::default();
        assert_eq!(game_env(&cfg, Some(60), |name| name == "MESA_VK_WSI_PRESENT_MODE"), vec![("DXVK_FRAME_RATE".to_string(), "60".to_string())]);
        assert_eq!(game_env(&cfg, Some(60), |_| true), vec![]);
        // Unlocked present off: leave the game alone entirely…
        let cfg = LaunchConfig { unlocked_present: false, frame_rate: None };
        assert_eq!(game_env(&cfg, Some(60), |_| false), vec![]);
        // …unless a cap was asked for outright.
        let cfg = LaunchConfig { unlocked_present: false, frame_rate: Some(72) };
        assert_eq!(game_env(&cfg, Some(60), |_| false), vec![("DXVK_FRAME_RATE".to_string(), "72".to_string())]);
    }

    #[test]
    fn the_fastest_current_mode_wins_rounded_to_whole_hz() {
        assert_eq!(fastest_refresh_hz([59_997, 170_001]), Some(170));
        assert_eq!(fastest_refresh_hz([60_000]), Some(60));
        assert_eq!(fastest_refresh_hz([0, -1]), None);
        assert_eq!(fastest_refresh_hz([]), None);
    }

    #[test]
    fn preflight_checks_the_user_bus_with_a_short_timeout() {
        assert_eq!(preflight_argv(), vec!["busctl", "--user", "--timeout=2", "status"]);
        // The real bound is the process-level one: `busctl status` ignores
        // `--timeout` entirely.
        assert!(PREFLIGHT_TIMEOUT <= Duration::from_secs(2), "a launch waits behind this");
    }

    #[test]
    fn a_preflight_that_times_out_is_treated_as_unreachable() {
        // `output_with_timeout` reports a timeout as `Ok(None)`, flattened
        // here to `None` — the same "do not wrap" answer as a missing
        // `busctl`.
        let timed_out: Option<Output> = yutani::proc::output_with_timeout(
            Command::new("sleep").arg("10"),
            Duration::from_millis(100),
        )
        .ok()
        .flatten();
        assert!(!should_wrap(timed_out.as_ref()), "a wedged manager must not wrap");
    }

    #[test]
    fn should_wrap_only_when_preflight_succeeded_with_output() {
        assert!(!should_wrap(None), "spawn error must not wrap");
        assert!(!should_wrap(Some(&output(1, b"nope"))), "non-zero exit must not wrap");
        assert!(!should_wrap(Some(&output(0, b""))), "no output must not wrap");
        assert!(should_wrap(Some(&output(0, b"BusAddress=unix:path=/run/user/1000/bus\n"))));
    }

    /// The direct path hands the whole command, untouched, to one exec and
    /// answers a failure with the shell's 127. Checked with a stub exec:
    /// `exec` itself never returns on success (the test would be replaced
    /// by the target) and a *failed* real exec leaves std's pre-exec reset
    /// behind for the rest of the test process.
    #[test]
    fn running_a_missing_command_directly_reports_127() {
        let command = vec!["/nonexistent-yutani-binary".to_string(), "--flag".to_string()];
        let mut seen: Option<Vec<String>> = None;
        let code = run_directly_with(&command, |argv| {
            seen = Some(argv.to_vec());
            io::Error::new(io::ErrorKind::NotFound, "No such file or directory")
        });
        assert_eq!(code, 127);
        assert_eq!(seen.as_deref(), Some(&command[..]), "the command must be exec'd exactly as given");
    }
}
