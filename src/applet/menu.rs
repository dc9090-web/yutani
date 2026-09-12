//! The popup's menu, as data. Spec §4.4–§4.5: the tunnel row, Accounts…
//! (expanding in place to one row per client), Preferences…, the
//! thumbnails row that replaces the old tray's Show/Hide, and Quit — or,
//! with no daemon, a single "Start Yutani".

use crate::applet::Action;
use crate::tunnel::status::Status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Normal,
    /// Quit: `#FF8A7E` text on a `#FF6B5C1F` hover.
    Danger,
    /// A client row under an expanded Accounts…; the dot shows focus.
    Account { active: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: String,
    /// Trailing mono hint, e.g. `systemd` or `not installed`.
    pub hint: Option<String>,
    /// Trailing mono value, e.g. the account count.
    pub trailing: Option<String>,
    /// `None` renders the row disabled.
    pub action: Option<Action>,
    pub kind: RowKind,
    /// The Accounts… header expands/collapses instead of acting.
    pub toggles_accounts: bool,
}

impl MenuRow {
    fn new(label: impl Into<String>, action: Option<Action>) -> Self {
        MenuRow {
            label: label.into(),
            hint: None,
            trailing: None,
            action,
            kind: RowKind::Normal,
            toggles_accounts: false,
        }
    }

    /// A row with no action and no toggle does nothing when pressed, so the
    /// view must paint it as disabled rather than merely inert.
    pub fn disabled(&self) -> bool {
        self.action.is_none() && !self.toggles_accounts
    }
}

