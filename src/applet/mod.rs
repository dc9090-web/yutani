//! The applet's pure model. Everything the panel applet decides — what to
//! poll, what to show, which icon, which menu rows, what an action sends —
//! lives here and is unit-tested; `src/bin/yutani-applet/` only renders it.
//! See docs/superpowers/specs/2026-09-12-yutani-applet-design.md.

pub mod client;
pub mod display;
pub mod format;
pub mod icon;
pub mod install;
pub mod menu;
pub mod rate;
pub mod theme;

use std::time::{Duration, Instant};

/// The longest the icon shows the sync state after a `tunnel
/// connect|disconnect` (spec §3). It is the *upper bound* on the wait, not
/// the wait: [`still_pending`] ends it the moment the daemon reports the
/// state that was asked for.
pub const PENDING_S: u64 = 10;

/// How long an `err …` note stays under the menu (spec §7).
pub const NOTE_MS: u64 = 3_000;

/// A menu press. `request()` is `None` for the one action the applet
/// performs itself instead of asking the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Connect,
    Disconnect,
    ShowThumbs,
    HideThumbs,
    /// 1-based index in the daemon's layout order — the order `status`
    /// lists clients in.
    Focus(usize),
    Quit,
    /// Ask the daemon to open its settings window (spec §6).
    Preferences,
    /// Spawn the daemon (offline state only).
    StartDaemon,
}

impl Action {
    pub fn request(&self) -> Option<crate::ipc::Request> {
        use crate::ipc::Request;
        match self {
            Action::Connect => Some(Request::TunnelConnect),
            Action::Disconnect => Some(Request::TunnelDisconnect),
            Action::ShowThumbs => Some(Request::Show),
            Action::HideThumbs => Some(Request::Hide),
            Action::Focus(n) => Some(Request::Focus(*n)),
            Action::Quit => Some(Request::Quit),
            Action::Preferences => Some(Request::Settings),
            Action::StartDaemon => None,
        }
    }
}

/// What the menu says under a Connect/Disconnect row while the daemon is
/// still working on it — up to [`client::TUNNEL_TIMEOUT`], most of which
/// would otherwise be a row that looks ignored. `None` for every other
/// action: they are answered at once.
pub fn waiting_note(action: Action) -> Option<String> {
    let verb = match action {
        Action::Connect => "connecting",
        Action::Disconnect => "disconnecting",
        _ => return None,
    };
    Some(format!("{verb}… (up to {} s)", client::TUNNEL_TIMEOUT.as_secs()))
}

/// 1 s with the popup open, 5 s with it closed (spec §2). The applet's
/// timer subscription is keyed on this duration, so flipping it restarts
/// the timer — which is exactly the intent.
pub fn poll_interval(popup_open: bool) -> Duration {
    Duration::from_secs(if popup_open { 1 } else { 5 })
}

/// The daemon binary: the `yutani` next to this applet if it is there
/// (a cargo target dir, a prefix bin dir), else whatever `yutani` `PATH`
/// finds.
pub fn daemon_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("yutani")))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("yutani"))
}

/// What "Start Yutani" runs: `yutani start`, the same as the launcher
/// entry — through the systemd user unit when `yutani service install`
/// has been run (crash restart, journald, stoppable via the unit), else
/// the daemon in that process. Bare `yutani` would bypass an installed
/// unit and leave a daemon it cannot see.
pub fn start_command() -> std::process::Command {
    let mut cmd = std::process::Command::new(daemon_exe());
    cmd.arg("start");
    cmd
}

/// A pending connect/disconnect has got what it asked for: the tunnel a
/// Connect wanted up is up, or the one a Disconnect wanted down is down.
pub fn pending_done(want_connected: bool, observed_connected: bool) -> bool {
    want_connected == observed_connected
}

/// Whether a connect/disconnect armed with deadline `until` is still
/// settling at `now`. The observed state ends it early (`satisfied`); the
/// deadline only stops the sync icon spinning forever when the daemon never
/// gets there.
pub fn still_pending(until: Instant, now: Instant, satisfied: bool) -> bool {
    !satisfied && now < until
}

/// The applet's `status` poll guard.
///
/// At most one `status` request is ever outstanding: a daemon slower than
/// the 1 s popup cadence would otherwise collect a growing queue of them,
/// and two in flight can land out of order and leave the popup showing the
/// older reply.
///
/// The two ways to ask for a poll differ in what happens when the socket is
/// busy, and that is the whole point of the type:
///
/// - [`Poll::tick`] — the timer. It *skips*: the next tick is only 1–5 s
///   away and nothing is waiting on this one.
/// - [`Poll::request`] — an action's follow-up. It *defers*: this poll is
///   the only thing that will show the action's result before the next
///   tick, so it is remembered and issued by [`Poll::replied`] the moment
///   the outstanding one comes back. One flag, not a queue — five presses
///   in a row still cost exactly one follow-up poll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Poll {
    in_flight: bool,
    deferred: bool,
}

impl Poll {
    /// Ask for a poll. `true` means "send it now"; `false` means one was
    /// already outstanding and this one has been remembered.
    pub fn request(&mut self) -> bool {
        if self.in_flight {
            self.deferred = true;
            return false;
        }
        self.in_flight = true;
        true
    }

    /// The timer's form of [`Poll::request`]: skip instead of deferring.
    pub fn tick(&mut self) -> bool {
        !self.in_flight && self.request()
    }

    /// A reply landed. `true` means a deferred request is going out now.
    pub fn replied(&mut self) -> bool {
        self.in_flight = false;
        std::mem::take(&mut self.deferred) && self.request()
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }
}

