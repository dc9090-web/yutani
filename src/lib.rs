//! The Yutani library: everything shared by the `yutani` daemon/CLI binary
//! and the `yutani-applet` panel applet. Capture, the layer-shell UI and the
//! CLI plumbing stay private to `src/main.rs`; only the IPC protocol, the
//! status types, the user config, the icon assets, the bounded subprocess
//! helper and the applet's pure model live here.

pub mod applet;
pub mod assets;
pub mod eve_settings;
pub mod ipc;
pub mod model;
/// Bounded subprocess execution. In the lib because `tunnel::control` — a
/// lib module — has to run `systemctl is-failed` under a timeout.
pub mod proc;
pub mod service;
/// Steam's launch options for EVE: read, judged, never written.
pub mod steam;
pub mod tunnel;

/// What EVE Online's *Launch Options* in Steam must say for Yutani to see
/// the client (Steam → EVE Online → Properties → Launch Options).
///
/// `%command%` is Steam's placeholder for everything it would have run; by
/// putting `yutani launch --` in front of it, Steam launches the game
/// *through* Yutani, which is what lets the daemon adopt the process into
/// `yutani-eve.slice` (so the tunnel's cgroup rule applies to it) and
/// track its window. `PROTON_ENABLE_WAYLAND=1` keeps the client on a
/// native Wayland surface — thumbnails capture that, not an XWayland
/// window — and `WINE_NO_WM_DECORATION=1` stops Wine drawing its own
/// title bar over it.
///
/// It names `yutani` bare, so Steam has to be able to find it on `PATH`;
/// [`STEAM_LAUNCH_ARGS_ABSOLUTE`] is the same line for a Steam that
/// cannot (a Flatpak one, or a login shell whose `PATH` Steam did not
/// inherit).
pub const STEAM_LAUNCH_ARGS: &str =
    "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 yutani launch -- %command%";

/// [`STEAM_LAUNCH_ARGS`] with the binary spelled out at the path the
/// tunnel's install instructions use (`sudo install … /usr/local/bin/yutani`).
pub const STEAM_LAUNCH_ARGS_ABSOLUTE: &str =
    "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%";
