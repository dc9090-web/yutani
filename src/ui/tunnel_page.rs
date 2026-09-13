//! The settings window's Tunnel page: take the WireGuard `.conf` (browse,
//! type, or drop it on the window) and install / connect / uninstall the
//! EVE-only tunnel (spec
//! `docs/superpowers/specs/2026-09-13-yutani-tunnel-page-and-ansible-design.md`).
//!
//! State and view only; the daemon (`App::run_tunnel_action`) does the work
//! on the blocking pool, because `install` shells out to `pkexec` and
//! `connect`/`disconnect` to `systemctl`.

use std::path::{Path, PathBuf};

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget;

use yutani::model::config::Config;
use yutani::tunnel::status::TunnelStatus;

use super::settings::Msg;

pub const NO_FILE: &str = "Choose the WireGuard configuration file first.";
pub const NOT_A_FILE: &str = "That path is not a readable file.";
pub const BUSY: &str = "Waiting for the previous action to finish…";

/// Whether the XDG file-chooser portal is compiled in (libcosmic's
/// `xdg-portal` feature, enabled in `Cargo.toml`). A one-line change here
/// is the whole fallback if that feature ever has to go: the Browse…
/// button disappears and the typed path and drop route stay.
pub const CAN_BROWSE: bool = true;

#[derive(Debug, Default)]
pub struct State {
    /// The `.conf` path as typed, browsed or dropped.
    pub conf_path: String,
    /// The last status read (`None` before the first refresh).
    pub status: Option<TunnelStatus>,
    /// An install/uninstall/connect/disconnect is in flight.
    pub busy: bool,
    /// Whether the file chooser could be built in ([`CAN_BROWSE`]).
    pub can_browse: bool,
}

/// The first `.conf` among dropped paths.
pub fn conf_candidate(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("conf"))).cloned()
}

/// Why Install is disabled, or `None`.
pub fn install_blocker(state: &State) -> Option<&'static str> {
    if state.busy {
        return Some(BUSY);
    }
    if state.conf_path.trim().is_empty() {
        return Some(NO_FILE);
    }
    if !Path::new(state.conf_path.trim()).is_file() {
        return Some(NOT_A_FILE);
    }
    None
}

/// One line of state; never a key, never an endpoint.
pub fn summary(status: Option<&TunnelStatus>) -> String {
    let Some(t) = status else { return "Status unknown (is the daemon running?)".to_string() };
    if !t.installed {
        return "Not installed".to_string();
    }
    let state = if t.failed {
        "unit failed".to_string()
    } else if t.connected {
        match &t.exit_address {
            Some(ip) => format!("connected, exit {ip}"),
            None => "connected".to_string(),
        }
    } else {
        "disconnected".to_string()
    };
    format!("Installed ({}) · {state}", t.location)
}

/// The note an install leaves behind: the file's *name* and nothing from
/// inside it — the file holds a private key.
pub fn install_note(conf: &Path, result: Result<(), String>) -> String {
    match result {
        Ok(()) => format!(
            "tunnel installed from {}; press Connect",
            conf.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ),
        Err(e) => e,
    }
}

