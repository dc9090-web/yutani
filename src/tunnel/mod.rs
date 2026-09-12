//! EVE-only WireGuard tunnel: root worker, install, control, status.
//! See docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md.

pub mod conf;
pub mod rules;
pub mod status;

pub const IFACE: &str = "yutani0";
pub const FWMARK: u32 = 0x59;
pub const TABLE: u32 = 51820;
pub const SLICE: &str = "yutani-eve.slice";
pub const CONF_PATH: &str = "/etc/yutani/tunnel.conf";
pub const STATUS_PATH: &str = "/run/yutani/tunnel.json";
pub const UNIT_NAME: &str = "yutani-tunnel.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/yutani-tunnel.service";
pub const POLKIT_PATH: &str = "/etc/polkit-1/rules.d/50-yutani-tunnel.rules";
