//! The applet's pure model. Everything the panel applet decides — what to
//! poll, what to show, which icon, which menu rows, what an action sends —
//! lives here and is unit-tested; `src/bin/yutani-applet/` only renders it.
//! See docs/superpowers/specs/2026-09-12-yutani-applet-design.md.

pub mod client;
pub mod display;
pub mod format;
pub mod icon;
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

/// A menu press. `request()` is `None` for the two actions the applet
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
    /// Open `~/.config/yutani/config.ron` with `xdg-open`.
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
            Action::Preferences | Action::StartDaemon => None,
        }
    }
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

/// Whether the poll timer should issue a `status` request. A daemon slower
/// than the 1 s popup cadence would otherwise get a growing queue of them.
pub fn should_poll(request_in_flight: bool) -> bool {
    !request_in_flight
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
        assert_eq!(Action::Preferences.request(), None);
        assert_eq!(Action::StartDaemon.request(), None);
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
        assert!(exe.is_absolute() && exe.is_file() || exe == std::path::PathBuf::from("yutani"));
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

    #[test]
    fn a_poll_is_skipped_while_one_is_outstanding() {
        assert!(should_poll(false));
        assert!(!should_poll(true));
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
