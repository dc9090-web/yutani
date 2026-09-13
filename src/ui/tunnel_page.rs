//! The settings window's Tunnel page: take the WireGuard `.conf` (browse,
//! type, or drag it from Files onto the window) and install / connect /
//! uninstall the EVE-only tunnel (spec
//! `docs/superpowers/specs/2026-09-13-yutani-tunnel-page-and-ansible-design.md`).
//!
//! State and view only; the daemon (`App::run_tunnel_action`) does the work
//! on the blocking pool, because `install` shells out to `pkexec` and
//! `connect`/`disconnect` to `systemctl`.

use std::borrow::Cow;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::iced::clipboard::mime::AllowedMimeTypes;
use cosmic::widget;
use cosmic::widget::dnd_destination::FILE_TRANSFER_MIME;

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

/// The MIME type a file manager offers for a dragged file: one URI per
/// line (RFC 2483). A Wayland client hears about a drop *only* through the
/// compositor's data device, as bytes of some agreed type — never through
/// `iced::window::Event::FileDropped`, which only winit's X11, macOS and
/// Windows backends emit.
pub const URI_LIST_MIME: &str = "text/uri-list";

#[derive(Debug, Default)]
pub struct State {
    /// The `.conf` path as typed, browsed or dropped.
    pub conf_path: String,
    /// The path an install/uninstall in flight was started with, so the
    /// note it leaves names *that* file and not whatever has been typed
    /// into the field since. `None` while nothing is in flight.
    pub pending_conf: Option<PathBuf>,
    /// The last status read (`None` before the first refresh).
    pub status: Option<TunnelStatus>,
    /// An install/uninstall/connect/disconnect is in flight.
    pub busy: bool,
    /// Whether the file chooser could be built in ([`CAN_BROWSE`]).
    pub can_browse: bool,
}

/// The files of one drop, as libcosmic's drag-and-drop destination widget
/// (`widget::dnd_destination::dnd_destination_for_data`, which the settings
/// window wraps its page in) decodes them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DroppedFiles(pub Vec<PathBuf>);

impl AllowedMimeTypes for DroppedFiles {
    /// Most preferred first: the compositor hands the destination the first
    /// of these types the source also offers. `text/uri-list` is the one
    /// every file manager offers and the only one that carries paths; the
    /// portal's file-transfer type is listed after it so that a source
    /// offering only *that* still finds a destination here — its payload is
    /// a portal token, which [`DroppedFiles::try_from`] then refuses rather
    /// than mistaking for a path.
    fn allowed() -> Cow<'static, [String]> {
        Cow::Owned(vec![URI_LIST_MIME.to_string(), FILE_TRANSFER_MIME.to_string()])
    }
}

impl TryFrom<(Vec<u8>, String)> for DroppedFiles {
    type Error = ();

    fn try_from((data, _mime): (Vec<u8>, String)) -> Result<Self, Self::Error> {
        let paths = parse_uri_list(&data);
        if paths.is_empty() { Err(()) } else { Ok(Self(paths)) }
    }
}

/// A `text/uri-list` payload (RFC 2483): one URI per line, blank lines and
/// `#` comment lines skipped. `file:` URIs become paths; every other scheme
/// is dropped — a URI naming something this machine cannot open is not a
/// file `install` could read.
pub fn parse_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(file_uri_to_path)
        .collect()
}

/// `file:///home/you/My%20confs/a.conf` → `/home/you/My confs/a.conf`.
/// Only this machine's files: an empty host or `localhost`, never
/// `file://other-host/…`. (`url::Url::to_file_path` does exactly this, but
/// `url` is not one of our dependencies — it only reaches the build through
/// libcosmic — and this is fifteen lines.)
fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.get(..7).filter(|scheme| scheme.eq_ignore_ascii_case("file://")).and(uri.get(7..))?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(percent_decode(path))))
}

/// Percent-decoding, to bytes: a path is bytes on Linux, and a file name
/// that is not UTF-8 still names a file. An invalid escape (`%zz`, a `%` at
/// the end) is left as the literal characters it is.
fn percent_decode(text: &str) -> Vec<u8> {
    let src = text.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        let escape = (src[i] == b'%')
            .then(|| Some((hex_digit(*src.get(i + 1)?)?, hex_digit(*src.get(i + 2)?)?)))
            .flatten();
        match escape {
            Some((hi, lo)) => {
                out.push(hi * 16 + lo);
                i += 3;
            }
            None => {
                out.push(src[i]);
                i += 1;
            }
        }
    }
    out
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
/// inside it — the file holds a private key. It says so, and says to delete
/// the download, in the same words the CLI's `success_message` uses: the
/// key is in there in the clear, and the download is world-readable.
pub fn install_note(conf: &Path, result: Result<(), String>) -> String {
    match result {
        Ok(()) => {
            let name = conf_name(conf);
            format!(
                "tunnel installed from {name}; press Connect — you can delete {name} now, it holds the private key"
            )
        }
        Err(e) => e,
    }
}

