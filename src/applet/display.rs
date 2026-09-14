//! Everything the popup's read-only area shows, as finished strings. The
//! view renders this and decides nothing; the rules live here where they
//! are tested. Spec §4.1–§4.3.

use crate::applet::format;
use crate::applet::icon::HANDSHAKE_STALE_S;
use crate::applet::rate::Rates;
use crate::applet::theme::DASH;
use crate::tunnel::status::Status;

/// One row of the services band (2026-09-14 spec §1): a dot, a name, a
/// note. `up` is the dot's colour — green or red, nothing in between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Service {
    pub name: &'static str,
    pub up: bool,
    pub note: &'static str,
}

/// What the applet can see of the tunnel *without* the daemon: the worker's
/// status file says whether the link is up, the unit file whether the
/// tunnel is installed at all. Only consulted in the offline state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OfflineTunnel {
    pub link_up: bool,
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// The daemon answered.
    pub online: bool,
    /// The services band: Yutani (the daemon), then WireGuard (the tunnel).
    pub services: [Service; 2],
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

/// Stale the last good reply so it presents as Disconnected (spec §7).
///
/// A poll that fails after a success keeps the daemon's last `status` on
/// screen — the totals are counters and must not jump back to zero — but
/// that reply is now old news, so it may not go on claiming a live tunnel
/// with a fresh handshake. Dropping the link and the handshake age is
/// enough: [`display`] derives the status text, the dot, the address and
/// the rates from them.
pub fn degrade(status: &mut Status) {
    status.tunnel.connected = false;
    status.tunnel.handshake_age_s = None;
}

/// The services band's two rows. Yutani is up when the daemon answered;
/// WireGuard is up on exactly the header's "Connected" (link up and a fresh
/// handshake), and its note says which of the red states it is in.
fn services(status: Option<&Status>, connected: bool, offline: OfflineTunnel) -> [Service; 2] {
    let yutani = Service { name: "Yutani", up: status.is_some(), note: if status.is_some() { "running" } else { "not running" } };
    let wireguard = match status {
        Some(s) => {
            let t = &s.tunnel;
            let note = if connected {
                "connected"
            } else if !t.installed {
                "not installed"
            } else if t.failed {
                "failed"
            } else if t.connected {
                "no handshake"
            } else {
                "disconnected"
            };
            Service { name: "WireGuard", up: connected, note }
        }
        None => Service {
            name: "WireGuard",
            up: offline.link_up,
            note: if offline.link_up {
                "connected"
            } else if offline.installed {
                "disconnected"
            } else {
                "not installed"
            },
        },
    };
    [yutani, wireguard]
}

