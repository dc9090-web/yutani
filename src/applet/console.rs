//! Everything the Nostromo popover shows (handoff v6 "Screen"), as
//! finished strings and small enums. The view renders a `Console` and
//! decides nothing; the rules live here, where they are tested.

use crate::applet::Action;
use crate::applet::format;
use crate::applet::host::HostReading;
use crate::applet::icon::HANDSHAKE_STALE_S;
use crate::applet::ping::{PingSummary, PingWindow};
use crate::applet::rate::Rates;
use crate::applet::skin::Ink;
pub use crate::launch_eve::Step;
use crate::tunnel::status::Status;

pub const DASH: &str = "—";

pub struct Inputs<'a> {
    pub rates: Rates,
    /// Session totals `(rx, tx)`: the counters while connected, frozen at
    /// their last value otherwise (the interface is per session, so its
    /// counters *are* the session's).
    pub totals: (u64, u64),
    pub host: &'a HostReading,
    pub ping: &'a PingWindow,
    /// A connect (`true`) or disconnect (`false`) is still settling.
    pub tunnel_pending: Option<bool>,
    /// A start or quit of the service is still settling.
    pub service_pending: bool,
    /// The height the popover may take (`None`: unknown).
    pub available: Option<i32>,
    pub menu_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RockerState {
    Off,
    On,
    Pending,
    Disabled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControlRow {
    pub title: &'static str,
    pub sub: String,
    pub sub_ink: Ink,
    pub led: Ink,
    pub glow: bool,
    pub rocker: RockerState,
    pub press: Option<Action>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub index: usize,
    pub name: String,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accounts {
    Stopped,
    Empty,
    Rows(Vec<AccountRow>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    /// Tunnel up and service running: wind blows, readouts are live.
    pub live: bool,
    pub up: String,
    pub down: String,
    pub tx_total: String,
    pub rx_total: String,
    pub up_kbps: f32,
    pub down_kbps: f32,
    /// The scope band fits (short-screen rule).
    pub scope: bool,
    pub ping: PingSummary,
    pub endpoint: String,
    pub peer: String,
    pub uptime: String,
    pub uptime_ink: Ink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warn,
    Crit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gauge {
    pub label: &'static str,
    pub lit: usize,
    pub level: Level,
    pub value: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchButton {
    Ready,
    Launching { step: u8 },
    Inert(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub text: String,
    pub status: Step,
}

/// The log card's height: padding, 4 lines, gaps, border and margin.
pub const LOG_CARD_PX: i32 = 7 + 4 * 13 + 3 * 3 + 8 + 2 + 10;

#[derive(Clone, Debug, PartialEq)]
pub struct Console {
    pub running: bool,
    pub state_word: &'static str,
    pub control: [ControlRow; 2],
    pub accounts_label: &'static str,
    /// `02`, only when running with more than one client.
    pub count: Option<String>,
    pub accounts: Accounts,
    pub network: Network,
    pub host_meta: String,
    pub host: [Gauge; 3],
    pub notice: Option<String>,
    pub launch: LaunchButton,
    /// The four-step Launch EVE log, while a launch is on show.
    pub launch_log: Option<Vec<LogLine>>,
    pub launch_failed: Option<String>,
    pub footer: String,
}

/// Lit cells out of 20 and the colour band: amber from 70 %, red from 90 %.
pub fn gauge_cells(pct: Option<f32>) -> (usize, Level) {
    let Some(p) = pct else { return (0, Level::Normal) };
    let p = p.clamp(0.0, 100.0);
    let lit = (p / 100.0 * crate::applet::skin::GAUGE_CELLS as f32).round() as usize;
    let level = if p >= 90.0 {
        Level::Crit
    } else if p >= 70.0 {
        Level::Warn
    } else {
        Level::Normal
    };
    (lit, level)
}

fn gauge(label: &'static str, pct: Option<f32>, detail: String) -> Gauge {
    let (lit, level) = gauge_cells(pct);
    Gauge { label, lit, level, value: pct.map_or_else(|| DASH.to_string(), |p| format!("{}%", p.round() as i64)), detail }
}

fn host_gauges(h: &HostReading) -> [Gauge; 3] {
    let temp = |t: Option<f32>| t.map_or_else(|| DASH.to_string(), |t| format!("{}°C", t.round() as i64));
    let ram_pct = match (h.ram_used, h.ram_total) {
        (Some(u), Some(t)) if t > 0 => Some(u as f32 / t as f32 * 100.0),
        _ => None,
    };
    [
        gauge("CPU", h.cpu, temp(h.cpu_temp)),
        gauge("GPU", h.gpu, temp(h.gpu_temp)),
        gauge("RAM", ram_pct, h.ram_used.map_or_else(|| DASH.to_string(), format::gb1)),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TunnelState {
    Stopped,
    NotInstalled,
    Connecting,
    Stale,
    Connected,
    Idle,
    /// systemd reports the tunnel unit failed.
    Failed,
}

fn tunnel_state(status: Option<&Status>, pending: Option<bool>) -> TunnelState {
    let Some(s) = status else { return TunnelState::Stopped };
    let t = &s.tunnel;
    if !t.installed {
        return TunnelState::NotInstalled;
    }
    if pending == Some(true) && !t.connected {
        return TunnelState::Connecting;
    }
    if pending == Some(false) && t.connected {
        // Disconnect pressed: show it at once, not when the poll catches up.
        return TunnelState::Idle;
    }
    match (t.connected, t.handshake_age_s) {
        (false, _) if t.failed => TunnelState::Failed,
        (false, _) => TunnelState::Idle,
        (true, None) if t.up_for_s.is_none_or(|up| up < HANDSHAKE_STALE_S) => TunnelState::Connecting,
        (true, None) => TunnelState::Stale,
        (true, Some(age)) if age >= HANDSHAKE_STALE_S => TunnelState::Stale,
        (true, Some(_)) => TunnelState::Connected,
    }
}

fn tunnel_row(state: TunnelState, location: &str) -> ControlRow {
    let loc = location.to_uppercase();
    let (sub, ink, rocker, press) = match state {
        TunnelState::Stopped => ("START YUTANI TO ROUTE TRAFFIC".to_string(), Ink::Dimmer, RockerState::Disabled, None),
        TunnelState::NotInstalled => ("NO TUNNEL INSTALLED · PREFERENCES → TUNNEL".to_string(), Ink::Dimmer, RockerState::Disabled, None),
        TunnelState::Connecting => (format!("CONNECTING · {loc}"), Ink::Amber, RockerState::Pending, Some(Action::Disconnect)),
        TunnelState::Stale => (format!("{loc} · HANDSHAKE STALE"), Ink::Amber, RockerState::On, Some(Action::Disconnect)),
        TunnelState::Connected => (format!("CONNECTED · {loc}"), Ink::Dim, RockerState::On, Some(Action::Disconnect)),
        TunnelState::Idle => (format!("IDLE · {loc}"), Ink::Dimmer, RockerState::Off, Some(Action::Connect)),
        TunnelState::Failed => (format!("TUNNEL FAILED · {loc}"), Ink::Amber, RockerState::Off, Some(Action::Connect)),
    };
    let led = match state {
        TunnelState::Connected => Ink::Phosphor,
        TunnelState::Connecting | TunnelState::Stale | TunnelState::Failed => Ink::Amber,
        _ => Ink::Dimmer,
    };
    ControlRow { title: "WIREGUARD TUNNEL", sub, sub_ink: ink, led, glow: state == TunnelState::Connected, rocker, press }
}

fn service_row(running: bool, pending: bool) -> ControlRow {
    let rocker = if pending { RockerState::Pending } else if running { RockerState::On } else { RockerState::Off };
    if running {
        ControlRow {
            title: "YUTANI SERVICE",
            sub: "MULTIBOX · HOTKEYS · ROUTING ACTIVE".into(),
            sub_ink: Ink::Dim,
            led: Ink::Phosphor,
            glow: true,
            rocker,
            press: Some(Action::Quit),
        }
    } else {
        ControlRow {
            title: "YUTANI SERVICE",
            sub: "STOPPED · MULTIBOX, HOTKEYS, ROUTING OFF".into(),
            sub_ink: Ink::Dimmer,
            led: Ink::Dimmer,
            glow: false,
            rocker,
            press: Some(Action::StartDaemon),
        }
    }
}

/// The scope band's share of the height: 104 + its hairline.
pub const SCOPE_BAND_PX: i32 = 105;

/// The popover's height in logical px, estimated from the handoff's
/// geometry (iced does not lay a popup out before it opens). Measured on the
/// panel on 2026-10-07: the real popover was 679 px tall for running, tunnel
/// connected, 0 clients (`Accounts::Empty`), menu closed, no notice, no
/// note, scope shown — before Phase 2 moved JIT · LOSS under MIN/AVG/MAX,
/// which adds one 8 px stat line (+10) to the ping row.
pub fn console_height(c: &Console, menu_open: bool) -> i32 {
    const HEADER: i32 = 48;
    const SECTION_HEAD: i32 = 23;
    const CONTROL: i32 = 2 * 46 + 1 + 2;
    const PING_AND_FACTS: i32 = 60 + 1 + 27 + 2;
    const HOST: i32 = 9 + 3 * 14 + 2 * 6 + 9 + 2;
    const ACTION: i32 = 36;
    const MENU: i32 = 1 + 2 * 5 + 3 * 28 + 2;
    const FOOTER: i32 = 1 + 7 + 12 + 8;
    const GAP: i32 = 10;
    let accounts = match &c.accounts {
        Accounts::Rows(rows) => 2 * 5 + 28 * rows.len() as i32 + (rows.len() as i32 - 1).max(0) + 2,
        _ => 10 + 14 + 3 + 26 + 11 + 2,
    };
    let scope = if c.network.scope { SCOPE_BAND_PX } else { 0 };
    let notice = c.notice.as_ref().map_or(0, |n| 4 + 7 + 8 + 13 * n.chars().count().div_ceil(52).max(1) as i32 + 2 + GAP);
    let menu = if menu_open { MENU } else { 0 };
    let log = if c.launch_log.is_some() { LOG_CARD_PX + if c.launch_failed.is_some() { 13 } else { 0 } } else { 0 };
    HEADER
        + 4 * SECTION_HEAD
        + CONTROL + GAP
        + accounts + GAP
        + scope + PING_AND_FACTS + GAP
        + HOST + GAP
        + notice
        + ACTION + GAP
        + log
        + menu
        + FOOTER
}

fn launch_button(status: Option<&Status>) -> LaunchButton {
    let Some(s) = status else { return LaunchButton::Inert("START YUTANI TO LAUNCH") };
    if s.clients.len() >= 9 {
        return LaunchButton::Inert("ALL 9 SLOTS IN USE");
    }
    match &s.launch {
        Some(l) if !l.finished() && !l.is_failed() => LaunchButton::Launching { step: l.current() },
        _ => LaunchButton::Ready,
    }
}

fn launch_log(l: &crate::launch_eve::LaunchState) -> Vec<LogLine> {
    let texts = [
        "STEAM · APPLAUNCH 8500".to_string(),
        "STEAM · WINDOW MINIMISED".to_string(),
        "EVE CLIENT · STARTING".to_string(),
        format!("HOTKEY ASSIGNED · {}", l.hotkey),
    ];
    texts.into_iter().zip(l.steps).map(|(text, status)| LogLine { text, status }).collect()
}

/// The Steam notice, shouted like the rest of the popover, except that a
/// filesystem path inside it keeps its case (paths are case-sensitive).
fn steam_notice(findings: &[crate::steam::Finding]) -> Option<String> {
    use crate::steam::Verdict;
    let f = findings.iter().find(|f| f.verdict.message().is_some())?;
    let msg = f.verdict.message()?;
    let mut shouted = msg.to_uppercase();
    if let Verdict::Broken { path } = &f.verdict {
        shouted = shouted.replace(&path.to_uppercase(), path);
    }
    Some(format!("{shouted} OPEN SETTINGS → STEAM."))
}

pub fn console(status: Option<&Status>, i: &Inputs) -> Console {
    let running = status.is_some();
    let tstate = tunnel_state(status, i.tunnel_pending);
    let location = status.map_or("", |s| s.tunnel.location.as_str());
    let live = running && status.is_some_and(|s| s.tunnel.connected);
    let rate = |r: f64| if live { r } else { 0.0 };
    let (rx_total, tx_total) = i.totals;
    let (accounts, count, accounts_label) = match status {
        None => (Accounts::Stopped, None, "ACCOUNTS"),
        Some(s) if s.clients.is_empty() => (Accounts::Empty, None, "ACCOUNTS"),
        Some(s) => {
            let rows: Vec<AccountRow> = s
                .clients
                .iter()
                .enumerate()
                .map(|(n, c)| AccountRow { index: n + 1, name: c.name.to_uppercase(), focused: c.active })
                .collect();
            let n = rows.len();
            (Accounts::Rows(rows), (n > 1).then(|| format!("{n:02}")), if n == 1 { "ACCOUNT" } else { "ACCOUNTS" })
        }
    };
    let t = status.map(|s| &s.tunnel);
    let endpoint = t
        .filter(|t| t.connected)
        .and_then(|t| t.endpoint.as_ref().map(|e| e.rsplit_once(':').map_or(e.as_str(), |(h, _)| h).to_string()))
        .unwrap_or_else(|| DASH.to_string());
    let (uptime, uptime_ink) = match t.filter(|t| t.connected).and_then(|t| t.up_for_s) {
        Some(secs) if running => (format::mission_clock(secs), Ink::Phosphor),
        _ => ("IDLE".to_string(), Ink::Dimmer),
    };
    let ram_total = i.host.ram_total.map_or_else(|| DASH.to_string(), format::gb_ceil);
    let notice = status.and_then(|s| steam_notice(&s.steam));
    let mut c = Console {
        running,
        state_word: if running { "RUNNING" } else { "STOPPED" },
        control: [service_row(running, i.service_pending), tunnel_row(tstate, location)],
        accounts_label,
        count,
        accounts,
        network: Network {
            live,
            up: format::kb_int(rate(i.rates.tx)),
            down: format::kb_int(rate(i.rates.rx)),
            tx_total: format!("{} TX", format::mb1(tx_total)),
            rx_total: format!("{} RX", format::mb1(rx_total)),
            up_kbps: (rate(i.rates.tx) / 1_000.0) as f32,
            down_kbps: (rate(i.rates.rx) / 1_000.0) as f32,
            scope: true,
            ping: i.ping.summary(live),
            endpoint,
            peer: t.map_or_else(|| DASH.to_string(), |t| t.iface.clone()),
            uptime,
            uptime_ink,
        },
        host_meta: format!("{ram_total} · 1 HZ"),
        host: host_gauges(i.host),
        notice,
        launch: launch_button(status),
        launch_log: status.and_then(|s| s.launch.as_ref()).map(launch_log),
        launch_failed: status.and_then(|s| s.launch.as_ref()).and_then(|l| l.failed.clone()),
        footer: format!("YUTANI OS · BUILD {}", env!("YUTANI_BUILD")),
    };
    c.network.scope = i.available.is_none_or(|h| console_height(&c, i.menu_open) <= h);
    c
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::{ClientStatus, TunnelStatus};

    fn status(connected: bool, handshake_age_s: Option<u64>, clients: usize) -> Status {
        Status {
            clients: (0..clients).map(|i| ClientStatus { name: format!("Pilot {i}"), active: i == 0 }).collect(),
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: None,
                endpoint: connected.then(|| "203.0.113.42:51820".into()),
                handshake_age_s,
                up_for_s: connected.then_some(5_880),
                failed: false,
                exit_address: None,
                ping_us: None,
                ping_seq: 0,
                rx_bytes: 693_600_000,
                tx_bytes: 105_800_000,
            },
            shortcuts: None,
            outputs: Vec::new(),
            steam: Vec::new(),
            launch: None,
        }
    }

    fn inputs<'a>(host: &'a HostReading, ping: &'a PingWindow) -> Inputs<'a> {
        Inputs { rates: Rates { tx: 1_000.0, rx: 8_000.0 }, totals: (693_600_000, 105_800_000), host, ping, tunnel_pending: None, service_pending: false, available: None, menu_open: false }
    }

    fn launching(steps: [Step; 4], failed: Option<&str>) -> Status {
        let mut s = status(true, Some(4), 2);
        s.launch = Some(crate::launch_eve::LaunchState { steps, hotkey: "CTRL+ALT+3".into(), failed: failed.map(Into::into) });
        s
    }

    #[test]
    fn the_button_follows_the_service_the_slots_and_the_launch() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let i = inputs(&h, &p);
        assert_eq!(console(None, &i).launch, LaunchButton::Inert("START YUTANI TO LAUNCH"));
        assert_eq!(console(Some(&status(true, Some(4), 2)), &i).launch, LaunchButton::Ready);
        assert_eq!(console(Some(&status(true, Some(4), 9)), &i).launch, LaunchButton::Inert("ALL 9 SLOTS IN USE"));
        use Step::*;
        let c = console(Some(&launching([Done, Done, Running, Pending], None)), &i);
        assert_eq!(c.launch, LaunchButton::Launching { step: 3 });
        assert_eq!(console(Some(&launching([Done; 4], None)), &i).launch, LaunchButton::Ready);
        assert_eq!(console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &i).launch, LaunchButton::Ready);
    }

    #[test]
    fn the_log_names_each_step_and_the_hotkey() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        use Step::*;
        let c = console(Some(&launching([Done, Done, Running, Pending], None)), &inputs(&h, &p));
        let log = c.launch_log.expect("log while launching");
        let texts: Vec<&str> = log.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["STEAM · APPLAUNCH 8500", "STEAM · WINDOW MINIMISED", "EVE CLIENT · STARTING", "HOTKEY ASSIGNED · CTRL+ALT+3"]);
        assert_eq!(log[2].status, Running);
        assert!(console(Some(&status(true, Some(4), 2)), &inputs(&h, &p)).launch_log.is_none());
        let f = console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &inputs(&h, &p));
        assert_eq!(f.launch_failed.as_deref(), Some("LAUNCHER CLOSED"));
    }

    #[test]
    fn the_log_card_counts_in_the_height() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        use Step::*;
        let plain = console(Some(&status(true, Some(4), 2)), &inputs(&h, &p));
        let busy = console(Some(&launching([Done, Done, Running, Pending], None)), &inputs(&h, &p));
        assert_eq!(console_height(&busy, false) - console_height(&plain, false), LOG_CARD_PX);
        let failed = console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &inputs(&h, &p));
        assert_eq!(console_height(&failed, false) - console_height(&busy, false), 13);
    }

    #[test]
    fn running_and_connected_fills_every_section() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let c = console(Some(&status(true, Some(4), 2)), &inputs(&h, &p));
        assert!(c.running);
        assert_eq!(c.state_word, "RUNNING");
        assert_eq!(c.control[0].sub, "MULTIBOX · HOTKEYS · ROUTING ACTIVE");
        assert_eq!(c.control[0].rocker, RockerState::On);
        assert_eq!(c.control[0].press, Some(Action::Quit));
        assert_eq!((c.control[1].sub.as_str(), c.control[1].sub_ink), ("CONNECTED · LONDON", Ink::Dim));
        assert_eq!((c.control[1].rocker, c.control[1].press), (RockerState::On, Some(Action::Disconnect)));
        assert_eq!((c.accounts_label, c.count.as_deref()), ("ACCOUNTS", Some("02")));
        let Accounts::Rows(rows) = &c.accounts else { panic!() };
        assert_eq!(rows[0], AccountRow { index: 1, name: "PILOT 0".into(), focused: true });
        let n = &c.network;
        assert_eq!((n.up.as_str(), n.down.as_str()), ("1", "8"));
        assert_eq!((n.tx_total.as_str(), n.rx_total.as_str()), ("105.8 MB TX", "693.6 MB RX"));
        assert_eq!((n.endpoint.as_str(), n.peer.as_str()), ("203.0.113.42", "yutani0"));
        assert_eq!((n.uptime.as_str(), n.uptime_ink), ("T+ 1H 38M", Ink::Phosphor));
        assert!(n.live && n.scope);
        assert_eq!(c.launch, LaunchButton::Ready);
        assert!(c.footer.starts_with("YUTANI OS · BUILD "));
    }

    #[test]
    fn a_stopped_service_reads_stopped_everywhere() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let c = console(None, &inputs(&h, &p));
        assert_eq!((c.running, c.state_word), (false, "STOPPED"));
        assert_eq!(c.control[0].sub, "STOPPED · MULTIBOX, HOTKEYS, ROUTING OFF");
        assert_eq!((c.control[0].rocker, c.control[0].press), (RockerState::Off, Some(Action::StartDaemon)));
        assert_eq!((c.control[1].sub.as_str(), c.control[1].rocker, c.control[1].press), ("START YUTANI TO ROUTE TRAFFIC", RockerState::Disabled, None));
        assert_eq!(c.accounts, Accounts::Stopped);
        assert_eq!(c.network.peer, "—", "no service, no interface to name");
        assert_eq!(c.count, None);
        assert_eq!((c.network.up.as_str(), c.network.down.as_str()), ("0", "0"));
        assert_eq!((c.network.endpoint.as_str(), c.network.uptime.as_str(), c.network.uptime_ink), ("—", "IDLE", Ink::Dimmer));
        assert_eq!(c.network.tx_total, "105.8 MB TX", "totals freeze, they do not reset");
        assert!(!c.network.live);
    }

    #[test]
    fn every_tunnel_state_has_its_copy() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let i = inputs(&h, &p);
        let sub = |s: &Status, i: &Inputs| {
            let c = console(Some(s), i);
            (c.control[1].sub.clone(), c.control[1].sub_ink, c.control[1].rocker)
        };
        let mut not_installed = status(false, None, 0);
        not_installed.tunnel.installed = false;
        assert_eq!(sub(&not_installed, &i), ("NO TUNNEL INSTALLED · PREFERENCES → TUNNEL".into(), Ink::Dimmer, RockerState::Disabled));
        assert_eq!(sub(&status(false, None, 0), &i), ("IDLE · LONDON".into(), Ink::Dimmer, RockerState::Off));
        assert_eq!(sub(&status(true, Some(200), 0), &i), ("LONDON · HANDSHAKE STALE".into(), Ink::Amber, RockerState::On));
        let mut just_up = status(true, None, 0);
        just_up.tunnel.up_for_s = Some(5);
        assert_eq!(sub(&just_up, &i), ("CONNECTING · LONDON".into(), Ink::Amber, RockerState::Pending), "up, never handshaked, just up");
        let mut never = status(true, None, 0);
        never.tunnel.up_for_s = Some(HANDSHAKE_STALE_S);
        assert_eq!(sub(&never, &i).1, Ink::Amber, "up for 3 min without a handshake is stale");
        let pending = Inputs { tunnel_pending: Some(true), ..inputs(&h, &p) };
        assert_eq!(sub(&status(false, None, 0), &pending), ("CONNECTING · LONDON".into(), Ink::Amber, RockerState::Pending));
    }

    #[test]
    fn a_failed_unit_reads_failed_and_offers_connect() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let mut s = status(false, None, 0);
        s.tunnel.failed = true;
        let c = console(Some(&s), &inputs(&h, &p));
        let row = &c.control[1];
        assert_eq!((row.sub.as_str(), row.sub_ink, row.led), ("TUNNEL FAILED · LONDON", Ink::Amber, Ink::Amber));
        assert_eq!((row.rocker, row.press), (RockerState::Off, Some(Action::Connect)));
    }

    #[test]
    fn a_pending_disconnect_shows_idle_at_once() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let i = Inputs { tunnel_pending: Some(false), ..inputs(&h, &p) };
        let c = console(Some(&status(true, Some(4), 0)), &i);
        let row = &c.control[1];
        assert_eq!((row.sub.as_str(), row.rocker, row.press), ("IDLE · LONDON", RockerState::Off, Some(Action::Connect)));
    }

    /// Measured on the panel (2026-10-07): 679 px, plus the Phase 2 stat
    /// line: 689.
    #[test]
    fn the_height_estimate_matches_the_measured_popover() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let c = console(Some(&status(true, Some(4), 0)), &inputs(&h, &p));
        assert_eq!(c.accounts, Accounts::Empty);
        assert!(c.notice.is_none() && c.network.scope);
        let est = console_height(&c, false);
        assert!((est - 689).abs() <= 10, "{est}");
    }

    #[test]
    fn accounts_label_count_and_empty_state() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let one = console(Some(&status(true, Some(4), 1)), &inputs(&h, &p));
        assert_eq!((one.accounts_label, one.count.clone()), ("ACCOUNT", None));
        let none = console(Some(&status(true, Some(4), 0)), &inputs(&h, &p));
        assert_eq!(none.accounts, Accounts::Empty);
    }

    #[test]
    fn gauges_light_twenty_cells_and_turn_amber_then_red() {
        assert_eq!(gauge_cells(None), (0, Level::Normal));
        assert_eq!(gauge_cells(Some(23.0)), (5, Level::Normal));
        assert_eq!(gauge_cells(Some(69.9)), (14, Level::Normal));
        assert_eq!(gauge_cells(Some(70.0)), (14, Level::Warn));
        assert_eq!(gauge_cells(Some(90.0)), (18, Level::Crit));
        assert_eq!(gauge_cells(Some(140.0)), (20, Level::Crit));
        let h = HostReading { cpu: Some(23.4), gpu: Some(40.0), ram_used: Some(10_952_166_604), ram_total: Some(33_438_498_816), cpu_temp: Some(46.1), gpu_temp: None };
        let p = PingWindow::default();
        let c = console(None, &inputs(&h, &p));
        assert_eq!((c.host[0].value.as_str(), c.host[0].detail.as_str()), ("23%", "46°C"));
        assert_eq!((c.host[1].value.as_str(), c.host[1].detail.as_str()), ("40%", "—"));
        assert_eq!((c.host[2].value.as_str(), c.host[2].detail.as_str()), ("33%", "10.2 GB"));
        assert_eq!(c.host_meta, "32 GB · 1 HZ");
    }

    #[test]
    fn the_steam_notice_is_shouted_with_the_settings_pointer() {
        use crate::steam::{Finding, Verdict};
        let (h, p) = (HostReading::default(), PingWindow::default());
        let mut s = status(true, Some(4), 0);
        s.steam = vec![Finding { verdict: Verdict::NoWrapper, file: std::path::PathBuf::from("/home/user/.steam/steam/userdata/1/config/localconfig.vdf") }];
        let c = console(Some(&s), &inputs(&h, &p));
        let n = c.notice.expect("notice");
        assert!(n.ends_with("OPEN SETTINGS → STEAM."), "{n}");
        assert_eq!(n, n.to_uppercase());
    }

    #[test]
    fn a_path_in_the_steam_notice_keeps_its_case() {
        use crate::steam::{Finding, Verdict};
        let (h, p) = (HostReading::default(), PingWindow::default());
        let mut s = status(true, Some(4), 0);
        let path = "/home/Daniel/bin/yutani".to_string();
        s.steam = vec![Finding { verdict: Verdict::Broken { path: path.clone() }, file: std::path::PathBuf::from("/x/localconfig.vdf") }];
        let n = console(Some(&s), &inputs(&h, &p)).notice.expect("notice");
        assert!(n.contains(&path), "{n}");
        assert!(n.starts_with("STEAM LAUNCHES EVE THROUGH /home/Daniel/bin/yutani, WHICH IS MISSING."), "{n}");
        assert!(n.ends_with("OPEN SETTINGS → STEAM."), "{n}");
    }

    /// The scope band is the first (and only) thing to drop on a short screen.
    #[test]
    fn the_scope_drops_when_the_popover_would_not_fit() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let s = status(true, Some(4), 2);
        let tall = console(Some(&s), &Inputs { available: Some(2_000), ..inputs(&h, &p) });
        let full = console_height(&tall, false);
        assert!(tall.network.scope);
        let short = console(Some(&s), &Inputs { available: Some(full - 1), ..inputs(&h, &p) });
        assert!(!short.network.scope);
        assert_eq!(full - console_height(&short, false), SCOPE_BAND_PX);
        let unknown = console(Some(&s), &Inputs { available: None, ..inputs(&h, &p) });
        assert!(unknown.network.scope);
    }

    #[test]
    fn degrade_takes_the_link_down_but_keeps_the_counters() {
        let mut s = status(true, Some(4), 0);
        degrade(&mut s);
        assert!(!s.tunnel.connected && s.tunnel.handshake_age_s.is_none() && s.tunnel.up_for_s.is_none());
        assert_eq!(s.tunnel.rx_bytes, 693_600_000);
    }
}
