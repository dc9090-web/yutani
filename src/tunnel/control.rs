//! Unprivileged side: start/stop the unit (polkit-authorised) and read the
//! tunnel's state without root.

use anyhow::{Context as _, ensure};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::status::{TunnelFile, TunnelStatus, assemble};
use super::{IFACE, STATUS_PATH, UNIT_NAME, UNIT_PATH};

pub fn installed() -> bool {
    std::path::Path::new(UNIT_PATH).exists()
}

fn systemctl(verb: &str) -> anyhow::Result<()> {
    ensure!(installed(), "tunnel is not installed; run `yutani tunnel install <conf>`");
    let out = Command::new("systemctl").args([verb, UNIT_NAME]).output().context("systemctl")?;
    ensure!(out.status.success(), "systemctl {verb} {UNIT_NAME}: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

pub fn connect() -> anyhow::Result<()> {
    systemctl("start")
}

pub fn disconnect() -> anyhow::Result<()> {
    systemctl("stop")
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

pub fn current_tunnel_status(location: &str) -> TunnelStatus {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    assemble(read_tunnel_file().as_ref(), iface_present(), sysfs_counters(), installed(), location, now)
}
