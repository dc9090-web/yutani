//! The popover's overflow menu, as data (redesign spec §2): Layouts &
//! characters…, Preferences…, Quit. Every row needs the daemon, so with
//! the service stopped they are all disabled.

use crate::applet::Action;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: &'static str,
    /// Trailing mono hint, right-aligned.
    pub hint: Option<String>,
    /// `None` renders the row disabled.
    pub action: Option<Action>,
    /// Quit: destructive text.
    pub danger: bool,
}

impl MenuRow {
    pub fn disabled(&self) -> bool {
        self.action.is_none()
    }
}

pub fn rows(running: bool) -> Vec<MenuRow> {
    let on = |a: Action| running.then_some(a);
    vec![
        MenuRow { label: "Layouts & characters…", hint: None, action: on(Action::LayoutsAndCharacters), danger: false },
        MenuRow { label: "Preferences…", hint: None, action: on(Action::Preferences), danger: false },
        MenuRow { label: "Quit", hint: None, action: on(Action::Quit), danger: true },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_is_layouts_preferences_quit_in_that_order() {
        let rows = rows(true);
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), ["Layouts & characters…", "Preferences…", "Quit"]);
        assert_eq!(rows[0].action, Some(Action::LayoutsAndCharacters));
        assert_eq!(rows[1].action, Some(Action::Preferences));
        assert_eq!(rows[2].action, Some(Action::Quit));
        assert!(rows[2].danger && !rows[0].danger && !rows[1].danger);
        assert!(rows.iter().all(|r| !r.disabled()));
    }

    /// Nothing in the menu works without the daemon; the rows stay, dimmed,
    /// so the popover keeps its shape.
    #[test]
    fn a_stopped_service_disables_every_row() {
        let rows = rows(false);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.disabled()));
    }
}
