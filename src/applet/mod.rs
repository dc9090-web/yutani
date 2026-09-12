//! The applet's pure model. Everything the panel applet decides — what to
//! poll, what to show, which icon, which menu rows, what an action sends —
//! lives here and is unit-tested; `src/bin/yutani-applet/` only renders it.
//! See docs/superpowers/specs/2026-09-12-yutani-applet-design.md.

pub mod client;
pub mod format;
pub mod icon;
pub mod rate;
pub mod theme;

use std::time::Duration;

/// How long after a `tunnel connect|disconnect` the icon shows the sync
/// state even if the daemon has not caught up yet (spec §3).
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
    fn a_note_lives_for_three_seconds() {
        assert!(note_visible(1_000, 1_000));
        assert!(note_visible(1_000, 3_999));
        assert!(!note_visible(1_000, 4_000));
        assert!(!note_visible(1_000, 9_999));
        // A clock that went backwards must not make the note immortal.
        assert!(note_visible(5_000, 1_000));
    }
}
