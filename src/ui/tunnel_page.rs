//! The settings window's Tunnel page: take the WireGuard `.conf` (browse,
//! type, or drag it from Files onto the window) and install / connect /
//! uninstall the EVE-only tunnel (spec
//! `docs/superpowers/specs/2026-09-13-yutani-tunnel-page-and-ansible-design.md`).
//!
//! State and view only; the daemon (`App::run_tunnel_action`) does the work
//! on the blocking pool, because `install` shells out to `pkexec` and
//! `connect`/`disconnect` to `systemctl`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use cosmic::Element;
use cosmic::iced::clipboard::mime::AllowedMimeTypes;
use cosmic::widget;

use yutani::model::config::Config;
use yutani::tunnel::status::TunnelStatus;

use super::settings::Msg;

pub const NO_FILE: &str = "Choose the WireGuard configuration file first.";
pub const NOT_A_FILE: &str = "That path is not a readable file.";
/// A dropped or browsed file whose name is not UTF-8: it reaches the field
/// through `Path::display`, which swaps the bad bytes for U+FFFD, and no
/// file is called that. (Carrying the `PathBuf` beside the text would let
/// such a file be installed; that is the window's drop and chooser
/// handlers' business, in `ui/mod.rs`.)
pub const NOT_UTF8: &str =
    "That file's name is not valid UTF-8, which this field cannot hold; rename the file and choose it again.";
/// A drop that carried no `.conf` (a folder, a text file, or a payload the
/// widget could not decode).
pub const NOT_A_CONF_DROP: &str = "dropped, but that was not a .conf file";
pub const BUSY: &str = "Waiting for the previous action to finish…";

/// Whether the XDG file-chooser portal is compiled in (libcosmic's
/// `xdg-portal` feature, enabled in `Cargo.toml`). If that feature ever
/// has to go, the fallback is this constant plus `App::browse_tunnel_conf`
/// (the only other place that names `cosmic::dialog::file_chooser`): the
/// Browse… button disappears and the typed path and drop route stay. This
/// module deliberately imports nothing else the feature gates — the
/// portal's drag MIME type is spelled out below rather than taken from
/// libcosmic — so the drop route does not go with it.
pub const CAN_BROWSE: bool = true;

/// The type the XDG document portal offers for a file dragged out of a
/// sandboxed app: its payload is a portal token, not a path. Spelled out
/// here (and checked against libcosmic's constant in the tests) so that
/// the drop route does not depend on the `xdg-portal` feature.
pub const FILE_TRANSFER_MIME: &str = "application/vnd.portal.filetransfer";

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
    /// The last status read (`None` before the first refresh).
    pub status: Option<TunnelStatus>,
    /// An install/uninstall/connect/disconnect is in flight. The action
    /// itself belongs to the daemon (`App::tunnel_in_flight`), which
    /// outlives this window: a window opened while pkexec is still asking
    /// for the password starts out busy too.
    pub busy: bool,
    /// Whether the file chooser could be built in ([`CAN_BROWSE`]).
    pub can_browse: bool,
    /// What `conf_path` (trimmed) was the last time [`install_blocker`]
    /// looked, and whether it named a file then. `install_blocker` runs
    /// from `view`, which iced calls after every update batch, so without
    /// this the path is `stat`ed ~30x/s while the page shows — and a Files
    /// drop from an SMB share is a gvfs path, where each `stat` is a
    /// network round trip on the UI thread. One look per distinct text
    /// instead; a file moved after that is reported by `install` itself.
    /// Interior mutability because `view` only borrows the state. Not set
    /// by hand: `Default` is the whole of it.
    pub conf_check: RefCell<Option<(String, bool)>>,
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
    let text = state.conf_path.trim();
    if text.is_empty() {
        return Some(NO_FILE);
    }
    let mut check = state.conf_check.borrow_mut();
    let is_file = match check.as_ref() {
        Some((seen, is_file)) if seen == text => *is_file,
        _ => {
            let is_file = Path::new(text).is_file();
            *check = Some((text.to_string(), is_file));
            is_file
        }
    };
    if !is_file {
        return Some(if text.contains('\u{FFFD}') { NOT_UTF8 } else { NOT_A_FILE });
    }
    None
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

