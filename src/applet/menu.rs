//! The popover's overflow menu, as data (redesign spec §2): Layouts &
//! characters…, Preferences…, Exit Yutani. The first two need the daemon,
//! so with the service stopped they are disabled; Exit always works — it
//! is also how the applet itself goes away. (Stopping only the daemon is
//! the service rocker's job.)

use crate::applet::Action;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: &'static str,
    /// `None` renders the row disabled.
    pub action: Option<Action>,
    /// Exit: destructive text.
    pub danger: bool,
}

pub fn rows(running: bool) -> Vec<MenuRow> {
    let on = |a: Action| running.then_some(a);
    vec![
        MenuRow { label: "Layouts & characters…", action: on(Action::LayoutsAndCharacters), danger: false },
        MenuRow { label: "Preferences…", action: on(Action::Preferences), danger: false },
        MenuRow { label: "Exit Yutani", action: Some(Action::Exit), danger: true },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_is_layouts_preferences_exit_in_that_order() {
        let rows = rows(true);
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), ["Layouts & characters…", "Preferences…", "Exit Yutani"]);
        assert_eq!(rows[0].action, Some(Action::LayoutsAndCharacters));
        assert_eq!(rows[1].action, Some(Action::Preferences));
        assert_eq!(rows[2].action, Some(Action::Exit));
        assert!(rows[2].danger && !rows[0].danger && !rows[1].danger);
        assert!(rows.iter().all(|r| r.action.is_some()));
    }

    /// The settings rows need the daemon; they stay, dimmed, so the
    /// popover keeps its shape. Exit still closes the applet.
    #[test]
    fn a_stopped_service_disables_all_but_exit() {
        let rows = rows(false);
        assert_eq!(rows.len(), 3);
        assert!(rows[..2].iter().all(|r| r.action.is_none()));
        assert_eq!(rows[2].action, Some(Action::Exit));
    }
}
