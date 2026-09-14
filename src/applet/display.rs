//! Everything the popover shows, as finished strings and small enums. The
//! view renders this and decides nothing; the rules live here where they
//! are tested. Redesign spec §2 / handoff "Screen 1".

use crate::applet::Action;
use crate::applet::format;
use crate::applet::icon::HANDSHAKE_STALE_S;
use crate::applet::rate::Rates;
use crate::tunnel::status::{ShortcutHint, Status};

pub const DASH: &str = "—";

/// One row of the accounts card: the hotkey digit, the character, and
/// whether it is the one the keyboard is driving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub index: usize,
    pub name: String,
    pub focused: bool,
}

/// The small button in the accounts card's footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbsButton {
    Hide,
    Show,
    /// The service is stopped: dim and inert.
    Off,
}

impl ThumbsButton {
    pub fn label(self) -> &'static str {
        match self {
            ThumbsButton::Hide => "Hide thumbnails",
            ThumbsButton::Show => "Show thumbnails",
            ThumbsButton::Off => "Thumbnails off",
        }
    }

    pub fn action(self) -> Option<Action> {
        match self {
            ThumbsButton::Hide => Some(Action::HideThumbs),
            ThumbsButton::Show => Some(Action::ShowThumbs),
            ThumbsButton::Off => None,
        }
    }
}

/// The tunnel status line: dot, name, city, uptime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelLine {
    /// The handoff's "connected": link up *and* a fresh handshake. Drives
    /// the dot, its glow ring and the uptime's colour.
    pub on: bool,
    pub location: String,
    /// `1h 12m 22s` while the link is up, `idle` otherwise.
    pub uptime: String,
}