/// `status` is `None` in the offline state.
pub fn rows(status: Option<&Status>, accounts_open: bool) -> Vec<MenuRow> {
    let Some(s) = status else {
        return vec![MenuRow::new("Start Yutani", Some(Action::StartDaemon))];
    };
    let mut out = Vec::new();

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

    let mut accounts = MenuRow::new("Accounts…", None);
    accounts.trailing = Some(s.clients.len().to_string());
    // With nothing to list there is nothing to expand: the header still
    // shows the count (0) but is disabled like any other actionless row.
    accounts.toggles_accounts = !s.clients.is_empty();
    out.push(accounts);
    if accounts_open {
        for (i, client) in s.clients.iter().enumerate() {
            let mut row = MenuRow::new(client.name.clone(), Some(Action::Focus(i + 1)));
            row.kind = RowKind::Account { active: client.active };
            out.push(row);
        }
    }

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
        let rows = rows(None, false);
        assert_eq!(labels(&rows), vec!["Start Yutani"]);
        assert_eq!(rows[0].action, Some(Action::StartDaemon));
        assert_eq!(rows[0].kind, RowKind::Normal);
        assert!(!rows[0].toggles_accounts);
        // The expansion state cannot resurrect account rows while offline.
        // (`super::` — the local `rows` binding above shadows the function.)
        assert_eq!(super::rows(None, true).len(), 1);
    }

    #[test]
    fn a_connected_tunnel_offers_disconnect_with_the_truthful_hint() {
        let s = status(true, true, false, &["A", "B"]);
        let rows = rows(Some(&s), false);
        assert_eq!(
            labels(&rows),
            vec!["Disconnect tunnel", "Accounts…", "Preferences…", "Hide thumbnails", "Quit"]
        );
        assert_eq!(rows[0].action, Some(Action::Disconnect));
        assert_eq!(rows[0].hint.as_deref(), Some("systemd"));
        assert_eq!(rows[1].trailing.as_deref(), Some("2"));
        assert!(rows[1].toggles_accounts && rows[1].action.is_none());
        assert_eq!(rows[2].action, Some(Action::Preferences));
        assert_eq!(rows[3].action, Some(Action::HideThumbs));
        assert_eq!(rows[4].action, Some(Action::Quit));
        assert_eq!(rows[4].kind, RowKind::Danger);
    }

    #[test]
    fn a_disconnected_tunnel_offers_connect_and_an_uninstalled_one_is_disabled() {
        let s = status(true, false, false, &[]);
        let rows = rows(Some(&s), false);
        assert_eq!(rows[0].label, "Connect tunnel");
        assert_eq!(rows[0].action, Some(Action::Connect));
        assert_eq!(rows[0].hint.as_deref(), Some("systemd"));

        let s = status(false, false, false, &[]);
        let rows = super::rows(Some(&s), false);
        assert_eq!(rows[0].label, "Connect tunnel");
        assert_eq!(rows[0].action, None, "an uninstalled tunnel cannot be connected");
        assert_eq!(rows[0].hint.as_deref(), Some("not installed"));
    }

    #[test]
    fn expanding_accounts_adds_one_focus_row_per_client_in_layout_order() {
        let s = status(true, true, false, &["KestrelVance", "TrilliumTWO", "TrilliumTHREE"]);
        assert_eq!(rows(Some(&s), false).len(), 5);
        let rows = rows(Some(&s), true);
        assert_eq!(
            labels(&rows),
            vec![
                "Disconnect tunnel",
                "Accounts…",
                "KestrelVance",
                "TrilliumTWO",
                "TrilliumTHREE",
                "Preferences…",
                "Hide thumbnails",
                "Quit",
            ]
        );
        // `focus <n>` is 1-based over the daemon's layout order.
        assert_eq!(rows[2].action, Some(Action::Focus(1)));
        assert_eq!(rows[3].action, Some(Action::Focus(2)));
        assert_eq!(rows[4].action, Some(Action::Focus(3)));
        assert_eq!(rows[2].kind, RowKind::Account { active: false });
        assert_eq!(rows[3].kind, RowKind::Account { active: true });
        assert!(rows.iter().all(|r| r.label != "Accounts…" || r.toggles_accounts));
    }

    #[test]
    fn the_thumbnails_row_follows_the_daemons_hidden_flag() {
        let s = status(true, true, true, &["A"]);
        let rows = rows(Some(&s), false);
        assert_eq!(rows[3].label, "Show thumbnails");
        assert_eq!(rows[3].action, Some(Action::ShowThumbs));
        assert_eq!(rows[1].trailing.as_deref(), Some("1"));
    }

    #[test]
    fn quit_is_always_last_and_the_only_danger_row() {
        for accounts_open in [false, true] {
            let s = status(true, true, false, &["A", "B"]);
            let rows = rows(Some(&s), accounts_open);
            assert_eq!(rows.last().unwrap().label, "Quit");
            assert_eq!(rows.iter().filter(|r| r.kind == RowKind::Danger).count(), 1);
        }
    }

    /// A header with nothing under it must not pretend to expand: no
    /// clients means no expansion rows even while `accounts_open` is true,
    /// and the header itself reads as disabled — the trailing count still
    /// shows the truthful `0`.
    #[test]
    fn accounts_with_no_clients_cannot_be_expanded() {
        let s = status(true, true, false, &[]);
        let rows = rows(Some(&s), true);
        assert_eq!(
            labels(&rows),
            vec!["Disconnect tunnel", "Accounts…", "Preferences…", "Hide thumbnails", "Quit"]
        );
        assert_eq!(rows[1].trailing.as_deref(), Some("0"));
        assert!(!rows[1].toggles_accounts, "an empty list has nothing to expand");
        assert!(rows[1].action.is_none());
        assert!(rows[1].disabled());
    }

    /// Every combination of `action` and `toggles_accounts`:
    /// [`MenuRow::disabled`] is true only when neither can fire a message.
    #[test]
    fn disabled_covers_every_combination_of_action_and_toggle() {
        let mut row = MenuRow::new("x", None);
        assert!(row.disabled(), "no action, no toggle: pressing it does nothing");

        row.toggles_accounts = true;
        assert!(!row.disabled(), "a toggle presses even with no action (Accounts…)");

        row.toggles_accounts = false;
        row.action = Some(Action::Quit);
        assert!(!row.disabled(), "an action presses even with no toggle");

        row.toggles_accounts = true;
        assert!(!row.disabled(), "either an action or a toggle is enough to enable a row");
    }
}
