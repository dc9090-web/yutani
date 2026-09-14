//! The popup's menu, as data. Spec §4.4–§4.5 as amended by the
//! 2026-09-14 services-and-characters spec §2: one row per running
//! character (always visible, the focused one dotted), the tunnel row,
//! Preferences…, the thumbnails row (`show`/`hide` over IPC), and Quit —
//! or, with no daemon, a single "Start Yutani".

use crate::applet::Action;
use crate::tunnel::status::Status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Normal,
    /// Quit: `#FF8A7E` text on a `#FF6B5C1F` hover.
    Danger,
    /// A character row; the dot shows focus.
    Account { active: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: String,
    /// Trailing mono hint, e.g. `systemd` or `not installed`.
    pub hint: Option<String>,
    /// Trailing mono value.
    pub trailing: Option<String>,
    /// `None` renders the row disabled.
    pub action: Option<Action>,
    pub kind: RowKind,
}

/// The one row shown in place of the characters when none is running.
pub const NO_CHARACTERS: &str = "No characters running";

impl MenuRow {
    fn new(label: impl Into<String>, action: Option<Action>) -> Self {
        MenuRow { label: label.into(), hint: None, trailing: None, action, kind: RowKind::Normal }
    }

    /// A row with no action does nothing when pressed, so the view must
    /// paint it as disabled rather than merely inert.
    pub fn disabled(&self) -> bool {
        self.action.is_none()
    }
}

