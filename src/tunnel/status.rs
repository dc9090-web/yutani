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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelStatus {
    pub installed: bool,
    pub connected: bool,
    pub iface: String,
    pub location: String,
    pub address: Option<String>,
    pub endpoint: Option<String>,
    pub handshake_age_s: Option<u64>,
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
/// fresh even if the file is stale), and the unit-file presence.
pub fn assemble(
    file: Option<&TunnelFile>,
    iface_present: bool,
    sysfs: Option<(u64, u64)>,
    installed: bool,
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
        let s = assemble(Some(&f), true, None, true, "London", 1021);
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
        }
    }

    #[test]
    fn assemble_connected_prefers_sysfs_counters() {
        let s = assemble(Some(&file()), true, Some((50, 70)), true, "London", 1021);
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
        let s = assemble(Some(&file()), false, None, true, "London", 1021);
        assert!(!s.connected);
        assert_eq!(s.handshake_age_s, None);
        assert_eq!((s.rx_bytes, s.tx_bytes), (5, 7)); // last known totals stay
    }

    #[test]
    fn assemble_no_handshake_yet_and_not_installed() {
        let mut f = file();
        f.latest_handshake_unix = 0;
        let s = assemble(Some(&f), true, None, true, "London", 1021);
        assert!(s.connected);
        assert_eq!(s.handshake_age_s, None);
        let s = assemble(None, false, None, false, "London", 1);
        assert!(!s.installed && !s.connected && s.address.is_none());
    }

    #[test]
    fn status_json_round_trips() {
        let st = Status {
            clients: vec![ClientStatus {
                name: "KestrelVance".into(),
                active: true,
            }],
            hidden: false,
            tunnel: assemble(Some(&file()), true, Some((1, 2)), true, "London", 1021),
        };
        let json = serde_json::to_string(&st).unwrap();
        let back: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clients[0].name, "KestrelVance");
        assert_eq!(back.tunnel.handshake_age_s, Some(21));
    }
}
