//! Tunnel status: what the root worker publishes (`TunnelFile`), what the
//! daemon answers to the IPC `status` request (`Status`), and the pure
//! assembly between them.

use serde::{Deserialize, Serialize};

/// `/run/yutani/tunnel.json`, written by `yutani tunnel run` once a second.
/// Never contains key material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelFile {
    pub up: bool,
    pub iface: String,
    pub address: String,
    pub endpoint: String,
    /// 0 = no handshake yet.
    pub latest_handshake_unix: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub since_unix: u64,
    /// The public address the exit node answers on, as seen from inside the
    /// tunnel — the address EVE's traffic actually arrives from, which is
    /// what spec §4.2's "tunnel IP" means. `None` until the first successful
    /// lookup. `serde(default)`: a file written by an older worker (or one
    /// that has not looked yet) must still parse.
    #[serde(default)]
    pub exit_address: Option<String>,
}

/// Peer line of `wg show <iface> dump`: endpoint, latest handshake (unix
/// seconds, 0 if none), rx bytes, tx bytes. The first line (interface,
/// with the private key) is skipped and never stored.
///
/// Reads the first peer line only (line index 1); any further peer lines
/// (multi-peer configs) are ignored — this is a single-peer design.
pub fn parse_wg_dump(dump: &str) -> Option<(String, u64, u64, u64)> {
    let peer = dump.lines().nth(1)?;
    let f: Vec<&str> = peer.split('\t').collect();
    if f.len() < 7 {
        return None;
    }
    let endpoint = if f[2] == "(none)" {
        String::new()
    } else {
        f[2].to_string()
    };
    Some((
        endpoint,
        f[4].parse().ok()?,
        f[5].parse().ok()?,
        f[6].parse().ok()?,
    ))
}

/// `Default` is "nothing installed, nothing connected": the state the
/// settings window shows before its first refresh, and the base the page's
/// summary test builds on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelStatus {
    pub installed: bool,
    pub connected: bool,
    pub iface: String,
    pub location: String,
    pub address: Option<String>,
    pub endpoint: Option<String>,
    pub handshake_age_s: Option<u64>,
    /// Seconds since the link came up (`TunnelFile::since_unix`), `None`
    /// while it is down. It is what bounds the sync icon: a tunnel that is
    /// up and has never handshaked is *settling* only for as long as it has
    /// just come up (`applet::icon`).
    ///
    /// `serde(default)`: the applet is a separate binary and can be older or
    /// newer than the daemon it polls, so a reply that predates this field
    /// must still parse rather than fail the whole poll.
    #[serde(default)]
    pub up_for_s: Option<u64>,
    /// systemd reports the unit as `failed`: it is installed, the interface
    /// is absent, and `systemctl is-failed` says so. Drives the attention
    /// icon (spec §3). `serde(default)` for the same reason as `up_for_s`.
    #[serde(default)]
    pub failed: bool,
    /// The tunnel's *public* exit address (`TunnelFile::exit_address`),
    /// `None` while disconnected or before the worker's first successful
    /// lookup. The popup prefers it over the internal `address`: spec §4.2's
    /// "tunnel IP" is the address the world sees, not 10.2.0.2.
    /// `serde(default)` for the same reason as `up_for_s`.
    #[serde(default)]
    pub exit_address: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStatus {
    pub name: String,
    pub active: bool,
}

/// Reply body of the IPC `status` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// In layout order (the index + 1 is what `focus <n>` takes).
    pub clients: Vec<ClientStatus>,
    pub hidden: bool,
    pub tunnel: TunnelStatus,
}