/// `status` is `None` in the offline state.
pub fn rows(status: Option<&Status>) -> Vec<MenuRow> {
    let Some(s) = status else {
        return vec![MenuRow::new("Start Yutani", Some(Action::StartDaemon))];
    };
    let mut out = Vec::new();

    // The characters first, always visible: which one is focused is the
    // thing the popup is opened to see, so it is never behind a click.
    if s.clients.is_empty() {
        out.push(MenuRow::new(NO_CHARACTERS, None));
    }
    for (i, client) in s.clients.iter().enumerate() {
        let mut row = MenuRow::new(client.name.clone(), Some(Action::Focus(i + 1)));
        row.kind = RowKind::Account { active: client.active };
        out.push(row);
    }

    // The handoff's hint is "wg-quick"; we drive systemd, so we say so.
    let mut tunnel = if s.tunnel.connected {
        MenuRow::new("Disconnect tunnel", Some(Action::Disconnect))
    } else {
        MenuRow::new("Connect tunnel", Some(Action::Connect))
    };
    if s.tunnel.installed {
        tunnel.hint = Some("systemd".to_string());
    } else {
        tunnel.action = None;
        tunnel.hint = Some("not installed".to_string());
    }
    out.push(tunnel);

    out.push(MenuRow::new("Preferences…", Some(Action::Preferences)));
    out.push(if s.hidden {
        MenuRow::new("Show thumbnails", Some(Action::ShowThumbs))
    } else {
        MenuRow::new("Hide thumbnails", Some(Action::HideThumbs))
    });

    let mut quit = MenuRow::new("Quit", Some(Action::Quit));
    quit.kind = RowKind::Danger;
    out.push(quit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::applet::Action;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    fn status(installed: bool, connected: bool, hidden: bool, names: &[&str]) -> Status {
        Status {
            clients: names
                .iter()
                .enumerate()
                .map(|(i, n)| ClientStatus { name: (*n).to_string(), active: i == 1 })
                .collect(),
            hidden,
            tunnel: TunnelStatus {
                installed,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: None,
                endpoint: None,
                handshake_age_s: None,
                up_for_s: None,
                failed: false,
                exit_address: None,
                rx_bytes: 0,
                tx_bytes: 0,
            },
        }
    }

    fn labels(rows: &[MenuRow]) -> Vec<&str> {
        rows.iter().map(|r| r.label.as_str()).collect()
    }

    #[test]
    fn without_a_daemon_the_only_thing_to_do_is_start_it() {
        let rows = rows(None);
        assert_eq!(labels(&rows), vec!["Start Yutani"]);
        assert_eq!(rows[0].action, Some(Action::StartDaemon));
        assert_eq!(rows[0].kind, RowKind::Normal);
    }

    /// The characters come first and are always there — no header to
    /// expand: `focus <n>` is 1-based over the daemon's layout order, and
    /// the focused one carries the active dot.
    #[test]
    fn every_character_is_a_focus_row_ahead_of_the_actions() {
        let s = status(true, true, false, &["KestrelVance", "TrilliumTWO", "TrilliumTHREE"]);
        let rows = rows(Some(&s));
        assert_eq!(
            labels(&rows),
            vec![
                "KestrelVance",
                "TrilliumTWO",
                "TrilliumTHREE",
                "Disconnect tunnel",
                "Preferences…",
                "Hide thumbnails",
                "Quit",
            ]
        );
        assert_eq!(rows[0].action, Some(Action::Focus(1)));
        assert_eq!(rows[1].action, Some(Action::Focus(2)));
        assert_eq!(rows[2].action, Some(Action::Focus(3)));
        assert_eq!(rows[0].kind, RowKind::Account { active: false });
        assert_eq!(rows[1].kind, RowKind::Account { active: true });
        assert_eq!(rows[2].kind, RowKind::Account { active: false });
        assert!(rows[..3].iter().all(|r| !r.disabled()));
    }

    #[test]
    fn a_connected_tunnel_offers_disconnect_with_the_truthful_hint() {
        let s = status(true, true, false, &["A", "B"]);
        let rows = rows(Some(&s));
        assert_eq!(labels(&rows), vec!["A", "B", "Disconnect tunnel", "Preferences…", "Hide thumbnails", "Quit"]);
        assert_eq!(rows[2].action, Some(Action::Disconnect));
        assert_eq!(rows[2].hint.as_deref(), Some("systemd"));
        assert_eq!(rows[3].action, Some(Action::Preferences));
        assert_eq!(rows[4].action, Some(Action::HideThumbs));
        assert_eq!(rows[5].action, Some(Action::Quit));
        assert_eq!(rows[5].kind, RowKind::Danger);
    }

    #[test]
    fn a_disconnected_tunnel_offers_connect_and_an_uninstalled_one_is_disabled() {
        let s = status(true, false, false, &[]);
        let rows = rows(Some(&s));
        assert_eq!(rows[1].label, "Connect tunnel");
        assert_eq!(rows[1].action, Some(Action::Connect));
        assert_eq!(rows[1].hint.as_deref(), Some("systemd"));

        let s = status(false, false, false, &[]);
        let rows = super::rows(Some(&s));
        assert_eq!(rows[1].label, "Connect tunnel");
        assert_eq!(rows[1].action, None, "an uninstalled tunnel cannot be connected");
        assert_eq!(rows[1].hint.as_deref(), Some("not installed"));
    }

    #[test]
    fn the_thumbnails_row_follows_the_daemons_hidden_flag() {
        let s = status(true, true, true, &["A"]);
        let rows = rows(Some(&s));
        assert_eq!(rows[3].label, "Show thumbnails");
        assert_eq!(rows[3].action, Some(Action::ShowThumbs));
    }

    #[test]
    fn quit_is_always_last_and_the_only_danger_row() {
        for names in [&[][..], &["A", "B"][..]] {
            let s = status(true, true, false, names);
            let rows = rows(Some(&s));
            assert_eq!(rows.last().unwrap().label, "Quit");
            assert_eq!(rows.iter().filter(|r| r.kind == RowKind::Danger).count(), 1);
        }
    }

    /// With nothing running the list must not simply vanish — the popup
    /// says so in one disabled row, in the characters' place.
    #[test]
    fn no_characters_is_said_in_one_disabled_row() {
        let s = status(true, true, false, &[]);
        let rows = rows(Some(&s));
        assert_eq!(labels(&rows), vec![NO_CHARACTERS, "Disconnect tunnel", "Preferences…", "Hide thumbnails", "Quit"]);
        assert!(rows[0].disabled());
        assert_eq!(rows[0].kind, RowKind::Normal);
        assert!(!rows.iter().any(|r| matches!(r.kind, RowKind::Account { .. })));
    }

    #[test]
    fn disabled_means_no_action() {
        assert!(MenuRow::new("x", None).disabled());
        assert!(!MenuRow::new("x", Some(Action::Quit)).disabled());
    }
}
