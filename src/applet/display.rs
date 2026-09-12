//! Everything the popup's read-only area shows, as finished strings. The
//! view renders this and decides nothing; the rules live here where they
//! are tested. Spec §4.1–§4.3.

use crate::applet::format;
use crate::applet::icon::HANDSHAKE_STALE_S;
use crate::applet::rate::Rates;
use crate::tunnel::status::Status;

const DASH: &str = "—";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// The daemon answered.
    pub online: bool,
    /// The tunnel is up *and* handshaked within the last
    /// [`HANDSHAKE_STALE_S`] seconds — the handoff's "Connected" state,
    /// which drives the dot, its glow, the colours and the menu label.
    pub connected: bool,
    pub status_text: &'static str,
    pub location: String,
    pub iface: String,
    pub accounts: usize,
    pub accounts_label: &'static str,
    pub address: String,
    pub handshake: String,
    pub up_total: String,
    pub up_rate: String,
    pub down_total: String,
    pub down_rate: String,
    /// Thumbnails are hidden (drives the Show/Hide row).
    pub hidden: bool,
    /// A tunnel conf has been installed (a false disables Connect).
    pub installed: bool,
}

pub fn display(status: Option<&Status>, rates: Rates) -> Display {
    let Some(s) = status else {
        return Display {
            online: false,
            connected: false,
            status_text: "Disconnected",
            location: DASH.to_string(),
            iface: crate::tunnel::IFACE.to_string(),
            accounts: 0,
            accounts_label: format::accounts_label(0),
            address: DASH.to_string(),
            handshake: format::handshake(None),
            up_total: format::bytes(0),
            up_rate: format::rate(0.0),
            down_total: format::bytes(0),
            down_rate: format::rate(0.0),
            hidden: false,
            installed: false,
        };
    };
    let t = &s.tunnel;
    // "Connected" is link-up *and* a fresh handshake (spec §4.1); a link
    // that is up but stale reads Disconnected while the panel icon shows
    // the attention badge.
    let connected = t.connected && t.handshake_age_s.is_some_and(|age| age < HANDSHAKE_STALE_S);
    let live = |r: f64| if t.connected { format::rate(r) } else { format::rate(0.0) };
    Display {
        online: true,
        connected,
        status_text: if connected { "Connected" } else { "Disconnected" },
        location: t.location.clone(),
        iface: t.iface.clone(),
        accounts: s.clients.len(),
        accounts_label: format::accounts_label(s.clients.len()),
        address: match (t.connected, t.address.as_deref()) {
            (true, Some(addr)) => addr.to_string(),
            _ => DASH.to_string(),
        },
        handshake: format::handshake(if t.connected { t.handshake_age_s } else { None }),
        up_total: format::bytes(t.tx_bytes),
        up_rate: live(rates.tx),
        down_total: format::bytes(t.rx_bytes),
        down_rate: live(rates.rx),
        hidden: s.hidden,
        installed: t.installed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::applet::rate::Rates;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    fn status(connected: bool, handshake_age_s: Option<u64>, clients: usize) -> Status {
        Status {
            clients: (0..clients)
                .map(|i| ClientStatus { name: format!("Pilot{i}"), active: i == 0 })
                .collect(),
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: Some("10.2.0.2".into()),
                endpoint: Some("198.51.100.10:51820".into()),
                handshake_age_s,
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
        }
    }

    #[test]
    fn a_healthy_tunnel_shows_everything_the_handoff_asks_for() {
        let s = status(true, Some(21), 3);
        let d = display(Some(&s), Rates { rx: 222_000.0, tx: 41_000.0 });
        assert!(d.online && d.connected && d.installed);
        assert_eq!(d.status_text, "Connected");
        assert_eq!(d.location, "London");
        assert_eq!(d.iface, "yutani0");
        assert_eq!((d.accounts, d.accounts_label), (3, "Accounts connected"));
        assert_eq!(d.address, "10.2.0.2");
        assert_eq!(d.handshake, "hs 21s ago");
        assert_eq!((d.up_total.as_str(), d.up_rate.as_str()), ("2.79 GB", "41 KB/s"));
        assert_eq!((d.down_total.as_str(), d.down_rate.as_str()), ("413.1 MB", "222 KB/s"));
        assert!(!d.hidden);
    }

    #[test]
    fn one_account_is_singular() {
        let s = status(true, Some(1), 1);
        assert_eq!(display(Some(&s), Rates::default()).accounts_label, "Account connected");
    }

    #[test]
    fn disconnecting_freezes_the_totals_and_zeroes_everything_live() {
        let s = status(false, None, 2);
        let d = display(Some(&s), Rates { rx: 999_000.0, tx: 999_000.0 });
        assert!(d.online && !d.connected);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!(d.address, "—");
        assert_eq!(d.handshake, "hs —");
        // Counters, not gauges: the totals stay where they stopped.
        assert_eq!((d.up_total.as_str(), d.down_total.as_str()), ("2.79 GB", "413.1 MB"));
        assert_eq!((d.up_rate.as_str(), d.down_rate.as_str()), ("0 KB/s", "0 KB/s"));
    }

    #[test]
    fn a_stale_handshake_reads_as_disconnected_even_though_the_link_is_up() {
        let s = status(true, Some(180), 1);
        let d = display(Some(&s), Rates::default());
        assert!(!d.connected);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!(d.handshake, "hs 180s ago");
        // The link really is up, so the address still shows.
        assert_eq!(d.address, "10.2.0.2");
    }

    #[test]
    fn the_offline_state_shows_dashes_and_zeroes() {
        let d = display(None, Rates { rx: 5.0, tx: 5.0 });
        assert!(!d.online && !d.connected && !d.installed);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!((d.location.as_str(), d.iface.as_str()), ("—", "yutani0"));
        assert_eq!((d.accounts, d.accounts_label), (0, "Accounts connected"));
        assert_eq!((d.address.as_str(), d.handshake.as_str()), ("—", "hs —"));
        assert_eq!((d.up_total.as_str(), d.down_total.as_str()), ("0 KB", "0 KB"));
        assert_eq!((d.up_rate.as_str(), d.down_rate.as_str()), ("0 KB/s", "0 KB/s"));
    }
}
