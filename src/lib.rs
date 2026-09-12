//! The Yutani library: everything shared by the `yutani` daemon/CLI binary
//! and the `yutani-applet` panel applet. Capture, the layer-shell UI and the
//! CLI plumbing stay private to `src/main.rs`; only the IPC protocol, the
//! status types, the user config, the icon assets, the bounded subprocess
//! helper and the applet's pure model live here.

pub mod applet;
pub mod assets;
pub mod ipc;
pub mod model;
/// Bounded subprocess execution. In the lib because `tunnel::control` — a
/// lib module — has to run `systemctl is-failed` under a timeout.
pub mod proc;
pub mod tunnel;