/// The state card's headline and sub line.
pub fn headline(status: Option<&TunnelStatus>) -> (String, String) {
    match status {
        Some(t) if t.installed && t.connected => (
            format!("Connected — {}", t.location),
            match &t.exit_address {
                Some(ip) => format!("exit {ip} · EVE traffic only"),
                None => "handshaking · EVE traffic only".to_string(),
            },
        ),
        Some(t) if t.installed && t.failed => (format!("Installed — unit failed"), "see journalctl -u yutani-tunnel · EVE uses your normal connection".to_string()),
        Some(t) if t.installed => ("Installed — not connected".to_string(), "ready to connect · EVE traffic only".to_string()),
        _ => ("No tunnel installed".to_string(), "EVE uses your normal connection".to_string()),
    }
}

/// When the unit file was written: the `INSTALLED` fact.
pub fn installed_when() -> Option<std::time::SystemTime> {
    std::fs::metadata(crate::tunnel::UNIT_PATH).and_then(|m| m.modified()).ok()
}

pub fn view<'a>(settings: &'a super::settings::State, config: &'a Config) -> Element<'a, Msg> {
    use super::settings::Msg as M;
    use super::settings_ui as ui;
    use cosmic::iced::font::Weight;
    use cosmic::iced::{Alignment, Length};
    use cosmic::widget::{Column, Row};

    let state = &settings.tunnel;
    let status = state.status.as_ref();
    let installed = status.is_some_and(|t| t.installed);
    let connected = status.is_some_and(|t| t.connected);
    let on = status.is_some_and(|t| t.installed && t.connected && !t.failed);
    let blocker = install_blocker(state);

    // State card
    let (head, sub) = headline(status);
    let primary: Element<'a, Msg> = if !installed {
        ui::primary_button("Install first", None)
    } else if connected {
        ui::standard_button("Disconnect", (!state.busy).then_some(M::TunnelDisconnect))
    } else {
        ui::primary_button("Connect", (!state.busy).then_some(M::TunnelConnect))
    };
    let line = widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(13)
            .align_y(Alignment::Center)
            .push(ui::glowing_dot(on, ui::Tint::Success, 9.0))
            .push(
                Column::new()
                    .width(Length::Fill)
                    .spacing(2)
                    .push(ui::text(head, ui::STATE_HEADLINE, Weight::Semibold, ui::Role::Ink))
                    .push(ui::mono(sub, ui::STATE_SUB, Weight::Normal, ui::Role::Secondary)),
            )
            .push(primary)
            .push(ui::glyph_button("↻", 36.0, (!state.busy).then_some(M::RefreshTunnel))),
    )
    .width(Length::Fill)
    .padding([15, 16]);
    let fact = |label: &'static str, value: String| {
        Column::new()
            .width(Length::FillPortion(1))
            .spacing(2)
            .push(ui::section_label(label))
            .push(widget::container(ui::mono(value, ui::FACT_VALUE, Weight::Normal, ui::Role::Ink)).width(Length::Fill).clip(true))
    };
    let dash = || "—".to_string();
    let facts = widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(10)
            .push(fact("Location", status.filter(|t| t.installed).map_or_else(dash, |t| t.location.clone())))
            .push(fact("Private key", if installed { "/etc/yutani · root only".to_string() } else { dash() }))
            .push(fact("Installed", if installed { installed_when().map_or_else(dash, |t| crate::model::date::date_time(t)) } else { "never".to_string() })),
    )
    .width(Length::Fill)
    .padding([11, 16]);
    let state_card = widget::container(Column::new().width(Length::Fill).push(line).push(ui::hairline()).push(facts))
        .width(Length::Fill)
        .class(if on { ui::panel_class(ui::Tint::Success, ui::CARD_RADIUS) } else { ui::card_class() });

    // Configuration card
    let mut path_field = widget::text_input("…or type a path", state.conf_path.as_str()).size(ui::FACT_VALUE).font(cosmic::font::mono()).width(Length::Fill);
    if !state.busy {
        path_field = path_field.on_input(M::TunnelConfPath);
    }
    let mut input_row = Row::new().width(Length::Fill).spacing(8).align_y(Alignment::Center).push(path_field);
    if state.can_browse {
        input_row = input_row.push(ui::standard_button("Browse…", (!state.busy).then_some(M::BrowseTunnelConf)));
    }
    input_row = input_row.push(ui::primary_button(if installed { "Replace" } else { "Install tunnel" }, blocker.is_none().then_some(M::InstallTunnel)));
    let mut zone = Column::new()
        .width(Length::Fill)
        .spacing(7)
        .push(ui::text("Drop a wg-quick .conf here", ui::ROW_LABEL, Weight::Normal, ui::Role::Ink))
        .push(ui::prose("From your VPN provider — one server, WireGuard format.", ui::ROW_HELP, ui::Role::Tertiary))
        .push(widget::container(input_row).width(Length::Fill).padding([4, 0, 0, 0]));
    if let Some(reason) = blocker.filter(|r| *r != NO_FILE) {
        zone = zone.push(ui::prose(reason, ui::ROW_HELP, ui::Role::Warning));
    }
    let drop_zone = widget::container(zone).width(Length::Fill).padding(ui::DROP_PAD).class(ui::sunken_class());
    let bullet = |t: &'static str| {
        Row::new().spacing(9).align_y(Alignment::Start).push(widget::container(ui::dot(true, ui::Tint::Accent, 4.0)).padding([6, 0, 0, 0])).push(ui::prose(t, ui::ROW_HELP, ui::Role::Secondary))
    };
    let notes = Column::new()
        .width(Length::Fill)
        .spacing(7)
        .push(bullet("Installing asks for your password once. The private key is written to /etc/yutani, readable only by root, and never shown here."))
        .push(bullet("Delete the downloaded .conf afterwards — it holds that same key and is world-readable in your Downloads folder."));
    let configuration = widget::container(
        Column::new().width(Length::Fill).spacing(14).push(drop_zone).push(notes),
    )
    .width(Length::Fill)
    .padding([16, 16, 14, 16])
    .class(ui::card_class());

    // DNS card
    let servers: Vec<String> = config.tunnel.dns_servers.iter().map(|s| s.to_string()).collect();
    let dns = ui::card(vec![
        ui::row_tight("Resolvers", None, ui::mono(servers.join(", "), ui::FACT_VALUE, Weight::Normal, ui::Role::Secondary)),
        ui::row_tight("Domains routed", None, ui::mono(config.tunnel.dns_domains.join(", "), ui::FACT_VALUE, Weight::Normal, ui::Role::Secondary)),
        widget::container(ui::prose(
            "Written into the tunnel when it is installed — replace the configuration to change them.",
            ui::ROW_HELP,
            ui::Role::Tertiary,
        ))
        .width(Length::Fill)
        .padding([10, 16])
        .into(),
    ]);

    // Danger card
    let actions: Element<'a, Msg> = if settings.uninstall_confirm {
        Row::new()
            .spacing(7)
            .push(ui::standard_button("Cancel", Some(M::CancelUninstall)))
            .push(ui::destructive_button("Uninstall", (!state.busy).then_some(M::UninstallTunnel)))
            .into()
    } else {
        ui::destructive_outline_button("Uninstall…", (installed && !state.busy).then_some(M::AskUninstall))
    };
    let danger = widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(13)
            .align_y(Alignment::Center)
            .push(
                Column::new()
                    .width(Length::Fill)
                    .spacing(2)
                    .push(ui::text("Uninstall tunnel", ui::ROW_LABEL, Weight::Medium, ui::Role::Ink))
                    .push(ui::prose("Removes the interface and the stored private key. EVE goes back to your normal connection.", ui::ROW_HELP, ui::Role::Secondary)),
            )
            .push(actions),
    )
    .width(Length::Fill)
    .padding([13, 15])
    .class(ui::panel_class(ui::Tint::Destructive, ui::CARD_RADIUS));

    Column::new()
        .width(Length::Fill)
        .spacing(ui::PANE_GAP)
        .push(ui::heading("Tunnel", "A WireGuard tunnel used by EVE's traffic only. Everything else keeps your normal route."))
        .push(state_card)
        .push(ui::section("Configuration", configuration))
        .push(ui::section("DNS inside the tunnel", dns))
        .push(danger)
        .into()
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
    fn a_path_the_field_could_not_hold_says_so_instead_of_not_a_file() {
        // A dropped or browsed `caf\xe9.conf` reaches the field through
        // `Path::display`, as `caf\u{FFFD}.conf` — a name no file has. Say
        // what happened rather than "not a readable file".
        let dir = std::env::temp_dir().join(format!("yutani-tunnel-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = State::default();
        s.conf_path = dir.join("caf\u{FFFD}.conf").display().to_string();
        assert_eq!(install_blocker(&s), Some(NOT_UTF8));
        // …unless a file really is called that, which is fine. (A fresh
        // state: the same text is not looked at twice, see `conf_check`.)
        std::fs::write(dir.join("caf\u{FFFD}.conf"), "[Interface]\n").unwrap();
        let mut s = State { conf_path: s.conf_path, ..Default::default() };
        assert_eq!(install_blocker(&s), None);
        // A missing file whose name is plain UTF-8 is still just missing.
        s.conf_path = dir.join("cafe.conf").display().to_string();
        assert_eq!(install_blocker(&s), Some(NOT_A_FILE));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_field_path_is_checked_once_per_text_not_once_per_redraw() {
        // `install_blocker` runs from `view`, so it must not `stat` the
        // path on every redraw: for a gvfs path from a Files drop that is a
        // network round trip on the UI thread ~30x/s. Same text, same
        // answer — even after the file has gone (`install` itself reports a
        // file moved since). New text, fresh look.
        let dir = std::env::temp_dir().join(format!("yutani-tunnel-stat-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("x.conf");
        std::fs::write(&conf, "[Interface]\n").unwrap();
        let mut s = State::default();
        s.conf_path = conf.display().to_string();
        assert_eq!(install_blocker(&s), None);
        std::fs::remove_file(&conf).unwrap();
        assert_eq!(install_blocker(&s), None, "the same text is not stat'ed again");
        // Surrounding whitespace is not new text.
        s.conf_path = format!("  {}  ", conf.display());
        assert_eq!(install_blocker(&s), None);
        s.conf_path = dir.join("y.conf").display().to_string();
        assert_eq!(install_blocker(&s), Some(NOT_A_FILE));
        s.conf_path = conf.display().to_string();
        assert_eq!(install_blocker(&s), Some(NOT_A_FILE), "coming back to a text is a fresh look");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The state card's two lines, per state — never a key.
    #[test]
    fn the_headline_names_the_state_without_secrets() {
        let mut t = TunnelStatus::default();
        assert_eq!(headline(None).0, "No tunnel installed");
        assert_eq!(headline(Some(&t)), ("No tunnel installed".to_string(), "EVE uses your normal connection".to_string()));
        t.installed = true;
        t.location = "London".into();
        assert_eq!(headline(Some(&t)), ("Installed — not connected".to_string(), "ready to connect · EVE traffic only".to_string()));
        t.connected = true;
        assert_eq!(headline(Some(&t)), ("Connected — London".to_string(), "handshaking · EVE traffic only".to_string()));
        t.exit_address = Some("203.0.113.42".into());
        assert_eq!(headline(Some(&t)).1, "exit 203.0.113.42 · EVE traffic only");
        t.connected = false;
        t.failed = true;
        assert!(headline(Some(&t)).0.contains("unit failed"));
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

    #[test]
    fn the_portal_mime_type_is_the_one_libcosmic_uses() {
        // Spelled out locally so the drop route survives without the
        // `xdg-portal` feature; it must still be the type libcosmic (and
        // the portal) actually speak.
        assert_eq!(FILE_TRANSFER_MIME, cosmic::widget::dnd_destination::FILE_TRANSFER_MIME);
    }

    #[test]
    fn percent_decoding_keeps_bad_escapes_and_non_utf8_bytes() {
        assert_eq!(percent_decode("a%20b"), b"a b");
        // Either case of hex digit.
        assert_eq!(percent_decode("%2f%2F"), b"//");
        // An escape that is not one stays as the characters it is.
        assert_eq!(percent_decode("100%"), b"100%");
        assert_eq!(percent_decode("50%4"), b"50%4");
        assert_eq!(percent_decode("%zz%g0"), b"%zz%g0");
        // A `%` right before a good escape: the first is literal, the
        // second decodes.
        assert_eq!(percent_decode("%%41"), b"%A");
        // A file name that is not UTF-8 is still a file name on Linux.
        assert_eq!(percent_decode("caf%E9.conf"), b"caf\xe9.conf");
        assert_eq!(percent_decode(""), b"");
        // Through the URI route, the bytes reach the path unchanged.
        assert_eq!(
            parse_uri_list(b"file:///tmp/caf%E9%20x.conf"),
            vec![PathBuf::from(OsString::from_vec(b"/tmp/caf\xe9 x.conf".to_vec()))]
        );
    }
}