/// An error note is shown for [`NOTE_MS`] after it was set.
pub fn note_visible(set_at_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(set_at_ms) < NOTE_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Request;

    #[test]
    fn actions_map_onto_the_ipc_protocol() {
        assert_eq!(Action::Connect.request(), Some(Request::TunnelConnect));
        assert_eq!(Action::Disconnect.request(), Some(Request::TunnelDisconnect));
        assert_eq!(Action::ShowThumbs.request(), Some(Request::Show));
        assert_eq!(Action::HideThumbs.request(), Some(Request::Hide));
        assert_eq!(Action::Focus(3).request(), Some(Request::Focus(3)));
        assert_eq!(Action::Quit.request(), Some(Request::Quit));
        assert_eq!(Action::Preferences.request(), Some(crate::ipc::Request::Settings));
        assert_eq!(Action::StartDaemon.request(), None);
    }

    /// I2: only the two tunnel actions keep the user waiting.
    #[test]
    fn only_the_tunnel_actions_have_something_to_say_while_waiting() {
        assert_eq!(waiting_note(Action::Connect).as_deref(), Some("connecting… (up to 15 s)"));
        assert_eq!(waiting_note(Action::Disconnect).as_deref(), Some("disconnecting… (up to 15 s)"));
        for action in [Action::ShowThumbs, Action::HideThumbs, Action::Focus(1), Action::Quit, Action::Preferences, Action::StartDaemon] {
            assert_eq!(waiting_note(action), None, "{action:?}");
        }
    }

    #[test]
    fn poll_is_one_second_open_and_five_closed() {
        assert_eq!(poll_interval(true), Duration::from_secs(1));
        assert_eq!(poll_interval(false), Duration::from_secs(5));
    }

    #[test]
    fn the_daemon_is_a_sibling_binary_or_just_a_name() {
        let exe = daemon_exe();
        assert_eq!(exe.file_name().unwrap(), "yutani");
        // Either an absolute sibling that exists, or the bare name for PATH.
        assert!(exe.is_absolute() && exe.is_file() || exe == std::path::Path::new("yutani"));
    }

    /// I1: "Start Yutani" runs `yutani start`, not bare `yutani` — that is
    /// what routes through the systemd user unit when one is installed
    /// (crash restart, journald, stoppable via the unit) and is byte-for-
    /// byte the in-process daemon when none is.
    #[test]
    fn start_yutani_goes_through_yutani_start() {
        let cmd = start_command();
        assert_eq!(std::path::Path::new(cmd.get_program()).file_name().unwrap(), "yutani");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["start"]);
    }

    #[test]
    fn a_pending_action_ends_when_the_daemon_agrees() {
        // Connect is satisfied by a link that is up, Disconnect by one down.
        assert!(pending_done(true, true));
        assert!(pending_done(false, false));
        assert!(!pending_done(true, false));
        assert!(!pending_done(false, true));
    }

    #[test]
    fn the_deadline_is_only_the_upper_bound_on_waiting() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(PENDING_S);
        // Not there yet, and time to spare: still settling.
        assert!(still_pending(deadline, now, false));
        // The daemon caught up early — done, well inside the deadline.
        assert!(!still_pending(deadline, now, true));
        // The daemon never caught up — the deadline gives up for us.
        assert!(!still_pending(now, now + Duration::from_secs(1), false));
        assert!(!still_pending(now, now, false));
    }

    /// The timer skips rather than queues: another tick is 1–5 s away and a
    /// second concurrent `status` would only race the first.
    #[test]
    fn a_timer_poll_is_skipped_while_one_is_outstanding() {
        let mut poll = Poll::default();
        assert!(poll.tick(), "nothing outstanding: it goes out");
        assert!(poll.in_flight());
        assert!(!poll.tick(), "one is already outstanding");
        assert!(!poll.replied(), "a skipped tick is not remembered");
        assert!(!poll.in_flight());
    }

    /// M5: an action's follow-up poll is the only thing that will show its
    /// result before the next tick, so it may not be dropped just because
    /// the timer's own poll happens to be in flight — it waits its turn.
    #[test]
    fn a_poll_wanted_while_one_is_outstanding_is_issued_when_that_one_lands() {
        let mut poll = Poll::default();
        assert!(poll.request(), "the first request goes out at once");
        assert!(!poll.request(), "the second is deferred, not sent");
        assert!(poll.replied(), "and goes out when the first comes back");
        assert!(poll.in_flight(), "which makes it the outstanding one");
        assert!(!poll.replied(), "nothing is left behind it");
        assert!(!poll.in_flight());
    }

    /// However many presses land while one poll is out, exactly one poll
    /// follows it — the guard is a flag, not a queue.
    #[test]
    fn many_deferred_polls_collapse_into_one() {
        let mut poll = Poll::default();
        assert!(poll.request());
        for _ in 0..5 {
            assert!(!poll.request());
        }
        assert!(poll.replied());
        assert!(!poll.replied());
    }

    /// A reply to a request that was never made (a late duplicate) must not
    /// leave the guard thinking one is still in flight.
    #[test]
    fn a_reply_with_nothing_outstanding_changes_nothing() {
        let mut poll = Poll::default();
        assert!(!poll.replied());
        assert!(!poll.in_flight());
        assert!(poll.request(), "and the guard is still usable");
    }

    #[test]
    fn a_note_lives_for_three_seconds() {
        assert!(note_visible(1_000, 1_000));
        assert!(note_visible(1_000, 3_999));
        assert!(!note_visible(1_000, 4_000));
        assert!(!note_visible(1_000, 9_999));
        // A clock that went backwards must not make the note immortal.
        assert!(note_visible(5_000, 1_000));
    }
}
