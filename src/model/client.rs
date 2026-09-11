//! Pure classification of compositor toplevels into EVE clients.

/// Login state derived from the window title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    /// Login screen, character select, or a transient title.
    LoggingIn,
    /// In game as this character.
    LoggedIn(String),
}

const LAUNCHER_TITLE: &str = "EVE Launcher";
const CLIENT_PREFIX: &str = "EVE - ";

impl Login {
    pub fn label(&self) -> &str {
        match self {
            Login::LoggedIn(name) => name,
            Login::LoggingIn => "Logging in…",
        }
    }
}

/// Decide whether a toplevel is an EVE client and, if so, its login state.
///
/// `None` means "not a client" (wrong app_id, or the launcher).
pub fn classify(app_ids: &[String], app_id: &str, title: &str) -> Option<Login> {
    if !app_ids.iter().any(|id| id == app_id) {
        return None;
    }
    if title == LAUNCHER_TITLE {
        return None;
    }
    if let Some(name) = title.strip_prefix(CLIENT_PREFIX) {
        let name = name.trim();
        if !name.is_empty() {
            return Some(Login::LoggedIn(name.to_string()));
        }
    }
    Some(Login::LoggingIn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        vec!["steam_app_8500".to_string()]
    }

    #[test]
    fn logged_in_title_yields_character_name() {
        assert_eq!(
            classify(&ids(), "steam_app_8500", "EVE - Aria Vex"),
            Some(Login::LoggedIn("Aria Vex".into()))
        );
    }

    #[test]
    fn bare_eve_title_is_logging_in() {
        assert_eq!(classify(&ids(), "steam_app_8500", "EVE"), Some(Login::LoggingIn));
    }

    #[test]
    fn launcher_is_not_a_client() {
        assert_eq!(classify(&ids(), "steam_app_8500", "EVE Launcher"), None);
    }

    #[test]
    fn other_app_id_is_not_a_client() {
        assert_eq!(classify(&ids(), "firefox", "EVE - Aria Vex"), None);
    }

    #[test]
    fn unknown_title_with_matching_app_id_is_logging_in() {
        assert_eq!(classify(&ids(), "steam_app_8500", "Wine crash"), Some(Login::LoggingIn));
    }

    #[test]
    fn empty_name_after_dash_is_logging_in() {
        assert_eq!(classify(&ids(), "steam_app_8500", "EVE -  "), Some(Login::LoggingIn));
    }

    #[test]
    fn label_is_name_or_placeholder() {
        assert_eq!(Login::LoggedIn("Kel".into()).label(), "Kel");
        assert_eq!(Login::LoggingIn.label(), "Logging in…");
    }

    #[test]
    fn shipped_defaults_detect_the_real_client_and_exclude_the_launcher() {
        let ids = crate::model::config::Config::default().app_ids;
        assert_eq!(classify(&ids, "exefile.exe", "EVE"), Some(Login::LoggingIn));
        assert_eq!(classify(&ids, "exefile.exe", "EVE - Aria Vex"), Some(Login::LoggedIn("Aria Vex".into())));
        assert_eq!(classify(&ids, "steam_app_8500", "EVE"), Some(Login::LoggingIn));
        assert_eq!(classify(&ids, "eve-online.exe", "EVE Launcher"), None);
        assert_eq!(classify(&ids, "exefile.exe", "EVE Launcher"), None);
    }
}
