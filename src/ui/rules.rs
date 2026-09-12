//! Pure decision rules for the UI, kept free of iced/Wayland types so they
//! can be unit-tested.

use crate::backend::Handle;
use crate::model::config::{Mode, Visibility};
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

/// What `focus_order` needs to know about one client.
pub struct FocusItem<H> {
    pub handle: H,
    pub label: String,
    /// Connector name of the output the thumbnail is on.
    pub output: String,
    /// Thumbnail top-left, logical px (floating mode).
    pub position: (i32, i32),
}

/// Layout order used by `focus <n>`, `next` and `prev` (spec §7): dock
/// order by label; floating by (output, y, x). Stable for ties.
pub fn focus_order<H: Clone>(mode: Mode, mut items: Vec<FocusItem<H>>) -> Vec<H> {
    match mode {
        Mode::Dock => items.sort_by_key(|i| i.label.to_lowercase()),
        Mode::Floating => items.sort_by(|a, b| {
            (a.output.as_str(), a.position.1, a.position.0).cmp(&(b.output.as_str(), b.position.1, b.position.0))
        }),
    }
    items.into_iter().map(|i| i.handle).collect()
}

/// Next (or previous) handle in `order` after `active`, wrapping. With no
/// active client — or one not in `order` — `next` is the first, `prev` the
/// last.
pub fn step<H: Clone + PartialEq>(order: &[H], active: Option<&H>, forward: bool) -> Option<H> {
    if order.is_empty() {
        return None;
    }
    let idx = active.and_then(|a| order.iter().position(|h| h == a));
    let next = match (idx, forward) {
        (Some(i), true) => (i + 1) % order.len(),
        (Some(i), false) => (i + order.len() - 1) % order.len(),
        (None, true) => 0,
        (None, false) => order.len() - 1,
    };
    Some(order[next].clone())
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
        assert!(!should_show(Visibility::Always, false, true, true, false)); // hidden via IPC
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

    fn item(h: u32, label: &str, output: &str, x: i32, y: i32) -> FocusItem<u32> {
        FocusItem { handle: h, label: label.into(), output: output.into(), position: (x, y) }
    }

    #[test]
    fn dock_focus_order_is_by_label_case_insensitive_and_stable() {
        let items = vec![item(1, "kel", "DP-1", 0, 0), item(2, "Aria", "DP-1", 0, 0), item(3, "Kel", "DP-2", 0, 0)];
        assert_eq!(focus_order(Mode::Dock, items), vec![2, 1, 3]);
    }

    #[test]
    fn floating_focus_order_is_output_then_row_then_column() {
        let items = vec![
            item(1, "z", "DP-2", 10, 10),
            item(2, "y", "DP-1", 500, 40),
            item(3, "x", "DP-1", 40, 40),
            item(4, "w", "DP-1", 40, 400),
        ];
        assert_eq!(focus_order(Mode::Floating, items), vec![3, 2, 4, 1]);
    }

    #[test]
    fn step_wraps_and_handles_no_active() {
        let order = [10, 20, 30];
        assert_eq!(step(&order, Some(&20), true), Some(30));
        assert_eq!(step(&order, Some(&30), true), Some(10));
        assert_eq!(step(&order, Some(&10), false), Some(30));
        assert_eq!(step(&order, None, true), Some(10));
        assert_eq!(step(&order, None, false), Some(30));
        // Active client unknown to the order (e.g. mid-update): treat as none.
        assert_eq!(step(&order, Some(&99), true), Some(10));
        assert_eq!(step::<u32>(&[], None, true), None);
    }
}