/// What to call the `.conf` in a note: its file name, or the path itself
/// when there is no file name (`/`, `..`) — never its contents.
fn conf_name(conf: &Path) -> String {
    conf.file_name().map_or_else(|| conf.display().to_string(), |n| n.to_string_lossy().into_owned())
}

pub fn view<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let installed = state.status.as_ref().is_some_and(|t| t.installed);
    let connected = state.status.as_ref().is_some_and(|t| t.connected);
    let blocker = install_blocker(state);

    let mut file = widget::settings::section().title("WireGuard configuration").add(widget::text::caption(
        "The wg-quick .conf you downloaded from Proton VPN (WireGuard, one server). Browse for it, type its \
         path, or drag it from Files and drop it on this window.",
    ));
    // No typing while an action is in flight: the field is what the running
    // install was started from, and `busy` already disables every button.
    let mut path_field = widget::text_input("/home/you/Downloads/EVE-UK-455.conf", state.conf_path.as_str())
        .width(Length::Fill);
    if !state.busy {
        path_field = path_field.on_input(Msg::TunnelConfPath);
    }
    let mut row: Vec<Element<'a, Msg>> = vec![path_field.into()];
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
        "After a successful install, delete the downloaded file: it holds the private key and is world-readable.",
    ));
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
            widget::button::standard("Refresh").on_press(Msg::RefreshTunnel).into(),
        ]));
    } else {
        tunnel = tunnel.add(widget::settings::item_row(vec![
            widget::button::standard("Refresh").on_press(Msg::RefreshTunnel).into(),
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
        // A field holding only spaces is an empty field, not a bad path.
        s.conf_path = "   ".to_string();
        assert_eq!(install_blocker(&s), Some(NO_FILE));
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
        // Connected before the exit address is known (no handshake yet, or
        // the lookup failed): the state still has to be readable.
        assert_eq!(summary(Some(&t)), "Installed (London) · connected");
        t.exit_address = Some("203.0.113.42".into());
        assert_eq!(summary(Some(&t)), "Installed (London) · connected, exit 203.0.113.42");
        t.connected = false;
        t.failed = true;
        assert_eq!(summary(Some(&t)), "Installed (London) · unit failed");
    }

    #[test]
    fn the_install_note_names_the_file_not_its_contents() {
        assert_eq!(
            install_note(Path::new("/home/d/Downloads/EVE-UK-455.conf"), Ok(())),
            "tunnel installed from EVE-UK-455.conf; press Connect — you can delete EVE-UK-455.conf now, it holds \
             the private key"
        );
        assert_eq!(install_note(Path::new("/x/a.conf"), Err("install cancelled or failed (pkexec exit 126)".into())), "install cancelled or failed (pkexec exit 126)");
    }

    #[test]
    fn the_install_note_falls_back_to_the_path_when_there_is_no_file_name() {
        assert_eq!(
            install_note(Path::new("/"), Ok(())),
            "tunnel installed from /; press Connect — you can delete / now, it holds the private key"
        );
    }

    #[test]
    fn a_uri_list_gives_the_local_files_it_names() {
        let payload = b"# this is a comment\r\n\
            file:///home/d/Downloads/EVE-UK-455.conf\r\n\
            \r\n\
            file:///home/d/My%20Downloads/EVE%20UK.conf\r\n\
            sftp://nas/share/other.conf\r\n";
        assert_eq!(
            parse_uri_list(payload),
            vec![
                PathBuf::from("/home/d/Downloads/EVE-UK-455.conf"),
                PathBuf::from("/home/d/My Downloads/EVE UK.conf"),
            ]
        );
        // Another machine's files are not ours to read; `localhost` is.
        assert_eq!(parse_uri_list(b"file://nas/share/a.conf"), Vec::<PathBuf>::new());
        assert_eq!(parse_uri_list(b"file://localhost/tmp/a.conf"), vec![PathBuf::from("/tmp/a.conf")]);
        assert_eq!(parse_uri_list(b""), Vec::<PathBuf>::new());
        // The drop only reaches the page as a `DroppedFiles`, and a payload
        // with no local file in it (a portal token, a web URL) is refused.
        assert_eq!(
            DroppedFiles::try_from((b"file:///tmp/a.conf".to_vec(), URI_LIST_MIME.to_string())),
            Ok(DroppedFiles(vec![PathBuf::from("/tmp/a.conf")]))
        );
        assert!(DroppedFiles::try_from((b"1234567".to_vec(), FILE_TRANSFER_MIME.to_string())).is_err());
        assert_eq!(DroppedFiles::allowed().first().map(String::as_str), Some(URI_LIST_MIME));
    }
}
