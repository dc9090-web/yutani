//! Everything the Nostromo popover shows (handoff v6 "Screen"), as
//! finished strings and small enums. The view renders a `Console` and
//! decides nothing; the rules live here, where they are tested.

use crate::applet::Action;
use crate::applet::format;
use crate::applet::host::HostReading;
use crate::applet::skin::Ink;
pub use crate::launch_eve::Step;
use crate::status::Status;

pub const DASH: &str = "—";

pub struct Inputs<'a> {
    pub host: &'a HostReading,
    /// A start or quit of the service is still settling.
    pub service_pending: bool,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Console {
    pub running: bool,
    pub state_word: &'static str,
    pub service: ControlRow,
    pub accounts_label: &'static str,
    /// `02`, only when running with more than one client.
    pub count: Option<String>,
    pub accounts: Accounts,
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

fn service_row(running: bool, pending: bool) -> ControlRow {
    let rocker = if pending { RockerState::Pending } else if running { RockerState::On } else { RockerState::Off };
    if running {
        ControlRow {
            title: "YUTANI SERVICE",
            sub: "MULTIBOX · HOTKEYS ACTIVE".into(),
            sub_ink: Ink::Dim,
            led: Ink::Phosphor,
            glow: true,
            rocker,
            press: Some(Action::Quit),
        }
    } else {
        ControlRow {
            title: "YUTANI SERVICE",
            sub: "STOPPED · MULTIBOX, HOTKEYS OFF".into(),
            sub_ink: Ink::Dimmer,
            led: Ink::Dimmer,
            glow: false,
            rocker,
            press: Some(Action::StartDaemon),
        }
    }
}

/// The launch failure line fits the log card on one line: at most this
/// many characters, the last an `…` when cut.
pub const FAILED_MAX_CHARS: usize = 40;

fn clip_failed(why: &str) -> String {
    if why.chars().count() <= FAILED_MAX_CHARS {
        return why.to_string();
    }
    let mut s: String = why.chars().take(FAILED_MAX_CHARS - 1).collect();
    s.push('…');
    s
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
    let ram_total = i.host.ram_total.map_or_else(|| DASH.to_string(), format::gb_ceil);
    let notice = status.and_then(|s| steam_notice(&s.steam));
    Console {
        running,
        state_word: if running { "RUNNING" } else { "STOPPED" },
        service: service_row(running, i.service_pending),
        accounts_label,
        count,
        accounts,
        host_meta: format!("{ram_total} · 1 HZ"),
        host: host_gauges(i.host),
        notice,
        launch: launch_button(status),
        launch_log: status.and_then(|s| s.launch.as_ref()).map(launch_log),
        launch_failed: status.and_then(|s| s.launch.as_ref()).and_then(|l| l.failed.as_deref()).map(clip_failed),
        footer: format!("YUTANI OS · BUILD {}", env!("YUTANI_BUILD")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::ClientStatus;

    fn status(clients: usize) -> Status {
        Status {
            clients: (0..clients).map(|i| ClientStatus { name: format!("Pilot {i}"), active: i == 0 }).collect(),
            hidden: false,
            shortcuts: None,
            steam: Vec::new(),
            launch: None,
        }
    }

    fn inputs(host: &HostReading) -> Inputs<'_> {
        Inputs { host, service_pending: false }
    }

    fn launching(steps: [Step; 4], failed: Option<&str>) -> Status {
        let mut s = status(2);
        s.launch = Some(crate::launch_eve::LaunchState { steps, hotkey: "CTRL+ALT+3".into(), failed: failed.map(Into::into) });
        s
    }

    #[test]
    fn the_button_follows_the_service_the_slots_and_the_launch() {
        let h = HostReading::default();
        let i = inputs(&h);
        assert_eq!(console(None, &i).launch, LaunchButton::Inert("START YUTANI TO LAUNCH"));
        assert_eq!(console(Some(&status(2)), &i).launch, LaunchButton::Ready);
        assert_eq!(console(Some(&status(9)), &i).launch, LaunchButton::Inert("ALL 9 SLOTS IN USE"));
        use Step::*;
        let c = console(Some(&launching([Done, Done, Running, Pending], None)), &i);
        assert_eq!(c.launch, LaunchButton::Launching { step: 3 });
        assert_eq!(console(Some(&launching([Done; 4], None)), &i).launch, LaunchButton::Ready);
        assert_eq!(console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &i).launch, LaunchButton::Ready);
    }

    #[test]
    fn the_log_names_each_step_and_the_hotkey() {
        let h = HostReading::default();
        use Step::*;
        let c = console(Some(&launching([Done, Done, Running, Pending], None)), &inputs(&h));
        let log = c.launch_log.expect("log while launching");
        let texts: Vec<&str> = log.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["STEAM · APPLAUNCH 8500", "STEAM · WINDOW MINIMISED", "EVE CLIENT · STARTING", "HOTKEY ASSIGNED · CTRL+ALT+3"]);
        assert_eq!(log[2].status, Running);
        assert!(console(Some(&status(2)), &inputs(&h)).launch_log.is_none());
        let f = console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &inputs(&h));
        assert_eq!(f.launch_failed.as_deref(), Some("LAUNCHER CLOSED"));
    }

    #[test]
    fn a_long_failure_reason_is_clipped_to_one_line() {
        let h = HostReading::default();
        use Step::*;
        let long = "CANNOT RUN STEAM: PERMISSION DENIED (OS ERROR 13) WHILE STARTING";
        let c = console(Some(&launching([Failed, Pending, Pending, Pending], Some(long))), &inputs(&h));
        let shown = c.launch_failed.unwrap();
        assert_eq!(shown.chars().count(), FAILED_MAX_CHARS);
        assert!(shown.ends_with('…'));
        assert!(long.starts_with(shown.trim_end_matches('…')));
        let short = console(Some(&launching([Failed, Pending, Pending, Pending], Some("LAUNCHER CLOSED"))), &inputs(&h));
        assert_eq!(short.launch_failed.as_deref(), Some("LAUNCHER CLOSED"), "short reasons are untouched");
    }

    #[test]
    fn running_fills_every_section() {
        let h = HostReading::default();
        let c = console(Some(&status(2)), &inputs(&h));
        assert!(c.running);
        assert_eq!(c.state_word, "RUNNING");
        assert_eq!(c.service.sub, "MULTIBOX · HOTKEYS ACTIVE");
        assert_eq!(c.service.rocker, RockerState::On);
        assert_eq!(c.service.press, Some(Action::Quit));
        assert_eq!((c.accounts_label, c.count.as_deref()), ("ACCOUNTS", Some("02")));
        let Accounts::Rows(rows) = &c.accounts else { panic!() };
        assert_eq!(rows[0], AccountRow { index: 1, name: "PILOT 0".into(), focused: true });
        assert_eq!(c.launch, LaunchButton::Ready);
        assert!(c.footer.starts_with("YUTANI OS · BUILD "));
    }

    #[test]
    fn a_stopped_service_reads_stopped_everywhere() {
        let h = HostReading::default();
        let c = console(None, &inputs(&h));
        assert_eq!((c.running, c.state_word), (false, "STOPPED"));
        assert_eq!(c.service.sub, "STOPPED · MULTIBOX, HOTKEYS OFF");
        assert_eq!((c.service.rocker, c.service.press), (RockerState::Off, Some(Action::StartDaemon)));
        assert_eq!(c.accounts, Accounts::Stopped);
        assert_eq!(c.count, None);
    }

    #[test]
    fn accounts_label_count_and_empty_state() {
        let h = HostReading::default();
        let one = console(Some(&status(1)), &inputs(&h));
        assert_eq!((one.accounts_label, one.count.clone()), ("ACCOUNT", None));
        let none = console(Some(&status(0)), &inputs(&h));
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
        let c = console(None, &inputs(&h));
        assert_eq!((c.host[0].value.as_str(), c.host[0].detail.as_str()), ("23%", "46°C"));
        assert_eq!((c.host[1].value.as_str(), c.host[1].detail.as_str()), ("40%", "—"));
        assert_eq!((c.host[2].value.as_str(), c.host[2].detail.as_str()), ("33%", "10.2 GB"));
        assert_eq!(c.host_meta, "32 GB · 1 HZ");
    }

    #[test]
    fn the_steam_notice_is_shouted_with_the_settings_pointer() {
        use crate::steam::{Finding, Verdict};
        let h = HostReading::default();
        let mut s = status(0);
        s.steam = vec![Finding { verdict: Verdict::NoWrapper, file: std::path::PathBuf::from("/home/user/.steam/steam/userdata/1/config/localconfig.vdf") }];
        let c = console(Some(&s), &inputs(&h));
        let n = c.notice.expect("notice");
        assert!(n.ends_with("OPEN SETTINGS → STEAM."), "{n}");
        assert_eq!(n, n.to_uppercase());
    }

    #[test]
    fn a_path_in_the_steam_notice_keeps_its_case() {
        use crate::steam::{Finding, Verdict};
        let h = HostReading::default();
        let mut s = status(0);
        let path = "/home/Daniel/bin/yutani".to_string();
        s.steam = vec![Finding { verdict: Verdict::Broken { path: path.clone() }, file: std::path::PathBuf::from("/x/localconfig.vdf") }];
        let n = console(Some(&s), &inputs(&h)).notice.expect("notice");
        assert!(n.contains(&path), "{n}");
        assert!(n.starts_with("STEAM LAUNCHES EVE THROUGH /home/Daniel/bin/yutani, WHICH IS MISSING."), "{n}");
        assert!(n.ends_with("OPEN SETTINGS → STEAM."), "{n}");
    }
}
