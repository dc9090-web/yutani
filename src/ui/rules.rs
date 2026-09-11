//! Pure decision rules for the UI, kept free of iced/Wayland types so they
//! can be unit-tested.

use crate::backend::Handle;
use crate::model::config::Visibility;
use crate::model::layout::ThumbPos;

/// Whether a client's thumbnail should be on screen right now.
pub fn should_show(
    visibility: Visibility,
    hide_active: bool,
    hidden: bool,
    any_activated: bool,
    this_activated: bool,
) -> bool {
    if hidden {
        return false;
    }
    let visible = match visibility {
        Visibility::Always => true,
        Visibility::EveFocusedOnly => any_activated,
    };
    visible && !(hide_active && this_activated)
}

/// Where a (re)created floating thumbnail goes: where it already was, else
/// the character's saved spot, else the next free slot.
pub fn choose_position(
    placed: Option<((i32, i32), bool)>,
    saved: Option<&ThumbPos>,
    next: (i32, i32),
) -> ((i32, i32), bool) {
    if let Some(p) = placed {
        return p;
    }
    if let Some(s) = saved {
        return ((s.x, s.y), s.pinned);
    }
    (next, false)
}

/// Which capture command (if any) brings the backend in line with `show`:
/// `Some(true)` = pause, `Some(false)` = resume, `None` = already there.
/// Keyed on the backend's actual paused state, not on whether a surface
/// happened to exist, so a client born hidden gets paused too.
pub fn capture_transition(show: bool, paused: bool) -> Option<bool /* pause? */> {
    if show == paused { Some(!show) } else { None }
}

/// Dock order: by label, case-insensitive, stable for equal labels.
pub fn dock_order<'a>(labels: impl Iterator<Item = (&'a Handle, &'a str)>) -> Vec<Handle> {
    let mut v: Vec<(&Handle, String)> = labels.map(|(h, l)| (h, l.to_lowercase())).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v.into_iter().map(|(h, _)| h.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_shows_unless_hidden_or_hide_active() {
        assert!(should_show(Visibility::Always, false, false, false, false));
        assert!(should_show(Visibility::Always, false, false, true, true));
        assert!(!should_show(Visibility::Always, true, false, true, true)); // hide_active + this is active
        assert!(should_show(Visibility::Always, true, false, true, false));
        assert!(!should_show(Visibility::Always, false, true, true, false)); // tray hidden
    }

    #[test]
    fn eve_focused_only_needs_some_activated_client() {
        assert!(!should_show(Visibility::EveFocusedOnly, false, false, false, false));
        assert!(should_show(Visibility::EveFocusedOnly, false, false, true, false));
        assert!(!should_show(Visibility::EveFocusedOnly, true, false, true, true));
    }

    #[test]
    fn capture_transition_is_an_edge_on_actual_paused_state() {
        assert_eq!(capture_transition(true, true), Some(false)); // shown but paused: resume
        assert_eq!(capture_transition(false, false), Some(true)); // hidden but running: pause
        assert_eq!(capture_transition(true, false), None); // shown and running
        assert_eq!(capture_transition(false, true), None); // hidden and paused
    }

    #[test]
    fn placement_prefers_placed_then_saved_then_next() {
        let saved = ThumbPos { output: "DP-1".into(), x: 5, y: 6, pinned: true };
        assert_eq!(choose_position(Some(((1, 2), false)), Some(&saved), (9, 9)), ((1, 2), false));
        assert_eq!(choose_position(None, Some(&saved), (9, 9)), ((5, 6), true));
        assert_eq!(choose_position(None, None, (9, 9)), ((9, 9), false));
    }
}