pub fn view<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let installed = state.status.as_ref().is_some_and(|t| t.installed);
    let connected = state.status.as_ref().is_some_and(|t| t.connected);
    let blocker = install_blocker(state);

    let mut file = widget::settings::section().title("WireGuard configuration").add(widget::text::caption(
        "The wg-quick .conf you downloaded from Proton VPN (WireGuard, one server). Browse for it, type its \
         path, or drop the file anywhere on this window.",
    ));
    let mut row: Vec<Element<'a, Msg>> = vec![
        widget::text_input("/home/you/Downloads/EVE-UK-455.conf", state.conf_path.as_str())
            .on_input(Msg::TunnelConfPath)
            .width(Length::Fill)
            .into(),
    ];
    if state.can_browse {
        row.push(
            widget::button::standard("Browse…")
                .on_press_maybe((!state.busy).then_some(Msg::BrowseTunnelConf))
                .into(),
        );
    }
    file = file.add(widget::settings::item_row(row));
    file = file.add(widget::settings::item_row(vec![
        widget::button::suggested(if installed { "Replace configuration" } else { "Install tunnel" })
            .on_press_maybe(blocker.is_none().then_some(Msg::InstallTunnel))
            .into(),
    ]));
    if let Some(reason) = blocker {
        file = file.add(widget::text::caption(reason));
    }
    file = file.add(widget::text::caption(
        "Installing asks for your password once (polkit). The file's private key goes to /etc/yutani, root-only; \
         it is never shown here.",
    ));

    let mut tunnel =
        widget::settings::section().title("Tunnel").add(widget::text::body(summary(state.status.as_ref())));
    if installed {
        tunnel = tunnel.add(widget::settings::item_row(vec![
            widget::button::suggested("Connect")
                .on_press_maybe((!state.busy && !connected).then_some(Msg::TunnelConnect))
                .into(),
            widget::button::standard("Disconnect")
                .on_press_maybe((!state.busy && connected).then_some(Msg::TunnelDisconnect))
                .into(),
            widget::button::destructive("Uninstall")
                .on_press_maybe((!state.busy).then_some(Msg::UninstallTunnel))
                .into(),
            widget::button::text("Refresh").on_press(Msg::RefreshTunnel).into(),
        ]));
    } else {
        tunnel = tunnel.add(widget::settings::item_row(vec![
            widget::button::text("Refresh").on_press(Msg::RefreshTunnel).into(),
        ]));
    }

    let servers: Vec<String> = config.tunnel.dns_servers.iter().map(|s| s.to_string()).collect();
    let dns = widget::settings::section()
        .title("DNS inside the tunnel")
        .add(widget::settings::item("Resolvers", widget::text::body(servers.join(", "))))
        .add(widget::settings::item("Domains", widget::text::body(config.tunnel.dns_domains.join(", "))))
        .add(widget::text::caption(
            "From tunnel.dns_servers / tunnel.dns_domains in config.ron. They are written into the tunnel at \
             install time, so after changing them press Install tunnel again.",
        ));

    widget::settings::view_column(vec![file.into(), tunnel.into(), dns.into()]).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_drop_takes_the_first_conf_file_only() {
        let paths = [PathBuf::from("/tmp/readme.txt"), PathBuf::from("/tmp/EVE-UK-455.conf"), PathBuf::from("/tmp/b.conf")];
        assert_eq!(conf_candidate(&paths), Some(PathBuf::from("/tmp/EVE-UK-455.conf")));
        assert_eq!(conf_candidate(&[PathBuf::from("/tmp/readme.txt")]), None);
        assert_eq!(conf_candidate(&[]), None);
    }

    #[test]
    fn install_needs_a_readable_conf_and_no_action_in_flight() {
        let dir = std::env::temp_dir().join(format!("yutani-tunnel-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("x.conf");
        std::fs::write(&conf, "[Interface]\n").unwrap();
        let mut s = State::default();
        assert_eq!(install_blocker(&s), Some(NO_FILE));
        s.conf_path = conf.to_string_lossy().into_owned();
        assert_eq!(install_blocker(&s), None);
        s.busy = true;
        assert_eq!(install_blocker(&s), Some(BUSY));
        s.busy = false;
        s.conf_path = dir.join("missing.conf").to_string_lossy().into_owned();
        assert_eq!(install_blocker(&s), Some(NOT_A_FILE));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_summary_names_the_state_without_secrets() {
        let mut t = TunnelStatus::default();
        assert_eq!(summary(None), "Status unknown (is the daemon running?)");
        assert_eq!(summary(Some(&t)), "Not installed");
        t.installed = true;
        t.location = "London".into();
        assert_eq!(summary(Some(&t)), "Installed (London) · disconnected");
        t.connected = true;
        t.handshake_age_s = Some(12);
        t.exit_address = Some("203.0.113.42".into());
        assert_eq!(summary(Some(&t)), "Installed (London) · connected, exit 203.0.113.42");
        t.connected = false;
        t.failed = true;
        assert_eq!(summary(Some(&t)), "Installed (London) · unit failed");
    }

    #[test]
    fn the_install_note_names_the_file_not_its_contents() {
        assert_eq!(install_note(Path::new("/home/d/Downloads/EVE-UK-455.conf"), Ok(())), "tunnel installed from EVE-UK-455.conf; press Connect");
        assert_eq!(install_note(Path::new("/x/a.conf"), Err("install cancelled or failed (pkexec exit 126)".into())), "install cancelled or failed (pkexec exit 126)");
    }
}
