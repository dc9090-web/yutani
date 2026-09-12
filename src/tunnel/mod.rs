//! EVE-only WireGuard tunnel: root worker, install, control, status.
//! See docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md.

pub mod conf;
pub mod control;
pub mod install;
pub mod rules;
pub mod status;
pub mod worker;

pub const IFACE: &str = "yutani0";
pub const FWMARK: u32 = 0x59;
pub const TABLE: u32 = 51820;
pub const SLICE: &str = "yutani-eve.slice";
pub const CONF_PATH: &str = "/etc/yutani/tunnel.conf";
pub const STATUS_PATH: &str = "/run/yutani/tunnel.json";
pub const UNIT_NAME: &str = "yutani-tunnel.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/yutani-tunnel.service";
pub const POLKIT_PATH: &str = "/etc/polkit-1/rules.d/50-yutani-tunnel.rules";

/// Create `path` with `mode` from the start: files that hold key material
/// must never be readable by anyone else, not even for the instant between
/// `write` and `chmod`.
fn write_with_mode(path: &str, text: &str, mode: u32) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("remove {path}")),
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .with_context(|| format!("create {path}"))?;
    f.write_all(text.as_bytes()).with_context(|| format!("write {path}"))
}
