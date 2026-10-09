//! The applet's pure model. Everything the panel applet decides — what to
//! poll, what to show, which icon, which menu rows, what an action sends —
//! lives here and is unit-tested; `src/bin/yutani-applet/` only renders it.
//! See docs/superpowers/specs/2026-09-12-yutani-applet-design.md.

pub mod client;
pub mod console;
pub mod fonts;
pub mod format;
pub mod host;
pub mod icon;
pub mod install;
pub mod menu;
pub mod rocker;
pub mod skin;
pub mod theme;

use std::time::{Duration, Instant};

/// The longest a start or quit of the service shows as settling. It is the
/// *upper bound* on the wait, not the wait: the poll ends it the moment the
/// daemon reports the state that was asked for.
pub const PENDING_S: u64 = 10;

/// How long an `err …` note stays under the menu (spec §7).
pub const NOTE_MS: u64 = 3_000;

/// The most of an `err …` a note shows. `malformed reply {line:?}` quotes
/// the whole line (up to 64 KiB) and an action error carries systemctl's
/// stderr; unclipped, either wraps into many lines in the 360 px popup and
/// pushes Quit toward `popup_container`'s 1000 px clip.
pub const NOTE_MAX_CHARS: usize = 160;

/// `text` as a note: whole if it fits [`NOTE_MAX_CHARS`], else its first
/// `NOTE_MAX_CHARS` chars and an ellipsis.
pub fn clip_note(text: &str) -> String {
    match text.char_indices().nth(NOTE_MAX_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// A menu press. `request()` is `None` for the one action the applet
/// performs itself instead of asking the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// 1-based index in the daemon's layout order — the order `status`
    /// lists clients in.
    Focus(usize),
    Quit,
    /// Ask the daemon to open its settings window (spec §6).
    Preferences,
    /// The settings window on its Layouts page (redesign spec §2).
    LayoutsAndCharacters,
    /// Spawn the daemon (offline state only).
    StartDaemon,
    /// Launch EVE through Steam (Phase 3).
    Launch,
}

impl Action {
    pub fn request(&self) -> Option<crate::ipc::Request> {
        use crate::ipc::Request;
        match self {
            Action::Focus(n) => Some(Request::Focus(*n)),
            Action::Quit => Some(Request::Quit),
            Action::Preferences => Some(Request::Settings),
            Action::LayoutsAndCharacters => Some(Request::SettingsPage("layouts".into())),
            Action::StartDaemon => None,
            Action::Launch => Some(crate::ipc::Request::Launch),
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

/// Whether a start or quit armed with deadline `until` is still settling
/// at `now`. The observed state ends it early (`satisfied`); the deadline
/// only stops it waiting forever when the daemon never gets there.
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
        assert_eq!(Action::Focus(3).request(), Some(Request::Focus(3)));
        assert_eq!(Action::Quit.request(), Some(Request::Quit));
        assert_eq!(Action::Preferences.request(), Some(crate::ipc::Request::Settings));
        assert_eq!(Action::LayoutsAndCharacters.request(), Some(crate::ipc::Request::SettingsPage("layouts".into())));
        assert_eq!(Action::StartDaemon.request(), None);
        assert_eq!(Action::Launch.request(), Some(Request::Launch));
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

    /// M2: a note is one aside under a row, not a paragraph. `malformed
    /// reply {line:?}` quotes up to 64 KiB and an action error carries
    /// systemctl's whole stderr; either would wrap into many lines and push
    /// Quit toward `popup_container`'s clip.
    #[test]
    fn a_note_is_clipped_to_one_readable_line() {
        assert_eq!(clip_note("short"), "short");
        let exact = "a".repeat(NOTE_MAX_CHARS);
        assert_eq!(clip_note(&exact), exact, "at the limit is kept whole");
        // Clipped on a char boundary, with an ellipsis that says so.
        let clipped = clip_note(&"é".repeat(NOTE_MAX_CHARS + 40));
        assert_eq!(clipped.chars().count(), NOTE_MAX_CHARS + 1);
        assert!(clipped.ends_with('…'), "{clipped}");
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