/// The throughput card.
#[derive(Clone, Debug, PartialEq)]
pub struct Throughput {
    /// Number and unit, set apart so the unit can be smaller.
    pub up: (String, &'static str),
    pub down: (String, &'static str),
    /// `(up, down)` per column in `0..=1`, oldest first.
    pub bars: Vec<(f32, f32)>,
    pub sent: String,
    pub received: String,
}

/// The one primary action, in the handoff's three treatments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    /// Muted surface, dim text, no press.
    Inert(&'static str),
    Standard(&'static str, Action),
    Accent(&'static str, Action),
}

impl Primary {
    pub fn label(self) -> &'static str {
        match self {
            Primary::Inert(l) | Primary::Standard(l, _) | Primary::Accent(l, _) => l,
        }
    }

    pub fn action(self) -> Option<Action> {
        match self {
            Primary::Inert(_) => None,
            Primary::Standard(_, a) | Primary::Accent(_, a) => Some(a),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Popover {
    /// The daemon answered: the master switch is on.
    pub running: bool,
    pub state_word: &'static str,
    pub accounts: Vec<AccountRow>,
    /// The accounts card's count, mono.
    pub count: String,
    /// `Ctrl+Alt+1…4 · Ctrl+Alt+←/→`, or `hotkeys inactive`.
    pub hint: String,
    pub thumbs: ThumbsButton,
    pub tunnel: TunnelLine,
    pub throughput: Throughput,
    /// Whether the throughput card fits: on a short screen it is the first
    /// thing to drop (the rates still show nowhere else — the card is the
    /// whole of the traffic display).
    pub graph: bool,
    /// `ENDPOINT` and `PEER`.
    pub tiles: [(&'static str, String); 2],
    pub primary: Primary,
}

/// The popover's height in logical pixels, estimated from the handoff's
/// section sizes, for `accounts` rows, with or without the menu and the
/// graph card. Estimates, not measurements: iced does not lay the popup
/// out before it opens, and a few pixels either way only matter right at
/// the threshold.
pub fn popover_height(accounts: usize, menu_open: bool, graph: bool) -> i32 {
    let header = 50;
    let rows = if accounts == 0 { 44 } else { 30 * accounts as i32 };
    let accounts_card = 36 + rows + 44 + 12;
    let tunnel = 12 + 26 + 32;
    let graph_card = if graph { 172 + 12 } else { 0 };
    let tiles = 60 + 12;
    let action = 38 + 12;
    let menu = if menu_open { 8 + 3 * 31 + 8 } else { 0 };
    header + accounts_card + tunnel + graph_card + tiles + action + menu
}

/// Whether the graph card fits in `available` pixels (`None`: unknown, so
/// it is kept). The handoff's invariant: the popover never exceeds the
/// panel work area, and the graph card is the first thing to drop.
pub fn graph_fits(available: Option<i32>, accounts: usize, menu_open: bool) -> bool {
    available.is_none_or(|h| popover_height(accounts, menu_open, true) <= h)
}

/// Stale the last good reply so it presents as disconnected (spec §7).
///
/// A poll that fails after a success keeps the daemon's last `status` on
/// screen — the totals are counters and must not jump back to zero — but
/// that reply is now old news, so it may not go on claiming a live tunnel
/// with a fresh handshake.
pub fn degrade(status: &mut Status) {
    status.tunnel.connected = false;
    status.tunnel.handshake_age_s = None;
    status.tunnel.up_for_s = None;
}

/// The accounts card's footer hint, from the real bound prefix and the
/// real client count — never hardcoded. With nothing running the digits
/// are the whole range the shortcuts are installed for.
pub fn hotkey_hint(hint: Option<&ShortcutHint>, clients: usize) -> String {
    let Some(h) = hint else { return "hotkeys active".to_string() };
    let digits = match clients {
        0 => "1…9".to_string(),
        1 => "1".to_string(),
        n => format!("1…{}", n.min(9)),
    };
    format!("{p}+{digits} · {p}+{prev}/{next}", p = h.prefix, prev = h.prev, next = h.next)
}

/// The popover for `status` (`None` = the service is stopped), the live
/// rates, the graph's bars, the height the popover may take (`None`:
/// unknown) and whether the menu is open.
pub fn popover(status: Option<&Status>, rates: Rates, bars: Vec<(f32, f32)>, available: Option<i32>, menu_open: bool) -> Popover {
    let Some(s) = status else {
        // Stopping the service takes the tunnel idle with it: no part of
        // the popover may claim traffic is flowing while the overlay is
        // down (handoff invariant), whatever the worker is doing.
        return Popover {
            running: false,
            state_word: "Stopped",
            accounts: Vec::new(),
            count: "0".to_string(),
            hint: "hotkeys inactive".to_string(),
            thumbs: ThumbsButton::Off,
            tunnel: TunnelLine { on: false, location: DASH.to_string(), uptime: "idle".to_string() },
            throughput: Throughput {
                up: format::rate_parts(0.0),
                down: format::rate_parts(0.0),
                bars,
                sent: format::bytes(0),
                received: format::bytes(0),
            },
            graph: graph_fits(available, 0, menu_open),
            tiles: [("ENDPOINT", DASH.to_string()), ("PEER", crate::tunnel::IFACE.to_string())],
            primary: Primary::Inert("Start Yutani to route traffic"),
        };
    };
    let t = &s.tunnel;
    let fresh = t.connected && t.handshake_age_s.is_some_and(|age| age < HANDSHAKE_STALE_S);
    let live = |r: f64| if t.connected { r } else { 0.0 };
    let accounts = s
        .clients
        .iter()
        .enumerate()
        .map(|(i, c)| AccountRow { index: i + 1, name: c.name.clone(), focused: c.active })
        .collect::<Vec<_>>();
    let endpoint = if t.connected {
        t.exit_address
            .clone()
            .or_else(|| t.endpoint.as_ref().map(|e| e.rsplit_once(':').map_or(e.as_str(), |(host, _)| host).to_string()))
            .unwrap_or_else(|| DASH.to_string())
    } else {
        DASH.to_string()
    };
    let n = accounts.len();
    Popover {
        running: true,
        state_word: "Running",
        count: n.to_string(),
        hint: hotkey_hint(s.shortcuts.as_ref(), n),
        accounts,
        thumbs: if s.hidden { ThumbsButton::Show } else { ThumbsButton::Hide },
        tunnel: TunnelLine {
            on: fresh,
            location: t.location.clone(),
            uptime: if t.connected {
                t.up_for_s.map_or_else(|| DASH.to_string(), format::uptime)
            } else {
                "idle".to_string()
            },
        },
        throughput: Throughput {
            up: format::rate_parts(live(rates.tx)),
            down: format::rate_parts(live(rates.rx)),
            bars,
            sent: format::bytes(t.tx_bytes),
            received: format::bytes(t.rx_bytes),
        },
        graph: graph_fits(available, n, menu_open),
        tiles: [("ENDPOINT", endpoint), ("PEER", t.iface.clone())],
        primary: if !t.installed {
            Primary::Inert("No tunnel installed")
        } else if t.connected {
            Primary::Standard("Disconnect tunnel", Action::Disconnect)
        } else {
            Primary::Accent("Connect tunnel", Action::Connect)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::applet::rate::Rates;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    fn hint() -> ShortcutHint {
        ShortcutHint { prefix: "Ctrl+Alt".into(), next: "→".into(), prev: "←".into() }
    }

    fn status(connected: bool, handshake_age_s: Option<u64>, clients: usize) -> Status {
        Status {
            clients: (0..clients).map(|i| ClientStatus { name: format!("Pilot{i}"), active: i == 1 }).collect(),
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: Some("10.2.0.2".into()),
                endpoint: Some("198.51.100.10:51820".into()),
                handshake_age_s,
                up_for_s: connected.then_some(742),
                failed: false,
                exit_address: connected.then(|| "203.0.113.42".to_string()),
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
            shortcuts: Some(hint()),
            outputs: Vec::new(),
        }
    }

    fn flat() -> Vec<(f32, f32)> {
        vec![(0.0, 0.0); 34]
    }

    #[test]
    fn a_running_service_with_a_healthy_tunnel_fills_every_section() {
        let s = status(true, Some(21), 3);
        let p = popover(Some(&s), Rates { rx: 222_000.0, tx: 41_000.0 }, flat(), None, false);
        assert!(p.running);
        assert_eq!(p.state_word, "Running");
        assert_eq!(p.accounts.len(), 3);
        assert_eq!(p.accounts[1], AccountRow { index: 2, name: "Pilot1".into(), focused: true });
        assert!(!p.accounts[0].focused && !p.accounts[2].focused);
        assert_eq!(p.count, "3");
        assert_eq!(p.hint, "Ctrl+Alt+1…3 · Ctrl+Alt+←/→");
        assert_eq!(p.thumbs, ThumbsButton::Hide);
        assert_eq!(p.tunnel, TunnelLine { on: true, location: "London".into(), uptime: "12m 22s".into() });
        assert_eq!(p.throughput.up, ("41".to_string(), "KB/s"));
        assert_eq!(p.throughput.down, ("222".to_string(), "KB/s"));
        assert_eq!((p.throughput.sent.as_str(), p.throughput.received.as_str()), ("2.79 GB", "413.1 MB"));
        assert_eq!(p.throughput.bars.len(), 34);
        assert_eq!(p.tiles[0], ("ENDPOINT", "203.0.113.42".to_string()));
        assert_eq!(p.tiles[1], ("PEER", "yutani0".to_string()));
        assert_eq!(p.primary, Primary::Standard("Disconnect tunnel", Action::Disconnect));
    }

    /// The handoff's invariant: a stopped service takes the tunnel idle
    /// with it — dot off, uptime "idle", rates 0, tiles `—`, the primary
    /// button inert, and no claim of traffic anywhere.
    #[test]
    fn a_stopped_service_reads_idle_everywhere() {
        let p = popover(None, Rates { rx: 999.0, tx: 999.0 }, flat(), None, false);
        assert!(!p.running);
        assert_eq!(p.state_word, "Stopped");
        assert!(p.accounts.is_empty());
        assert_eq!((p.count.as_str(), p.hint.as_str()), ("0", "hotkeys inactive"));
        assert_eq!(p.thumbs, ThumbsButton::Off);
        assert_eq!(p.thumbs.action(), None);
        assert_eq!(p.tunnel, TunnelLine { on: false, location: DASH.into(), uptime: "idle".into() });
        assert_eq!(p.throughput.up, ("0".to_string(), "KB/s"));
        assert_eq!(p.tiles[0].1, DASH);
        assert_eq!(p.tiles[1].1, "yutani0");
        assert_eq!(p.primary, Primary::Inert("Start Yutani to route traffic"));
        assert_eq!(p.primary.action(), None);
    }

    #[test]
    fn a_disconnected_tunnel_offers_connect_in_the_accent_and_shows_no_endpoint() {
        let s = status(false, None, 1);
        let p = popover(Some(&s), Rates { rx: 5.0, tx: 5.0 }, flat(), None, false);
        assert_eq!(p.primary, Primary::Accent("Connect tunnel", Action::Connect));
        assert_eq!(p.tunnel.uptime, "idle");
        assert!(!p.tunnel.on);
        assert_eq!(p.tiles[0].1, DASH);
        assert_eq!(p.throughput.up, ("0".to_string(), "KB/s"), "rates are zero while down");
        // Counters, not gauges: the totals stay where they stopped.
        assert_eq!(p.throughput.sent, "2.79 GB");
        assert_eq!(p.hint, "Ctrl+Alt+1 · Ctrl+Alt+←/→");
    }

    /// Up but silent: the link is there (Disconnect is offered, the uptime
    /// counts) but the dot is off — the header's "Connected" needs a fresh
    /// handshake.
    #[test]
    fn a_stale_handshake_keeps_the_link_but_turns_the_dot_off() {
        let s = status(true, Some(180), 1);
        let p = popover(Some(&s), Rates::default(), flat(), None, false);
        assert!(!p.tunnel.on);
        assert_eq!(p.tunnel.uptime, "12m 22s");
        assert_eq!(p.primary, Primary::Standard("Disconnect tunnel", Action::Disconnect));
    }

    #[test]
    fn an_uninstalled_tunnel_has_an_inert_primary_button() {
        let mut s = status(false, None, 0);
        s.tunnel.installed = false;
        let p = popover(Some(&s), Rates::default(), flat(), None, false);
        assert_eq!(p.primary, Primary::Inert("No tunnel installed"));
        assert_eq!(p.hint, "Ctrl+Alt+1…9 · Ctrl+Alt+←/→", "no clients: the whole installed range");
    }

    #[test]
    fn the_endpoint_tile_falls_back_to_the_peers_host_and_the_hint_to_a_generic_line() {
        let mut s = status(true, Some(2), 12);
        s.tunnel.exit_address = None;
        s.shortcuts = None;
        let p = popover(Some(&s), Rates::default(), flat(), None, false);
        assert_eq!(p.tiles[0].1, "198.51.100.10");
        assert_eq!(p.hint, "hotkeys active", "an older daemon sends no prefix");
        s.shortcuts = Some(hint());
        assert_eq!(popover(Some(&s), Rates::default(), flat(), None, false).hint, "Ctrl+Alt+1…9 · Ctrl+Alt+←/→", "capped at 9");
        s.hidden = true;
        assert_eq!(popover(Some(&s), Rates::default(), flat(), None, false).thumbs, ThumbsButton::Show);
    }

    /// The handoff's short-screen invariant: the graph card is the first
    /// thing to drop, and only when the estimate says it would not fit.
    #[test]
    fn the_graph_card_is_dropped_first_on_a_short_screen() {
        let s = status(true, Some(21), 3);
        assert!(popover(Some(&s), Rates::default(), flat(), None, false).graph, "unknown height keeps it");
        assert!(popover(Some(&s), Rates::default(), flat(), Some(1360), false).graph, "a 1440 screen");
        let full = popover_height(3, false, true);
        let without = popover_height(3, false, false);
        assert_eq!(full - without, 184, "the card and its margin");
        assert!(popover(Some(&s), Rates::default(), flat(), Some(full), false).graph, "exactly fits");
        assert!(!popover(Some(&s), Rates::default(), flat(), Some(full - 1), false).graph, "one pixel short");
        // Opening the menu adds to the estimate; more rows too.
        assert!(popover_height(3, true, true) > full);
        assert!(popover_height(9, false, true) > full);
        assert!(!popover(None, Rates::default(), flat(), Some(200), false).graph, "stopped and short: gone");
    }

    #[test]
    fn a_failed_poll_degrades_the_last_reply_to_disconnected() {
        let mut s = status(true, Some(21), 3);
        degrade(&mut s);
        let p = popover(Some(&s), Rates { rx: 222_000.0, tx: 41_000.0 }, flat(), None, false);
        assert!(p.running && !p.tunnel.on);
        assert_eq!(p.tunnel.uptime, "idle");
        assert_eq!(p.throughput.up, ("0".to_string(), "KB/s"));
        assert_eq!(p.throughput.sent, "2.79 GB");
        assert_eq!(p.accounts.len(), 3);
        let mut twice = s.clone();
        degrade(&mut twice);
        assert_eq!(s, twice, "degrading is idempotent");
    }
}