pub fn display(status: Option<&Status>, rates: Rates, offline: OfflineTunnel) -> Display {
    let Some(s) = status else {
        return Display {
            online: false,
            services: services(None, false, offline),
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
        services: services(status, connected, offline),
        connected,
        status_text: if connected { "Connected" } else { "Disconnected" },
        location: t.location.clone(),
        iface: t.iface.clone(),
        accounts: s.clients.len(),
        accounts_label: format::accounts_label(s.clients.len()),
        // Spec §4.2's "tunnel IP" is the address the world sees EVE at, so
        // the public exit address wins whenever the worker has one. The
        // internal 10.2.0.2 is the fallback — it is at least *an* answer
        // while the first lookup is in flight, or against a daemon too old
        // to send the exit address at all.
        address: match (t.connected, t.exit_address.as_deref().or(t.address.as_deref())) {
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
                up_for_s: None,
                failed: false,
                exit_address: None,
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
        }
    }

    #[test]
    fn a_healthy_tunnel_shows_everything_the_handoff_asks_for() {
        let s = status(true, Some(21), 3);
        let d = display(Some(&s), Rates { rx: 222_000.0, tx: 41_000.0 }, OfflineTunnel::default());
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

    /// The band's right column is the *public* exit address when there is
    /// one: 10.2.0.2 is an implementation detail of the tunnel, not the IP
    /// the user is asking about.
    #[test]
    fn the_band_prefers_the_public_exit_address_over_the_internal_one() {
        let mut s = status(true, Some(21), 1);
        s.tunnel.exit_address = Some("198.51.100.10".into());
        assert_eq!(display(Some(&s), Rates::default(), OfflineTunnel::default()).address, "198.51.100.10");

        // No lookup yet (or an older daemon): the internal address is still
        // better than a dash.
        s.tunnel.exit_address = None;
        assert_eq!(display(Some(&s), Rates::default(), OfflineTunnel::default()).address, "10.2.0.2");

        // Neither: a dash, not an empty gap.
        s.tunnel.address = None;
        assert_eq!(display(Some(&s), Rates::default(), OfflineTunnel::default()).address, DASH);

        // Nothing is up, so neither address is shown.
        let mut down = status(false, None, 1);
        down.tunnel.exit_address = Some("198.51.100.10".into());
        assert_eq!(display(Some(&down), Rates::default(), OfflineTunnel::default()).address, DASH);
    }

    #[test]
    fn one_account_is_singular() {
        let s = status(true, Some(1), 1);
        assert_eq!(display(Some(&s), Rates::default(), OfflineTunnel::default()).accounts_label, "Account connected");
    }

    #[test]
    fn disconnecting_freezes_the_totals_and_zeroes_everything_live() {
        let s = status(false, None, 2);
        let d = display(Some(&s), Rates { rx: 999_000.0, tx: 999_000.0 }, OfflineTunnel::default());
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
        let d = display(Some(&s), Rates::default(), OfflineTunnel::default());
        assert!(!d.connected);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!(d.handshake, "hs 180s ago");
        // The link really is up, so the address still shows.
        assert_eq!(d.address, "10.2.0.2");
    }

    /// Spec §7: a poll that fails after a success leaves the last reply on
    /// screen, but that reply is now stale — it must not keep claiming a
    /// live tunnel with live rates.
    #[test]
    fn a_failed_poll_degrades_the_last_reply_to_disconnected() {
        let mut s = status(true, Some(21), 3);
        degrade(&mut s);
        let d = display(Some(&s), Rates { rx: 222_000.0, tx: 41_000.0 }, OfflineTunnel::default());
        // The daemon still answered once, so this is not the offline state.
        assert!(d.online && !d.connected);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!((d.address.as_str(), d.handshake.as_str()), ("—", "hs —"));
        assert_eq!((d.up_rate.as_str(), d.down_rate.as_str()), ("0 KB/s", "0 KB/s"));
        // Counters, not gauges: the totals stay where the last good poll
        // left them, as do the accounts and the location.
        assert_eq!((d.up_total.as_str(), d.down_total.as_str()), ("2.79 GB", "413.1 MB"));
        assert_eq!((d.accounts, d.location.as_str()), (3, "London"));
        assert!(d.installed);
    }

    /// Degrading twice is degrading once — `update` calls it on every failed
    /// poll, not only the first.
    #[test]
    fn degrading_is_idempotent() {
        let mut once = status(true, Some(21), 1);
        degrade(&mut once);
        let mut twice = once.clone();
        degrade(&mut twice);
        assert_eq!(once, twice);
    }

    /// 2026-09-14 spec §1: two dots, green or red, with the reason in a
    /// note. WireGuard is green on exactly the header's "Connected".
    #[test]
    fn the_services_band_says_which_of_the_two_is_up_and_why_not() {
        let svc = |s: &Status| display(Some(s), Rates::default(), OfflineTunnel::default()).services;
        let up = |name, note| Service { name, up: true, note };
        let down = |name, note| Service { name, up: false, note };

        assert_eq!(svc(&status(true, Some(21), 1)), [up("Yutani", "running"), up("WireGuard", "connected")]);
        assert_eq!(svc(&status(true, Some(180), 1)), [up("Yutani", "running"), down("WireGuard", "no handshake")]);
        assert_eq!(svc(&status(false, None, 1)), [up("Yutani", "running"), down("WireGuard", "disconnected")]);
        let mut failed = status(false, None, 1);
        failed.tunnel.failed = true;
        assert_eq!(svc(&failed)[1], down("WireGuard", "failed"));
        let mut missing = status(false, None, 1);
        missing.tunnel.installed = false;
        missing.tunnel.failed = true;
        assert_eq!(svc(&missing)[1], down("WireGuard", "not installed"), "not installed outranks failed");

        // Offline: Yutani is red, and WireGuard is read off the worker's
        // own file and the unit file, since there is no daemon to ask.
        let off = |o| display(None, Rates::default(), o).services;
        assert_eq!(
            off(OfflineTunnel { link_up: true, installed: true }),
            [down("Yutani", "not running"), up("WireGuard", "connected")]
        );
        assert_eq!(off(OfflineTunnel { link_up: false, installed: true })[1], down("WireGuard", "disconnected"));
        assert_eq!(off(OfflineTunnel { link_up: false, installed: false })[1], down("WireGuard", "not installed"));
    }

    #[test]
    fn the_offline_state_shows_dashes_and_zeroes() {
        let d = display(None, Rates { rx: 5.0, tx: 5.0 }, OfflineTunnel::default());
        assert!(!d.online && !d.connected && !d.installed);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!((d.location.as_str(), d.iface.as_str()), ("—", "yutani0"));
        assert_eq!((d.accounts, d.accounts_label), (0, "Accounts connected"));
        assert_eq!((d.address.as_str(), d.handshake.as_str()), ("—", "hs —"));
        assert_eq!((d.up_total.as_str(), d.down_total.as_str()), ("0 KB", "0 KB"));
        assert_eq!((d.up_rate.as_str(), d.down_rate.as_str()), ("0 KB/s", "0 KB/s"));
    }
}
