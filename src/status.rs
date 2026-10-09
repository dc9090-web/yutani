//! What the daemon answers to the IPC `status` request (`Status`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStatus {
    pub name: String,
    pub active: bool,
}

/// The hotkeys the daemon's shortcuts are bound to, as the popup prints
/// them (redesign spec §2): `Ctrl+Alt`, `→`, `←`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ShortcutHint {
    pub prefix: String,
    pub next: String,
    pub prev: String,
}

/// Reply body of the IPC `status` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// In layout order (the index + 1 is what `focus <n>` takes).
    pub clients: Vec<ClientStatus>,
    pub hidden: bool,
    /// `serde(default)`: `None` from a daemon older than this field.
    #[serde(default)]
    pub shortcuts: Option<ShortcutHint>,
    /// Steam accounts whose EVE launch line is not `steam::Verdict::Ok`:
    /// a `yutani` path that no longer exists, or no `yutani launch` at
    /// all. Empty when every account is fine or Steam was not found, and
    /// (`serde(default)`) from a daemon older than this field.
    #[serde(default)]
    pub steam: Vec<crate::steam::Finding>,
    /// A Launch EVE in progress or just ended (`launch_eve`); `None`
    /// otherwise and (`serde(default)`) from a daemon older than Phase 3.
    #[serde(default)]
    pub launch: Option<crate::launch_eve::LaunchState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The applet and the daemon are separate binaries and can be different
    /// versions: a reply from a daemon that still had the tunnel (removed
    /// 2026-10-09) parses, its tunnel and ping fields ignored.
    #[test]
    fn a_status_json_from_a_daemon_with_the_tunnel_still_parses() {
        let json = r#"{"clients":[{"name":"KestrelVance","active":true}],"hidden":false,
            "tunnel":{"installed":true,"connected":true,"iface":"yutani0","location":"London",
            "address":"10.2.0.2","endpoint":"198.51.100.10:51820","handshake_age_s":21,
            "rx_bytes":5,"tx_bytes":7},"direct_ping":{"rtt_us":16000,"seq":3}}"#;
        let back: Status = serde_json::from_str(json).expect("an older daemon's reply must parse");
        assert_eq!(back.clients[0].name, "KestrelVance");
        assert!(back.steam.is_empty(), "an older daemon reports no Steam findings");
    }

    #[test]
    fn status_json_round_trips() {
        let st = Status {
            clients: vec![ClientStatus { name: "KestrelVance".into(), active: true }],
            hidden: false,
            shortcuts: None,
            steam: vec![crate::steam::Finding {
                verdict: crate::steam::Verdict::Broken { path: "/usr/local/bin/yutani".into() },
                file: "/h/localconfig.vdf".into(),
            }],
            launch: None,
        };
        let json = serde_json::to_string(&st).unwrap();
        let back: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(back, st);
    }
}