/// Combine the worker's file, whether `/sys/class/net/yutani0` exists,
/// sysfs byte counters (preferred when present: they need no root and stay
/// fresh even if the file is stale), the unit-file presence, and whether
/// systemd calls the unit failed.
#[allow(clippy::too_many_arguments)]
pub fn assemble(
    file: Option<&TunnelFile>,
    iface_present: bool,
    sysfs: Option<(u64, u64)>,
    installed: bool,
    failed: bool,
    location: &str,
    now_unix: u64,
) -> TunnelStatus {
    let connected = iface_present && file.is_some_and(|f| f.up);
    let (rx, tx) = sysfs.unwrap_or_else(|| file.map_or((0, 0), |f| (f.rx_bytes, f.tx_bytes)));
    let handshake_age_s = match file {
        Some(f) if connected && f.latest_handshake_unix > 0 => {
            Some(now_unix.saturating_sub(f.latest_handshake_unix))
        }
        _ => None,
    };
    // `saturating_sub`: the worker's clock and ours are the same clock, but
    // a step backwards (NTP, suspend) must read as "just up", not as a
    // 584-million-year-old tunnel.
    let up_for_s = match file {
        Some(f) if connected => Some(now_unix.saturating_sub(f.since_unix)),
        _ => None,
    };
    TunnelStatus {
        installed,
        connected,
        iface: super::IFACE.to_string(),
        location: location.to_string(),
        address: file.map(|f| f.address.clone()),
        endpoint: file.and_then(|f| {
            if f.endpoint.is_empty() {
                None
            } else {
                Some(f.endpoint.clone())
            }
        }),
        handshake_age_s,
        up_for_s,
        failed,
        // Nothing is connected, so there is no exit to report — a stale
        // address from the last session would read as a live one.
        exit_address: if connected { file.and_then(|f| f.exit_address.clone()) } else { None },
        rx_bytes: rx,
        tx_bytes: tx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "cHJpdg==\tcHVi\t51820\toff\ncGVlcnB1YmxpY2tleXBlZXJwdWJsaWNrZXlwZWVycHU=\t(none)\t198.51.100.10:51820\t0.0.0.0/0,::/0\t1789180000\t413100000\t2790000000\t25\n";

    #[test]
    fn parses_the_peer_line_of_wg_dump() {
        assert_eq!(
            parse_wg_dump(DUMP),
            Some((
                "198.51.100.10:51820".into(),
                1789180000,
                413100000,
                2790000000
            ))
        );
        assert_eq!(parse_wg_dump("cHJpdg==\tcHVi\t51820\toff\n"), None);
        assert_eq!(parse_wg_dump(""), None);
    }

    const DUMP_NO_ENDPOINT: &str = "cHJpdg==\tcHVi\t51820\toff\ncGVlcnB1YmxpY2tleXBlZXJwdWJsaWNrZXlwZWVycHU=\t(none)\t(none)\t0.0.0.0/0,::/0\t0\t0\t0\t25\n";

    #[test]
    fn parse_wg_dump_maps_none_endpoint_to_empty_string() {
        assert_eq!(parse_wg_dump(DUMP_NO_ENDPOINT), Some(("".into(), 0, 0, 0)));
    }

    #[test]
    fn assemble_gives_none_endpoint_when_file_endpoint_is_empty() {
        let mut f = file();
        f.endpoint = "".into();
        let s = assemble(Some(&f), true, None, true, false, "London", 1021);
        assert_eq!(s.endpoint, None);
    }

    fn file() -> TunnelFile {
        TunnelFile {
            up: true,
            iface: "yutani0".into(),
            address: "10.2.0.2".into(),
            endpoint: "198.51.100.10:51820".into(),
            latest_handshake_unix: 1000,
            rx_bytes: 5,
            tx_bytes: 7,
            since_unix: 900,
            exit_address: Some("198.51.100.10".into()),
        }
    }

    #[test]
    fn assemble_connected_prefers_sysfs_counters() {
        let s = assemble(Some(&file()), true, Some((50, 70)), true, false, "London", 1021);
        assert!(s.installed && s.connected);
        assert_eq!(s.handshake_age_s, Some(21));
        assert_eq!((s.rx_bytes, s.tx_bytes), (50, 70));
        assert_eq!(s.address.as_deref(), Some("10.2.0.2"));
        assert_eq!(s.endpoint.as_deref(), Some("198.51.100.10:51820"));
        assert_eq!(s.location, "London");
        assert_eq!(s.iface, "yutani0");
    }

    #[test]
    fn assemble_without_iface_is_disconnected_even_if_file_says_up() {
        let s = assemble(Some(&file()), false, None, true, false, "London", 1021);
        assert!(!s.connected);
        assert_eq!(s.handshake_age_s, None);
        assert_eq!((s.rx_bytes, s.tx_bytes), (5, 7)); // last known totals stay
    }

    #[test]
    fn assemble_no_handshake_yet_and_not_installed() {
        let mut f = file();
        f.latest_handshake_unix = 0;
        let s = assemble(Some(&f), true, None, true, false, "London", 1021);
        assert!(s.connected);
        assert_eq!(s.handshake_age_s, None);
        let s = assemble(None, false, None, false, false, "London", 1);
        assert!(!s.installed && !s.connected && s.address.is_none());
    }

    #[test]
    fn assemble_reports_how_long_the_link_has_been_up() {
        // `since_unix` 900, now 1021 → 121 s up.
        let s = assemble(Some(&file()), true, None, true, false, "London", 1021);
        assert_eq!(s.up_for_s, Some(121));
        // A clock that went backwards must not underflow into a huge age.
        let s = assemble(Some(&file()), true, None, true, false, "London", 0);
        assert_eq!(s.up_for_s, Some(0));
        // Nothing is up, so there is no uptime to report.
        let s = assemble(Some(&file()), false, None, true, false, "London", 1021);
        assert_eq!(s.up_for_s, None);
        let s = assemble(None, false, None, false, false, "London", 1021);
        assert_eq!(s.up_for_s, None);
    }

    /// Spec §4.2's "tunnel IP" is the *public* exit address, so the worker's
    /// value has to reach the applet — and must not survive the link it
    /// belongs to.
    #[test]
    fn assemble_passes_the_exit_address_through_only_while_connected() {
        let s = assemble(Some(&file()), true, None, true, false, "London", 1021);
        assert_eq!(s.exit_address.as_deref(), Some("198.51.100.10"));
        // The link is gone: last session's exit address would read as live.
        let s = assemble(Some(&file()), false, None, true, false, "London", 1021);
        assert_eq!(s.exit_address, None);
        // Up, but the worker has not managed a lookup yet.
        let mut f = file();
        f.exit_address = None;
        let s = assemble(Some(&f), true, None, true, false, "London", 1021);
        assert_eq!(s.exit_address, None);
        assert_eq!(assemble(None, false, None, false, false, "London", 1).exit_address, None);
    }

    #[test]
    fn assemble_passes_the_units_failed_state_through() {
        let s = assemble(Some(&file()), true, None, true, false, "London", 1021);
        assert!(!s.failed);
        let s = assemble(None, false, None, true, true, "London", 1021);
        assert!(s.failed);
    }

    /// The same across the worker/daemon boundary: a `tunnel.json` written
    /// before the exit address existed must still load, or a tunnel that
    /// survived the upgrade would read as down.
    #[test]
    fn a_tunnel_file_without_the_exit_address_still_parses() {
        let json = r#"{"up":true,"iface":"yutani0","address":"10.2.0.2","endpoint":"198.51.100.10:51820",
            "latest_handshake_unix":1000,"rx_bytes":5,"tx_bytes":7,"since_unix":900}"#;
        let back: TunnelFile = serde_json::from_str(json).expect("an older worker's file must parse");
        assert_eq!(back.exit_address, None);
    }

    /// The applet and the daemon are separate binaries and can be different
    /// versions: a reply from a daemon that predates `up_for_s`/`failed`
    /// must still parse, not blow up the applet's poll.
    #[test]
    fn a_status_json_without_the_new_fields_still_parses() {
        let json = r#"{"clients":[{"name":"KestrelVance","active":true}],"hidden":false,
            "tunnel":{"installed":true,"connected":true,"iface":"yutani0","location":"London",
            "address":"10.2.0.2","endpoint":"198.51.100.10:51820","handshake_age_s":21,
            "rx_bytes":5,"tx_bytes":7}}"#;
        let back: Status = serde_json::from_str(json).expect("an older daemon's reply must parse");
        assert_eq!(back.tunnel.handshake_age_s, Some(21));
        assert_eq!(back.tunnel.up_for_s, None);
        assert!(!back.tunnel.failed);
        assert_eq!(back.tunnel.exit_address, None);
    }

    #[test]
    fn status_json_round_trips() {
        let st = Status {
            clients: vec![ClientStatus {
                name: "KestrelVance".into(),
                active: true,
            }],
            hidden: false,
            tunnel: assemble(Some(&file()), true, Some((1, 2)), true, false, "London", 1021),
        };
        let json = serde_json::to_string(&st).unwrap();
        let back: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clients[0].name, "KestrelVance");
        assert_eq!(back.tunnel.handshake_age_s, Some(21));
    }
}
