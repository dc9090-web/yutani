//! Start EVE through Steam for Launch EVE: `steam steam://rungameid/8500`,
//! detached (null stdio, own process group) and reaped on a thread, so a
//! Steam that exits is never a zombie under the daemon and a signal to the
//! daemon's group never reaches it.
//!
//! The daemon runs as `yutani.service` with `KillMode=control-group`. When
//! Steam is not running yet, the `steam` we spawn *becomes* the Steam
//! client, so it must not stay in the service's cgroup: restarting the
//! daemon would kill Steam, and with it every EVE client it started. So
//! the spawn goes through `systemd-run --user
//! --scope --slice=app.slice`, which puts it in its own scope like any app
//! the desktop starts. `systemd-run --scope` stays in the foreground for
//! the scope's lifetime; the reaper thread waits for it.
//!
//! Like `yutani launch` (`crate::launch`), the scope is only used when the
//! user's systemd/D-Bus manager answers a `busctl status` preflight and
//! the program is on `PATH` (otherwise `systemd-run` would fail after we
//! returned, with nobody to notice); if `systemd-run` itself cannot be
//! spawned, the program runs directly. A `steam` that cannot be found at
//! all (a Flatpak Steam) falls back to `xdg-open steam://rungameid/8500`.

use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

pub const URL: &str = "steam://rungameid/8500";

/// The slice a desktop-started app lives in: Steam's scope goes there.
pub const APP_SLICE: &str = "app.slice";

/// `steam steam://rungameid/8500`.
pub fn steam_argv() -> Vec<String> {
    vec!["steam".to_string(), URL.to_string()]
}

/// `xdg-open steam://rungameid/8500`, for a Steam that is not on `PATH`
/// (Flatpak): the URL handler starts it.
pub fn xdg_open_argv() -> Vec<String> {
    vec!["xdg-open".to_string(), URL.to_string()]
}

/// `argv` in its own transient scope under [`APP_SLICE`].
pub fn scoped_argv(argv: &[String]) -> Vec<String> {
    let mut v: Vec<String> = ["systemd-run", "--user", "--scope", &format!("--slice={APP_SLICE}"), "--collect", "--quiet", "--"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend(argv.iter().cloned());
    v
}

/// `argv` as a detached command: null stdio, own process group.
pub fn command(argv: &[String]) -> Command {
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    c
}

/// Spawn `argv` detached and reap it in the background; `Err` only if it
/// cannot be started at all. A reaper thread that cannot be created is not
/// a failure — the program is running — so it is logged and the child
/// dropped (it then lingers as a zombie until the daemon exits). The child
/// reaches the reaper over a channel so it is still ours to drop when the
/// thread could not be made.
pub fn spawn_detached(argv: &[String]) -> io::Result<()> {
    let child = command(argv).spawn()?;
    let program = argv[0].clone();
    let (tx, rx) = std::sync::mpsc::channel::<std::process::Child>();
    let reaper = std::thread::Builder::new().name("reap-steam".into()).spawn(move || {
        if let Ok(mut child) = rx.recv() {
            let _ = child.wait();
        }
    });
    match reaper {
        Ok(_) => {
            let _ = tx.send(child);
        }
        Err(e) => {
            tracing::warn!("launch: {program} started but cannot be reaped ({e}); it stays a zombie when it exits");
            // Dropping a `Child` neither kills nor waits: Steam keeps running.
            drop(child);
        }
    }
    Ok(())
}

/// Is `program` something `Command` would find: a path that exists, or a
/// name in one of `PATH`'s directories?
fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return std::path::Path::new(program).is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
}

/// The same preflight as `yutani launch`: does the user's systemd/D-Bus
/// manager answer within [`crate::launch::PREFLIGHT_TIMEOUT`]? Normally a
/// few milliseconds; it runs once per press of Launch EVE.
fn scope_usable() -> bool {
    let argv = crate::launch::preflight_argv();
    let out = yutani::proc::output_with_timeout(Command::new(&argv[0]).args(&argv[1..]), crate::launch::PREFLIGHT_TIMEOUT);
    let ok = crate::launch::should_wrap(out.as_ref().ok().and_then(|o| o.as_ref()));
    if !ok {
        tracing::warn!("launch: user systemd manager not reachable; Steam runs without its own scope");
    }
    ok
}

/// `argv` in a scope when that is usable, else (or when `systemd-run`
/// cannot be spawned) directly.
fn spawn_one(argv: &[String], scoped: bool) -> io::Result<()> {
    if scoped && on_path(&argv[0]) {
        match spawn_detached(&scoped_argv(argv)) {
            Ok(()) => return Ok(()),
            Err(e) => tracing::warn!("launch: cannot run systemd-run ({e}); running {} directly", argv[0]),
        }
    }
    spawn_detached(argv)
}

/// Ask Steam for EVE: `steam`, else (not found) `xdg-open`, each in its
/// own scope under `app.slice` when possible. `Err` is the reason the
/// first candidate could not be started, when none could.
pub fn spawn_eve() -> io::Result<()> {
    let scoped = scope_usable();
    let mut first: Option<io::Error> = None;
    for argv in [steam_argv(), xdg_open_argv()] {
        match spawn_one(&argv, scoped) {
            Ok(()) => {
                tracing::info!("launch: started {}", argv.join(" "));
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                tracing::warn!("launch: cannot run {}: {e}", argv[0]);
                first.get_or_insert(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(first.unwrap_or_else(|| io::Error::from(io::ErrorKind::NotFound)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(c: &Command) -> Vec<String> {
        std::iter::once(c.get_program()).chain(c.get_args()).map(|s| s.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn steam_runs_in_its_own_scope_under_app_slice() {
        let c = command(&scoped_argv(&steam_argv()));
        assert_eq!(
            args(&c),
            [
                "systemd-run",
                "--user",
                "--scope",
                "--slice=app.slice",
                "--collect",
                "--quiet",
                "--",
                "steam",
                "steam://rungameid/8500"
            ]
        );
    }

    #[test]
    fn the_direct_fallback_is_steam_with_the_eve_url() {
        assert_eq!(args(&command(&steam_argv())), ["steam", URL]);
    }

    #[test]
    fn the_flatpak_fallback_is_xdg_open_with_the_eve_url() {
        assert_eq!(args(&command(&xdg_open_argv())), ["xdg-open", URL]);
        assert_eq!(&scoped_argv(&xdg_open_argv())[7..], ["xdg-open", URL]);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let err = spawn_detached(&["/no/such/steam-yutani-test".to_string()]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(spawn_detached(&["/bin/true".to_string()]).is_ok());
    }

    #[test]
    fn on_path_finds_names_and_paths() {
        assert!(on_path("sh"));
        assert!(on_path("/bin/sh"));
        assert!(!on_path("no-such-program-yutani-test"));
        assert!(!on_path("/no/such/steam-yutani-test"));
    }
}
