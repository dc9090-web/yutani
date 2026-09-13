//! Pure decision rules for the UI, kept free of iced/Wayland types so they
//! can be unit-tested.

use std::time::Duration;

use crate::model::config::{Mode, Visibility};
use crate::model::layout::ThumbPos;

/// How long EVE still counts as focused after its last client lost focus.
/// Clicking from one EVE window to another passes through a moment where
/// no client is activated; without a grace every such click destroyed
/// every thumbnail surface and recreated it milliseconds later. Besides
/// the flicker, that destroy/recreate burst is what preceded each of the
/// daemon's crashes (a use-after-free in libcosmic's surface teardown), so
/// the grace is both polish and a mitigation.
pub const FOCUS_GRACE: Duration = Duration::from_millis(300);

/// Whether EVE counts as focused: a client is activated now, or one was
/// less than `grace` ago.
pub fn eve_focused(any_activated: bool, since_last_activation: Option<Duration>, grace: Duration) -> bool {
    any_activated || since_last_activation.is_some_and(|since| since < grace)
}

/// What the focus grace needs after a client update, given whether any
/// client is activated now, whether one was before the update, and
/// whether EVE still counts as focused (`eve_focused`): (stamp the last
/// focus now?, schedule a grace timer?). Focus *leaving* EVE is the moment
/// the grace starts — the last stamp dates from the last event that
/// arrived while EVE was focused, which after a quiet stretch of play is
/// minutes old — so that transition stamps and schedules; while focus is
/// still away inside the grace only the timer is (re)scheduled, so the
/// grace can never extend itself. Visibility is deliberately not an input:
/// one timer per focus loss costs nothing, and a switch to
/// `EveFocusedOnly` inside the grace needs that second look too.
pub fn grace_after_update(any_activated: bool, was_activated: bool, within_grace: bool) -> (bool, bool) {
    if any_activated {
        return (true, false);
    }
    if was_activated {
        return (true, true);
    }
    (false, within_grace)
}

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

/// What `focus_order` needs to know about one client.
pub struct FocusItem<H> {
    pub handle: H,
    pub label: String,
    /// Connector name of the output the thumbnail is on.
    pub output: String,
    /// Thumbnail top-left, logical px (floating mode).
    pub position: (i32, i32),
}

/// Rank used for dock placement, `focus <n>` and the `order` list saved in
/// `current.ron` (spec §7 and §9): a character the layout's `order` names
/// ranks by its place in that list; anyone else follows, by label,
/// case-insensitively. One comparator, so the dock arrangement and the
/// focus order can never disagree.
pub fn dock_rank(order: &[String], label: &str) -> (usize, String) {
    let index = order.iter().position(|n| n.eq_ignore_ascii_case(label)).unwrap_or(order.len());
    (index, label.to_lowercase())
}

/// Layout order used by `focus <n>`, `next`, `prev` and the dock (spec
/// §7): dock mode by `dock_rank`; floating by (output, y, x). Stable for
/// ties, so equal ranks keep their existing relative order.
pub fn focus_order<H>(mode: Mode, order: &[String], mut items: Vec<FocusItem<H>>) -> Vec<H> {
    match mode {
        Mode::Dock => items.sort_by_key(|i| dock_rank(order, &i.label)),
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
    fn eve_stays_focused_for_the_grace_after_its_last_client_loses_focus() {
        let g = Duration::from_millis(300);
        assert!(eve_focused(true, None, g));
        assert!(!eve_focused(false, None, g), "never focused");
        assert!(eve_focused(false, Some(Duration::from_millis(100)), g), "inside the grace");
        assert!(!eve_focused(false, Some(g), g), "the grace is exclusive");
        assert!(!eve_focused(false, Some(Duration::from_secs(5)), g));
    }

    /// [I1]/[M1] Focus leaving EVE is what starts the grace: the stamp is
    /// refreshed on that transition (the last one may be minutes old) and
    /// the timer scheduled — whatever the visibility setting, so a switch
    /// to `EveFocusedOnly` inside the grace still gets its second look.
    #[test]
    fn the_grace_starts_when_focus_leaves_eve() {
        // Focused now: stamp, no timer.
        assert_eq!(grace_after_update(true, true, true), (true, false));
        assert_eq!(grace_after_update(true, false, false), (true, false));
        // Focus just left: stamp now and arrange the second look.
        assert_eq!(grace_after_update(false, true, false), (true, true), "stale stamp must not matter");
        assert_eq!(grace_after_update(false, true, true), (true, true));
        // Still away inside the grace: another look, but no fresh stamp
        // (the grace must not extend itself).
        assert_eq!(grace_after_update(false, false, true), (false, true));
        // Long gone.
        assert_eq!(grace_after_update(false, false, false), (false, false));
    }

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
    fn dock_rank_matches_by_character_name_case_insensitively() {
        let order = ["Aria Vex".to_string()];
        assert_eq!(dock_rank(&order, "aria vex"), (0, "aria vex".to_string()));
        assert_eq!(dock_rank(&order, "Kel"), (1, "kel".to_string()));
        assert_eq!(dock_rank(&[], "Kel"), (0, "kel".to_string()));
    }

    #[test]
    fn dock_order_is_by_label_when_the_layout_records_no_order() {
        let items = vec![item(1, "kel", "DP-1", 0, 0), item(2, "Aria", "DP-1", 0, 0), item(3, "Kel", "DP-2", 0, 0)];
        assert_eq!(focus_order(Mode::Dock, &[], items), vec![2, 1, 3]);
    }

    #[test]
    fn a_recorded_order_comes_first_and_the_rest_follow_by_label() {
        let order = ["Kel".to_string(), "Zoe".to_string()];
        let items = vec![
            item(1, "Aria", "DP-1", 0, 0),
            item(2, "Zoe", "DP-1", 0, 0),
            item(3, "Kel", "DP-1", 0, 0),
            item(4, "bob", "DP-1", 0, 0),
        ];
        assert_eq!(focus_order(Mode::Dock, &order, items), vec![3, 2, 1, 4]);
    }

    #[test]
    fn floating_focus_order_is_output_then_row_then_column_and_ignores_the_recorded_order() {
        let order = ["z".to_string()];
        let items = vec![
            item(1, "z", "DP-2", 10, 10),
            item(2, "y", "DP-1", 500, 40),
            item(3, "x", "DP-1", 40, 40),
            item(4, "w", "DP-1", 40, 400),
        ];
        assert_eq!(focus_order(Mode::Floating, &order, items), vec![3, 2, 4, 1]);
    }

    #[test]
    fn a_name_the_order_lists_but_nobody_is_playing_does_not_disturb_the_rest() {
        // A logged-out character keeps its slot in `order`; the live ones
        // still sort among themselves in that same relative order.
        let order = ["Zoe".to_string(), "Kel".to_string(), "Aria".to_string()];
        let items = vec![item(1, "Aria", "DP-1", 0, 0), item(2, "Kel", "DP-1", 0, 0)];
        assert_eq!(focus_order(Mode::Dock, &order, items), vec![2, 1]);
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
