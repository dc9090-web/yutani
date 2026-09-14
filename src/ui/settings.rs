//! The settings window (spec §6). Opened by `yutani settings` and by the
//! applet's Preferences… row, both through the IPC `settings` request; a
//! second request raises the window that is already open.
//!
//! Every change applies live (`App::apply_config`) and is written back to
//! `config.ron` — except while that file fails to parse, when the window
//! shows what is wrong and writes nothing at all (spec §9: "never overwrite
//! the user's file").

use std::path::{Path, PathBuf};

use cosmic::Element;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::font::Weight;
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::segmented_button;
use cosmic::widget::{self, Column, Row};

use yutani::eve_settings::names::Names;
use yutani::tunnel::status::TunnelStatus;

use crate::model::config::{Config, Edge, Mode, Modifier, Visibility, config_path, parse_color};
use crate::model::layout;

/// The pages of spec §6, plus the Steam one: the launch options EVE needs,
/// which are not a setting at all — they are a string to copy into Steam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Display,
    Behavior,
    Layouts,
    Characters,
    Tunnel,
    Steam,
}

impl Page {
    /// The page an IPC `settings <page>` names; British and American
    /// spellings of Behaviour both work.
    pub fn from_name(name: &str) -> Option<Page> {
        match name.trim().to_ascii_lowercase().as_str() {
            "display" => Some(Page::Display),
            "behavior" | "behaviour" => Some(Page::Behavior),
            "layouts" => Some(Page::Layouts),
            "characters" => Some(Page::Characters),
            "tunnel" => Some(Page::Tunnel),
            "steam" => Some(Page::Steam),
            _ => None,
        }
    }
}

/// The tab strip, in order. `State::new` builds the segmented control from
/// this, so the list *is* the window: a page missing here has no tab.
pub const PAGES: [(&str, Page); 6] = [
    ("Display", Page::Display),
    ("Behavior", Page::Behavior),
    ("Layouts", Page::Layouts),
    ("Characters", Page::Characters),
    ("Tunnel", Page::Tunnel),
    ("Steam", Page::Steam),
];

/// Where a page sits in [`PAGES`], for the code that has to *select* a tab
/// rather than render one (dropping a `.conf` on the window jumps to the
/// Tunnel page). `None` for a page with no tab, which the page-order test
/// forbids.
pub fn page_index(page: Page) -> Option<usize> {
    index_of(&PAGES, &page)
}

/// The settings window's drag-and-drop destination id (see `view`). Far
/// above anything iced's widget-id counter reaches in a session.
pub const SETTINGS_DROP_ID: u64 = 0x5955_5441_4E49_0001; // "YUTANI" + 1

pub const MODES: [(&str, Mode); 2] = [("Floating", Mode::Floating), ("Dock", Mode::Dock)];
pub const EDGES: [(&str, Edge); 4] =
    [("Top", Edge::Top), ("Bottom", Edge::Bottom), ("Left", Edge::Left), ("Right", Edge::Right)];
pub const FPS: [u32; 4] = [10, 15, 30, 60];
pub const FPS_LABELS: [&str; 4] = ["10", "15", "30", "60"];
pub const VISIBILITIES: [(&str, Visibility); 2] =
    [("Always", Visibility::Always), ("Only with EVE focused", Visibility::EveFocusedOnly)];

/// The `focus_prefix` chords the dropdown offers (spec §6, "shortcut
/// prefix"): a fixed list keeps this one control instead of four
/// checkboxes, and every entry is a chord cosmic-comp accepts. A config
/// hand-edited to some other chord simply selects nothing.
pub const PREFIXES: [(&str, &[Modifier]); 5] = [
    ("Ctrl + Alt", &[Modifier::Ctrl, Modifier::Alt]),
    ("Super", &[Modifier::Super]),
    ("Super + Shift", &[Modifier::Super, Modifier::Shift]),
    ("Ctrl + Shift", &[Modifier::Ctrl, Modifier::Shift]),
    ("Super + Alt", &[Modifier::Super, Modifier::Alt]),
];

#[cfg(test)]
fn labels<T>(table: &[(&'static str, T)]) -> Vec<&'static str> {
    table.iter().map(|(label, _)| *label).collect()
}

fn index_of<T: PartialEq>(table: &[(&'static str, T)], value: &T) -> Option<usize> {
    table.iter().position(|(_, v)| v == value)
}

pub fn prefix_index(prefix: &[Modifier]) -> Option<usize> {
    PREFIXES.iter().position(|(_, p)| *p == prefix)
}

pub fn prefix_at(index: usize) -> Option<Vec<Modifier>> {
    PREFIXES.get(index).map(|(_, p)| p.to_vec())
}

/// The settings window's own building blocks.
use super::settings_ui as ui;

/// Where the Characters page's copy stands (handoff "Pane: Characters",
/// step 3): idle, asking for confirmation, or done with the sentence to
/// show.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum CopyPhase {
    #[default]
    Idle,
    Confirm,
    Done(String),
}

/// The frame rate pills the Behaviour pane offers (the handoff's three);
/// indices into [`FPS`].
pub const FPS_CHOICES: [usize; 3] = [1, 2, 3];
/// The prefix pills the Behaviour pane offers (the handoff's three);
/// indices into [`PREFIXES`].
pub const PREFIX_CHOICES: [usize; 3] = [0, 1, 3];

/// The handoff's frame-colour palette; `None` is the theme accent, offered
/// for the focused border only (it is today's default).
pub const PALETTE: [ui::Swatch; 4] = [
    ui::Swatch { hex: Some("#7aa2f7") },
    ui::Swatch { hex: Some("#9ece6a") },
    ui::Swatch { hex: Some("#e0af68") },
    ui::Swatch { hex: Some("#1b2130") },
];

/// The help line under the frame-rate pills, per value.
pub fn fps_help(fps: u32) -> &'static str {
    match fps {
        60 => "Smoothest, and the most GPU per client. Drop to 30 when running many clients.",
        30 => "A good balance for six or more clients.",
        _ => "Cheapest — thumbnails update visibly in steps.",
    }
}

/// The derived shortcut chips: `(what, combo)`.
pub fn shortcut_chips(shortcuts: &crate::model::config::ShortcutsConfig) -> [(&'static str, String); 4] {
    let p = shortcuts.focus_prefix.iter().map(|m| m.label()).collect::<Vec<_>>().join(" + ");
    let key = |k: &str| crate::model::config::key_symbol(k);
    [
        ("Focus client 1 – 9", format!("{p} + 1 … 9")),
        ("Next client", format!("{p} + {}", key(&shortcuts.next))),
        ("Previous client", format!("{p} + {}", key(&shortcuts.prev))),
        ("Show / hide thumbnails", format!("{p} + T")),
    ]
}

/// The Steam launch line, with the binary spelled out when Steam cannot
/// find `yutani` on its PATH.
pub fn launch_command(full_path: bool, exe: &str) -> String {
    if full_path {
        yutani::STEAM_LAUNCH_ARGS.replace(" yutani launch", &format!(" {exe} launch"))
    } else {
        yutani::STEAM_LAUNCH_ARGS.to_string()
    }
}

/// The Characters pane's confirmation sentence (handoff: it must name the
/// source, the count and the scope).
pub fn copy_confirm_text(source: &str, others: usize, account: bool) -> String {
    let scope = if account { "interface and account" } else { "interface" };
    let n = if others == 1 { "1 character".to_string() } else { format!("{others} characters") };
    format!("This replaces the {scope} settings of {n} with {source}’s. Their current files are backed up first, and can be restored below.")
}

/// The Characters pane's done sentence.
pub fn copy_done_text(source: &str, characters: usize) -> String {
    let n = if characters == 1 { "1 character".to_string() } else { format!("{characters} characters") };
    format!("{source}’s settings copied to {n}. Restart any running client to see them.")
}

/// The sidebar's live sublabel for the Tunnel pane.
pub fn tunnel_sublabel(status: Option<&TunnelStatus>) -> String {
    match status {
        Some(t) if t.installed && t.connected => format!("Connected · {}", t.location),
        Some(t) if t.installed => "Installed · idle".to_string(),
        _ => "Not installed".to_string(),
    }
}

/// Everything the settings window owns. `None` on `App` while it is closed.
pub struct State {
    pub window: SurfaceId,
    /// The pane list ([`PAGES`]); its active entity carries a `Page`.
    pub pages: segmented_button::SingleSelectModel,
    /// Set when `config.ron` exists but does not parse: the Display and
    /// Behavior pages are replaced by the reason and nothing is written.
    /// The Layouts page still works — it writes layout files, not this one.
    pub config_error: Option<String>,
    /// Saved layouts with their counts, refreshed after every Layouts action.
    pub layouts: Vec<layout::LayoutSummary>,
    /// The name last applied or saved (`Layout::last_applied`).
    pub applied_layout: Option<String>,
    /// The "Save current arrangement" field.
    pub name_field: String,
    /// Which layout's `⋯` menu is open.
    pub layout_menu: Option<String>,
    /// A rename in progress: `(name, draft)`.
    pub renaming: Option<(String, String)>,
    /// A delete awaiting its inline confirmation.
    pub confirm_delete: Option<String>,
    /// Text fields are kept as typed, so a half-typed value never reaches
    /// the config; they are seeded when the window opens and are not
    /// re-seeded by a later hand edit (which would eat what is being typed).
    pub active_border_field: String,
    pub inactive_border_field: String,
    /// One line of feedback, shown at the header's end in place of `saved`.
    pub note: Option<String>,
    /// Set when `current.ron` exists but does not parse: auto-save is
    /// suspended until it does (spec §10), so every page says so and the
    /// Layouts page shows the reason.
    pub layout_error: Option<String>,
    /// The Characters page's own state (`super::characters`): EVE's files,
    /// not ours, so it is refreshed by `App::refresh_characters`.
    pub characters: super::characters::State,
    /// The Characters pane's copy flow.
    pub copy_phase: CopyPhase,
    /// The Tunnel page's own state (`super::tunnel_page`): the systemd unit
    /// and `/etc/yutani`, not `config.ron`, so it is refreshed by
    /// `App::refresh_tunnel`.
    pub tunnel: super::tunnel_page::State,
    /// The Tunnel pane's Uninstall… awaits its confirmation.
    pub uninstall_confirm: bool,
    /// Steam: the Copy button reads "Copied" for 1.6 s.
    pub copied: bool,
    /// Steam: "Steam can't find yutani" — the command with the full path.
    pub steam_full_path: bool,
    /// This binary's absolute path, for that command.
    pub exe_path: String,
}

impl State {
    pub fn new(window: SurfaceId, config: &Config) -> Self {
        let mut pages = segmented_button::SingleSelectModel::default();
        for (label, page) in PAGES {
            pages.insert().text(label).data(page);
        }
        pages.activate_position(0);
        let exe_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "/usr/local/bin/yutani".to_string());
        let mut state = Self {
            window,
            pages,
            config_error: None,
            layouts: Vec::new(),
            applied_layout: None,
            name_field: String::new(),
            layout_menu: None,
            renaming: None,
            confirm_delete: None,
            active_border_field: config.active_border.clone().unwrap_or_default(),
            inactive_border_field: config.inactive_border.clone(),
            note: None,
            layout_error: None,
            characters: Default::default(),
            copy_phase: CopyPhase::Idle,
            tunnel: super::tunnel_page::State {
                can_browse: super::tunnel_page::CAN_BROWSE,
                ..Default::default()
            },
            uninstall_confirm: false,
            copied: false,
            steam_full_path: false,
            exe_path,
        };
        state.refresh();
        state
    }

    /// Re-read what lives outside `Config`: the saved layouts, whether
    /// `config.ron` currently parses, and whether `current.ron` does.
    pub fn refresh(&mut self) {
        self.layouts = layout::summaries();
        self.refresh_from(&config_path());
        self.refresh_layout_from(&layout::current_path());
    }

    /// The "does `config.ron` parse?" half of [`State::refresh`], against an
    /// explicit path — the seam `Config::try_load_from` gives the loader, so
    /// the guard that stops a broken file being overwritten is testable
    /// without a real `~/.config`.
    pub fn refresh_from(&mut self, config: &Path) {
        self.config_error = Config::try_load_from(config).err();
    }

    /// The same seam for `current.ron`. A missing file is not an error — it
    /// is simply the state before the first save.
    pub fn refresh_layout_from(&mut self, current: &Path) {
        self.layout_error = layout::Layout::try_load_from(current).err();
    }

    /// Transient UI state is never persisted and is cleared on pane change.
    pub fn clear_transient(&mut self) {
        self.layout_menu = None;
        self.renaming = None;
        self.confirm_delete = None;
        self.copy_phase = CopyPhase::Idle;
        self.uninstall_confirm = false;
    }

    pub fn page(&self) -> Page {
        self.pages.active_data::<Page>().copied().unwrap_or(Page::Display)
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// The window finished opening.
    Opened,
    /// An xdg-activation token for raising the window (`None`: the
    /// compositor refused, so nothing to do).
    Raise(Option<String>),
    /// The header bar's close button.
    Close,
    /// The window is gone.
    Closed,
    /// The header bar is being dragged.
    Drag,
    /// Re-read `config.ron` after the user fixed it.
    Recheck,
    /// A slider was released: write what the drag already applied.
    Commit,
    ThumbWidth(u32),
    Zoom(f32),
    /// Thumbnail opacity, percent (opacity spec §1.2).
    Opacity(u8),
    BorderPx(u32),
    CornerRadius(u32),
    ShowNames(bool),
    ActiveBorder(String),
    InactiveBorder(String),
    /// Border 1, radius 8, Accent, the inactive default.
    ResetFrame,
    /// A sidebar item was pressed.
    Page(segmented_button::Entity),
    Mode(usize),
    DockEdge(usize),
    Fps(usize),
    Visibility(usize),
    HideActive(bool),
    SnapGrid(bool),
    SnapEdges(bool),
    /// The prefix pills; the shortcuts are reinstalled with the new one.
    Prefix(usize),
    /// Put the launch line on the clipboard.
    CopySteamArgs,
    /// 1.6 s later: the Copy button reads "Copy" again.
    CopyReset,
    /// Steam: "Steam can't find yutani".
    SteamFullPath(bool),
    /// Characters page: the source card pressed.
    SourceCharacter(usize),
    /// Characters page: also copy the account-wide file.
    CopyAccount(bool),
    /// Characters page: step 3 idle → confirm.
    AskCopy,
    /// Characters page: back to idle from confirm or done.
    CancelCopy,
    /// Characters page: do the copy (refused while a client is running).
    CopyCharacters,
    /// Characters page: put the newest backup back.
    RestoreBackup,
    /// Characters page: open the folder chooser for the EVE profile.
    ChangeProfileDir,
    /// The folder chooser answered (`None`: cancelled or unavailable).
    ProfileDirChosen(Option<PathBuf>),
    /// A names lookup finished: what is known, and the error if any id is still unnamed.
    Names(Names, Option<String>),
    /// Files dragged onto this window and dropped on it, decoded from the
    /// drop's `text/uri-list`. The window's whole page is a drag-and-drop
    /// destination ([`view`]): on Wayland a drop arrives through the
    /// compositor's data device and nowhere else.
    FilesDropped(Vec<PathBuf>),
    /// Tunnel page: the `.conf` path field, as typed/browsed/dropped.
    TunnelConfPath(String),
    /// Tunnel page: open the XDG file chooser.
    BrowseTunnelConf,
    /// The file chooser answered (`None`: cancelled or unavailable).
    TunnelConfChosen(Option<PathBuf>),
    /// Tunnel page: install (or replace) the tunnel from the chosen file.
    InstallTunnel,
    /// Tunnel page: Uninstall… → the inline confirm.
    AskUninstall,
    CancelUninstall,
    /// Tunnel page: remove the unit, the rules and `/etc/yutani`.
    UninstallTunnel,
    /// Tunnel page: `systemctl start`.
    TunnelConnect,
    /// Tunnel page: `systemctl stop`.
    TunnelDisconnect,
    /// Tunnel page: re-read the state now.
    RefreshTunnel,
    /// The tunnel's current state, re-read after every action and on open.
    /// Boxed: `TunnelStatus` is much the largest thing a `Msg` could carry,
    /// and every other variant would pay for it.
    TunnelStatus(Box<TunnelStatus>),
    /// An action finished: which one, and how it went.
    TunnelDone(TunnelAction, Result<(), String>),
    /// The "Save current arrangement" field.
    Name(String),
    SaveAs,
    Apply(String),
    /// Layouts: the row's `⋯`.
    LayoutMenu(String),
    LayoutRenameStart(String),
    LayoutRenameDraft(String),
    LayoutRenameCommit,
    LayoutDuplicate(String),
    LayoutDeleteAsk(String),
    LayoutCancel,
    /// Layouts: the confirmed delete.
    Delete(String),
    /// Layouts: centre the floating thumbnails on each monitor.
    CentreVertically,
}

/// Why *Centre vertically* is disabled, or `None` (opacity-and-centre
/// spec §2.1).
pub fn centre_blocker(mode: Mode, thumbs_shown: bool) -> Option<&'static str> {
    if mode == Mode::Dock {
        Some("The dock is already centred along its edge.")
    } else if !thumbs_shown {
        Some("No thumbnails are showing.")
    } else {
        None
    }
}

/// The four things the Tunnel page can ask the daemon to do. Each runs the
/// same function the CLI and the IPC requests run, on the blocking pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelAction {
    Install,
    Uninstall,
    Connect,
    Disconnect,
}

/// The settings window: an ordinary xdg-toplevel. Undecorated because
/// libcosmic draws the header bar itself (`widget::header_bar`), and
/// `exit_on_close_request: true` so the compositor's own close gesture
/// closes it and the app only reacts to `Closed`.
pub fn window_settings(app_id: &str) -> cosmic::iced::window::Settings {
    cosmic::iced::window::Settings {
        size: cosmic::iced::Size::new(ui::WINDOW_W, ui::WINDOW_H),
        min_size: Some(cosmic::iced::Size::new(ui::MIN_W, ui::MIN_H)),
        resizable: true,
        decorations: false,
        exit_on_close_request: true,
        platform_specific: cosmic::iced::window::PlatformSpecific {
            application_id: app_id.to_string(),
            override_redirect: false,
        },
        ..Default::default()
    }
}

/// `active_border`: empty means "follow the COSMIC theme accent" (`None`);
/// anything else must parse as `#rrggbb[aa]` (spec §9).
pub fn parse_optional_color(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match parse_color(text) {
        Some(_) => Ok(Some(text.to_string())),
        None => Err(format!("{text:?} is not #rrggbb or #rrggbbaa")),
    }
}

/// `inactive_border` has no "follow the theme" option: it must parse.
pub fn parse_required_color(text: &str) -> Result<String, String> {
    let text = text.trim();
    match parse_color(text) {
        Some(_) => Ok(text.to_string()),
        None => Err(format!("{text:?} is not #rrggbb or #rrggbbaa")),
    }
}

/// True for the messages a slider fires continuously while dragging: they
/// apply live but must not write `config.ron` on every pixel. The slider's
/// `on_release` sends `Commit`, which writes once.
pub fn is_live_only(msg: &Msg) -> bool {
    matches!(msg, Msg::ThumbWidth(_) | Msg::Zoom(_) | Msg::Opacity(_))
}

/// True for the messages that make the note line stale: a page the note
/// was about is going away, or the files it was about have just been
/// re-read. (A config field clears the note too, but through
/// `apply_config_field`'s answer rather than this predicate.)
pub fn clears_note(msg: &Msg) -> bool {
    matches!(msg, Msg::Page(_) | Msg::Recheck)
}

/// Apply one settings change to `config`. `Ok(true)` = the config changed
/// and must be applied live and written; `Ok(false)` = the message was not
/// a config field; `Err` = a message for the note line, nothing changed.
pub fn apply_config_field(config: &mut Config, msg: &Msg) -> Result<bool, String> {
    let before = config.clone();
    match msg {
        Msg::ThumbWidth(v) => config.thumb_width = (*v).clamp(80, 1600),
        // Rounded to the slider's own step first: f32 arithmetic on the way
        // out of the widget otherwise puts a 1.3000001 in the file.
        Msg::Zoom(v) => config.zoom_factor = ((*v * 10.0).round() / 10.0).clamp(1.0, 4.0),
        Msg::Opacity(v) => config.thumb_opacity = (*v).clamp(20, 100),
        Msg::BorderPx(v) => config.border_px = (*v).min(16),
        Msg::CornerRadius(v) => config.corner_radius = (*v).min(64),
        Msg::ShowNames(v) => config.show_names = *v,
        Msg::ActiveBorder(text) => config.active_border = parse_optional_color(text)?,
        Msg::InactiveBorder(text) => config.inactive_border = parse_required_color(text)?,
        Msg::ResetFrame => {
            let d = Config::default();
            config.border_px = 1;
            config.corner_radius = 8;
            config.active_border = None;
            config.inactive_border = d.inactive_border;
        }
        Msg::Mode(i) => config.mode = MODES.get(*i).ok_or_else(|| "unknown mode".to_string())?.1,
        Msg::DockEdge(i) => config.dock_edge = EDGES.get(*i).ok_or_else(|| "unknown dock edge".to_string())?.1,
        Msg::Fps(i) => config.fps = *FPS.get(*i).ok_or_else(|| "unknown frame rate".to_string())?,
        Msg::Visibility(i) => {
            config.visibility = VISIBILITIES.get(*i).ok_or_else(|| "unknown visibility".to_string())?.1;
        }
        Msg::HideActive(v) => config.hide_active = *v,
        Msg::SnapGrid(v) => config.snap_grid = *v,
        Msg::SnapEdges(v) => config.snap_edges = *v,
        Msg::Prefix(i) => {
            config.shortcuts.focus_prefix = prefix_at(*i).ok_or_else(|| "unknown shortcut prefix".to_string())?;
        }
        Msg::ProfileDirChosen(Some(path)) => {
            let text = path.to_str().ok_or_else(|| "that folder's name is not valid UTF-8".to_string())?;
            config.eve_settings_dir = Some(text.to_string());
        }
        _ => return Ok(false),
    }
    Ok(*config != before)
}

// ---- the window ------------------------------------------------------------------

/// The whole window: header bar, sidebar, and the scrolling pane.
pub fn view<'a>(
    state: &'a State,
    config: &'a Config,
    focused: bool,
    clients_running: bool,
    thumbs_shown: bool,
) -> Element<'a, super::Msg> {
    let page = state.page();
    let body: Element<'a, Msg> = match (page, &state.config_error) {
        // Layout files are not `config.ron`; this page works either way.
        (Page::Layouts, _) => layouts_page(state, config, thumbs_shown),
        // Nor does the Steam page read or write anything: it is a fixed
        // string and a Copy button, and it is exactly the page someone
        // whose config is broken may still need.
        (Page::Steam, _) => steam_page(state),
        // Nor is EVE's profile directory `config.ron`: this page copies
        // CCP's files and is just as usable while ours does not parse.
        (Page::Characters, _) => super::characters::view(state, clients_running),
        // Nor is the tunnel `config.ron`: the page drives systemd units and
        // `/etc/yutani`. It only *reads* `config.tunnel` for the DNS lines,
        // and that is the in-memory (validated) config, so it works while
        // the file on disk does not parse.
        (Page::Tunnel, _) => super::tunnel_page::view(state, config),
        (_, Some(error)) => broken_config(error),
        (Page::Display, None) => display_page(state, config),
        (Page::Behavior, None) => behavior_page(config),
    };
    let pane = widget::scrollable(widget::container(body).width(Length::Fill).padding(ui::CONTENT_PAD))
        .width(Length::Fill)
        .height(Length::Fill);
    let split = Row::new().width(Length::Fill).height(Length::Fill).push(sidebar(state, config)).push(pane);

    // The header's quiet `saved`, or the last note in its place. A
    // suspended auto-save outranks both: the user is about to drag.
    let status: Element<'a, Msg> = match (&state.note, &state.layout_error, &state.config_error) {
        (Some(note), _, _) => ui::mono(note.as_str(), ui::SAVED, Weight::Normal, ui::Role::Tertiary),
        (None, Some(_), _) => ui::mono("positions not saved: current.ron unreadable", ui::SAVED, Weight::Normal, ui::Role::Warning),
        (None, None, Some(_)) => ui::mono("config.ron not written while unreadable", ui::SAVED, Weight::Normal, ui::Role::Warning),
        (None, None, None) => ui::mono("saved", ui::SAVED, Weight::Normal, ui::Role::Tertiary),
    };
    // The whole window — header bar included, so a file let go anywhere
    // on it lands — is the drop target for the Tunnel page's `.conf`
    // (`tunnel_page::DroppedFiles` names the MIME types it takes). The
    // destination widget hands every ordinary event to its child first,
    // so the header bar's drag and close still work inside it. This —
    // libcosmic's drag-and-drop destination widget, fed by the compositor's
    // data device — is the only route a drop takes on Wayland;
    // `iced::window::Event::FileDropped` is winit's X11/macOS/Windows
    // event and never fires here. A drop the widget cannot decode arrives
    // as `None`, i.e. no files, and changes nothing.
    // The destination's id is pinned: the widget mints a fresh one on every
    // `view`, and the compositor resolves a drop against the id it saw at
    // the last pointer motion — with thumbnails redrawing at 30 fps the two
    // would rarely match and the drop would be lost. And the drag is a
    // COPY: this window records a path, it never takes the file, so a file
    // manager that honours MOVE must not delete the user's only copy of the
    // private key before Install has run.
    let window = widget::column::with_children(vec![
        widget::header_bar()
            .title("Yutani Settings")
            .on_close(Msg::Close)
            .on_drag(Msg::Drag)
            .focused(focused)
            .end(widget::container(status).padding([0, 8]))
            .into(),
        split.into(),
    ]);
    let dropzone: Element<'a, Msg> =
        widget::dnd_destination::dnd_destination_for_data::<super::tunnel_page::DroppedFiles, _>(
            window,
            |data, _action| Msg::FilesDropped(data.map(|files| files.0).unwrap_or_default()),
        )
        .drag_id(SETTINGS_DROP_ID)
        .action(cosmic::iced::clipboard::dnd::DndAction::Copy)
        .preferred_action(cosmic::iced::clipboard::dnd::DndAction::Copy)
        .into();
    let content: Element<'a, Msg> = widget::container(dropzone)
        .class(cosmic::theme::Container::WindowBackground)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    content.map(super::Msg::Settings)
}

/// The sidebar: one two-line item per pane, with its live sublabel, and
/// the footer card.
fn sidebar<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let page = state.page();
    let mut items = Column::new().width(Length::Fill).spacing(2);
    for entity in state.pages.iter() {
        let Some(p) = state.pages.data::<Page>(entity).copied() else { continue };
        let (name, sub) = match p {
            Page::Display => ("Display", "Size, names, frame".to_string()),
            Page::Behavior => ("Behaviour", "Placement, keys".to_string()),
            Page::Layouts => ("Layouts", format!("{} saved", state.layouts.len())),
            Page::Characters => ("Characters", "Copy EVE settings".to_string()),
            Page::Tunnel => ("Tunnel", tunnel_sublabel(state.tunnel.status.as_ref())),
            Page::Steam => ("Steam", "Launch options".to_string()),
        };
        items = items.push(ui::sidebar_item(name, sub, p == page, Msg::Page(entity)));
    }
    let _ = config;
    widget::container(
        Column::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .push(items)
            .push(widget::space().height(Length::Fill))
            .push(ui::sidebar_footer()),
    )
    .width(Length::Fixed(ui::SIDEBAR_W))
    .height(Length::Fill)
    .padding(ui::SIDEBAR_PAD)
    .class(ui::sidebar_class())
    .into()
}

/// A pane: heading, then its sections 22 px apart.
fn pane<'a>(title: &'a str, sub: &'a str, sections: Vec<Element<'a, Msg>>) -> Element<'a, Msg> {
    let mut col = Column::new().width(Length::Fill).spacing(ui::PANE_GAP).push(ui::heading(title, sub));
    for s in sections {
        col = col.push(s);
    }
    col.into()
}

/// Shown instead of the pages while `config.ron` does not parse: nothing is
/// editable, because the only safe thing to do with the user's broken file
/// is leave it alone (spec §9/§10).
fn broken_config(error: &str) -> Element<'_, Msg> {
    pane(
        "Settings",
        "config.ron cannot be read.",
        vec![ui::panel(
            ui::Tint::Destructive,
            Column::new()
                .spacing(10)
                .push(ui::prose(error, ui::BODY, ui::Role::Ink))
                .push(ui::prose(
                    "Yutani is running with default settings. Nothing here can be changed until the file parses — \
                     it is never overwritten. Fix or delete it, then press Re-check.",
                    ui::ROW_HELP,
                    ui::Role::Secondary,
                ))
                .push(Row::new().push(ui::standard_button("Re-check", Some(Msg::Recheck)))),
        )],
    )
}

// ---- Display ----------------------------------------------------------------------

/// The preview strip: two mock thumbnails at 42 % of the real width, 16:9,
/// with the live radius, border, colours, opacity and captions.
fn preview_strip<'a>(config: &'a Config) -> Element<'a, Msg> {
    let w = (config.thumb_width as f32 * 0.42).round();
    let h = (w * 0.5625).round();
    let radius = config.corner_radius as f32;
    let border = (config.border_px as f32).max(1.0);
    let alpha = f32::from(config.thumb_opacity) / 100.0;
    let inactive = parse_color(&config.inactive_border).unwrap_or([0.25, 0.25, 0.25, 1.0]);
    let active = config.active_border.as_deref().and_then(parse_color);
    let mock = |label: &'static str, name: &'static str, focused: bool| {
        let frame = widget::container(
            widget::container(ui::mono(label, 9.5, Weight::Normal, ui::Role::Tertiary))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .class(cosmic::theme::Container::custom(move |theme| {
            let c = theme.cosmic();
            let edge = match (focused, active) {
                (true, Some([r, g, b, a])) => Color { r, g, b, a },
                (true, None) => c.accent_color().into(),
                (false, _) => Color { r: inactive[0], g: inactive[1], b: inactive[2], a: inactive[3] },
            };
            cosmic::iced::widget::container::Style {
                background: Some(cosmic::iced::Background::Color(yutani::applet::theme::with_alpha(yutani::applet::theme::ink(c), 0.06 * alpha))),
                border: cosmic::iced::Border { radius: radius.into(), width: border, color: yutani::applet::theme::with_alpha(edge, edge.a * alpha) },
                ..Default::default()
            }
        }));
        let mut col = Column::new().spacing(5).align_x(Alignment::Center).push(frame);
        if config.show_names {
            col = col.push(ui::mono(name, ui::CAPTION, Weight::Normal, ui::Role::Secondary));
        }
        col
    };
    widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(12)
            .align_y(Alignment::End)
            .push(mock("client 1", "Sasha-9999", true))
            .push(mock("client 2", "Ishukone", false))
            .push(widget::space().width(Length::Fill))
            .push(ui::mono(format!("live preview · {} px", config.thumb_width), ui::CAPTION, Weight::Normal, ui::Role::Tertiary)),
    )
    .width(Length::Fill)
    .padding(ui::PREVIEW_PAD)
    .class(ui::sunken_class())
    .into()
}

/// A slider row: label block, mono value, the slider.
fn slider_row<'a>(label: &'a str, help: &'a str, value: String, slider: Element<'a, Msg>) -> Element<'a, Msg> {
    ui::row(
        label,
        Some(help),
        Row::new()
            .spacing(16)
            .align_y(Alignment::Center)
            .push(widget::container(ui::mono(value, ui::VALUE, Weight::Normal, ui::Role::Ink)).width(Length::Fixed(ui::VALUE_W)).align_x(Horizontal::Right))
            .push(slider),
    )
}

fn display_page<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let width = slider_row(
        "Thumbnail width",
        "Height follows the client's aspect ratio.",
        format!("{} px", config.thumb_width),
        widget::slider(160..=640u32, config.thumb_width.clamp(160, 640), Msg::ThumbWidth).step(16u32).on_release(Msg::Commit).width(Length::Fixed(ui::SLIDER_W)).into(),
    );
    let zoom = slider_row(
        "Hover zoom",
        "Scale applied while the pointer is over a thumbnail.",
        format!("{:.1}×", config.zoom_factor),
        widget::slider(1.0..=2.5f32, config.zoom_factor.clamp(1.0, 2.5), Msg::Zoom).step(0.1f32).on_release(Msg::Commit).width(Length::Fixed(ui::SLIDER_W)).into(),
    );
    let opacity = slider_row(
        "Thumbnail opacity",
        "The thumbnail under the pointer is always fully opaque.",
        format!("{}%", config.thumb_opacity),
        widget::slider(20..=100u8, config.thumb_opacity, Msg::Opacity).step(1u8).on_release(Msg::Commit).width(Length::Fixed(ui::SLIDER_W)).into(),
    );
    let names = ui::row("Show character names", Some("Caption under each thumbnail."), ui::toggle(config.show_names, Msg::ShowNames));
    let thumbnails = ui::card(vec![preview_strip(config), width, zoom, opacity, names]);

    let border = ui::row_tight(
        "Border width",
        None,
        ui::stepper(
            format!("{} px", config.border_px),
            (config.border_px > 0).then(|| Msg::BorderPx(config.border_px - 1)),
            (config.border_px < 8).then(|| Msg::BorderPx(config.border_px + 1)),
        ),
    );
    let radius = ui::row_tight(
        "Corner radius",
        None,
        ui::stepper(
            format!("{} px", config.corner_radius),
            (config.corner_radius > 0).then(|| Msg::CornerRadius(config.corner_radius.saturating_sub(2))),
            (config.corner_radius < 24).then(|| Msg::CornerRadius(config.corner_radius + 2)),
        ),
    );
    let mut focused_palette = vec![ui::Swatch { hex: None }];
    focused_palette.extend(PALETTE);
    let active = ui::row_tight(
        "Focused client border",
        Some("Marks the client your keyboard is driving."),
        ui::swatches(&focused_palette, config.active_border.as_deref(), Msg::ActiveBorder),
    );
    let inactive = ui::row_tight(
        "Other clients border",
        Some("Keep it dim so the focused one stands out."),
        ui::swatches(&PALETTE, Some(config.inactive_border.as_str()), Msg::InactiveBorder),
    );
    let reset = widget::container(
        Row::new().width(Length::Fill).push(widget::space().width(Length::Fill)).push(ui::standard_button("Reset frame to defaults", Some(Msg::ResetFrame))),
    )
    .width(Length::Fill)
    .padding(ui::CARD_FOOTER_PAD)
    .into();
    let _ = state;
    pane(
        "Display",
        "How each EVE client's thumbnail looks on the overlay.",
        vec![thumbnails, ui::section("Frame", ui::card(vec![border, radius, active, inactive, reset]))],
    )
}

// ---- Behaviour ----------------------------------------------------------------------

fn behavior_page<'a>(config: &'a Config) -> Element<'a, Msg> {
    let floating = index_of(&MODES, &Mode::Floating).unwrap_or(0);
    let dock = index_of(&MODES, &Mode::Dock).unwrap_or(1);
    let modes = widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(8)
            .push(ui::choice_card("Floating", "Drag each thumbnail anywhere.", config.mode == Mode::Floating, Msg::Mode(floating)))
            .push(ui::choice_card("Docked", "Auto-arranged along one screen edge.", config.mode == Mode::Dock, Msg::Mode(dock))),
    )
    .width(Length::Fill)
    .padding([13, 16])
    .into();
    let mut arrangement = vec![modes];
    if config.mode == Mode::Dock {
        let edge_labels: Vec<&str> = EDGES.iter().map(|(l, _)| *l).collect();
        arrangement.push(ui::row_tight("Dock edge", None, ui::pills(&edge_labels, index_of(&EDGES, &config.dock_edge), false, Msg::DockEdge)));
    }
    let fps_labels: Vec<&str> = FPS_CHOICES.iter().map(|i| FPS_LABELS[*i]).collect();
    let fps_selected = FPS_CHOICES.iter().position(|i| FPS[*i] == config.fps);
    arrangement.push(ui::row_tight(
        "Capture frame rate",
        Some(fps_help(config.fps)),
        ui::pills(&fps_labels, fps_selected, true, |i| Msg::Fps(FPS_CHOICES[i])),
    ));

    let vis_labels: Vec<&str> = VISIBILITIES.iter().map(|(l, _)| *l).collect();
    let visibility = vec![
        ui::row_tight("Show thumbnails", None, ui::pills(&vis_labels, index_of(&VISIBILITIES, &config.visibility), false, Msg::Visibility)),
        ui::row_tight(
            "Hide the focused client's own thumbnail",
            Some("Avoids a thumbnail of the window you are already looking at."),
            ui::toggle(config.hide_active, Msg::HideActive),
        ),
        ui::row_tight("Snap to a 16 px grid while dragging", Some("Keeps rows tidy without fiddling."), ui::toggle(config.snap_grid, Msg::SnapGrid)),
        ui::row_tight("Snap flush against other thumbnails", Some("Thumbnails stick edge to edge."), ui::toggle(config.snap_edges, Msg::SnapEdges)),
    ];

    let prefix_labels: Vec<&str> = PREFIX_CHOICES.iter().map(|i| PREFIXES[*i].0).collect();
    let prefix_selected = prefix_index(&config.shortcuts.focus_prefix).and_then(|i| PREFIX_CHOICES.iter().position(|c| *c == i));
    let prefix = ui::row(
        "Modifier prefix",
        Some("Applies to every Yutani shortcut below."),
        ui::pills(&prefix_labels, prefix_selected, false, |i| Msg::Prefix(PREFIX_CHOICES[i])),
    );
    let mut grid = Column::new().width(Length::Fill).spacing(9);
    for (what, combo) in shortcut_chips(&config.shortcuts) {
        grid = grid.push(
            Row::new()
                .width(Length::Fill)
                .align_y(Alignment::Center)
                .push(ui::text(what, ui::ROW_LABEL, Weight::Normal, ui::Role::Ink))
                .push(widget::space().width(Length::Fill))
                .push(ui::chip(combo)),
        );
    }
    let chips = widget::container(grid).width(Length::Fill).padding(ui::ROW_PAD).into();

    pane(
        "Behaviour",
        "Where thumbnails sit, when they appear, and the keys that drive them.",
        vec![
            ui::section("Arrangement", ui::card(arrangement)),
            ui::section("Visibility & dragging", ui::card(visibility)),
            ui::section("Keyboard shortcuts", ui::card(vec![prefix, chips])),
        ],
    )
}

// ---- Layouts ----------------------------------------------------------------------

fn layout_row<'a>(state: &'a State, summary: &'a layout::LayoutSummary) -> Element<'a, Msg> {
    let name = summary.name.as_str();
    let applied = state.applied_layout.as_deref() == Some(name);
    let renaming = state.renaming.as_ref().filter(|(n, _)| n == name);
    let confirming = state.confirm_delete.as_deref() == Some(name);
    let menu_open = state.layout_menu.as_deref() == Some(name);

    let mut row = Row::new().width(Length::Fill).spacing(12).align_y(Alignment::Center).push(ui::dot(applied, ui::Tint::Success, ui::DOT_PX));
    if let Some((_, draft)) = renaming {
        row = row.push(
            widget::text_input("Layout name", draft.as_str())
                .on_input(Msg::LayoutRenameDraft)
                .on_submit(|_| Msg::LayoutRenameCommit)
                .size(ui::ROW_LABEL)
                .width(Length::Fill),
        );
        row = row.push(ui::standard_button("Cancel", Some(Msg::LayoutCancel)));
        row = row.push(ui::accent_button("Rename", (!draft.trim().is_empty()).then_some(Msg::LayoutRenameCommit)));
    } else {
        let mut title = Row::new().spacing(8).align_y(Alignment::Center).push(ui::text(name, ui::LAYOUT_NAME, Weight::Medium, ui::Role::Ink));
        if applied {
            title = title.push(ui::text("applied", ui::LAYOUT_META, Weight::Normal, ui::Role::Success));
        }
        row = row.push(
            Column::new()
                .width(Length::Fill)
                .spacing(2)
                .push(title)
                .push(ui::mono(summary.meta(), ui::LAYOUT_META, Weight::Normal, ui::Role::Tertiary)),
        );
        if confirming {
            row = row.push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(ui::text("Delete this layout?", ui::VALUE, Weight::Normal, ui::Role::Secondary))
                    .push(ui::standard_button("Cancel", Some(Msg::LayoutCancel)))
                    .push(ui::destructive_button("Delete", Some(Msg::Delete(summary.name.clone())))),
            );
        } else {
            let apply = if applied {
                ui::standard_button("Re-apply", Some(Msg::Apply(summary.name.clone())))
            } else {
                ui::accent_button("Apply", Some(Msg::Apply(summary.name.clone())))
            };
            row = row.push(Row::new().spacing(6).align_y(Alignment::Center).push(apply).push(ui::glyph_button("⋯", 30.0, Some(Msg::LayoutMenu(summary.name.clone())))));
        }
    }
    let mut col = Column::new().width(Length::Fill).push(widget::container(row).width(Length::Fill).padding(ui::LAYOUT_ROW_PAD));
    if menu_open {
        // An inline menu under the row: Rename…, Duplicate, Delete….
        let menu = widget::container(
            Column::new()
                .width(Length::Fixed(168.0))
                .spacing(1)
                .push(ui::menu_item("Rename…", false, Msg::LayoutRenameStart(summary.name.clone())))
                .push(ui::menu_item("Duplicate", false, Msg::LayoutDuplicate(summary.name.clone())))
                .push(ui::hairline())
                .push(ui::menu_item("Delete…", true, Msg::LayoutDeleteAsk(summary.name.clone()))),
        )
        .padding(5)
        .class(ui::inner_class());
        col = col.push(
            widget::container(Row::new().width(Length::Fill).push(widget::space().width(Length::Fill)).push(menu))
                .width(Length::Fill)
                .padding([0, 14, 12, 14]),
        );
    }
    let class = if applied { ui::panel_class(ui::Tint::Success, 0.0) } else { cosmic::theme::Container::Transparent };
    widget::container(col).width(Length::Fill).class(class).into()
}

fn layouts_page<'a>(state: &'a State, config: &'a Config, thumbs_shown: bool) -> Element<'a, Msg> {
    let mut sections: Vec<Element<'a, Msg>> = Vec::new();
    // Arrange (opacity-and-centre spec §2): one button, with its blocker as
    // the help line when it cannot act.
    let blocker = centre_blocker(config.mode, thumbs_shown);
    let help = blocker.unwrap_or("Moves the floating thumbnails on each monitor so the group sits in the middle of the screen. Left-right positions are kept.");
    sections.push(ui::section(
        "Arrange",
        ui::card(vec![ui::row("Centre vertically", Some(help), ui::standard_button("Centre vertically", blocker.is_none().then_some(Msg::CentreVertically)))]),
    ));
    if let Some(error) = state.layout_error.as_deref() {
        sections.push(ui::panel(
            ui::Tint::Destructive,
            Column::new()
                .spacing(10)
                .push(ui::text("current.ron cannot be read", ui::ROW_LABEL, Weight::Medium, ui::Role::Ink))
                .push(ui::prose(error, ui::ROW_HELP, ui::Role::Secondary))
                .push(ui::prose(
                    "Thumbnail positions are not being saved: the file is never overwritten while it cannot be read. \
                     Fix or delete it and saving resumes by itself — press Re-check to confirm.",
                    ui::ROW_HELP,
                    ui::Role::Secondary,
                ))
                .push(Row::new().push(ui::standard_button("Re-check", Some(Msg::Recheck)))),
        ));
    }
    let mut rows: Vec<Element<'_, Msg>> = state.layouts.iter().map(|s| layout_row(state, s)).collect();
    if rows.is_empty() {
        rows.push(
            widget::container(ui::prose(
                "No saved layouts yet. Arrange the thumbnails, name the arrangement below and save it.",
                ui::BODY,
                ui::Role::Secondary,
            ))
            .width(Length::Fill)
            .padding(ui::LAYOUT_ROW_PAD)
            .into(),
        );
    }
    let named = !state.name_field.trim().is_empty();
    rows.push(
        widget::container(
            Row::new()
                .width(Length::Fill)
                .spacing(9)
                .align_y(Alignment::Center)
                .push(
                    widget::text_input("Name this arrangement", state.name_field.as_str())
                        .on_input(Msg::Name)
                        .on_submit(|_| Msg::SaveAs)
                        .size(ui::ROW_LABEL)
                        .width(Length::Fill),
                )
                .push(ui::primary_button("Save current arrangement", named.then_some(Msg::SaveAs))),
        )
        .width(Length::Fill)
        .padding(ui::LAYOUT_FOOTER_PAD)
        .into(),
    );
    sections.push(ui::card(rows));
    sections.push(ui::info_note(
        "A layout that names a monitor you no longer have is applied to the primary display instead. Nothing is lost — \
         reconnect the monitor and apply it again.",
    ));
    pane("Layouts", "Saved thumbnail arrangements. Applying one moves every thumbnail at once.", sections)
}

// ---- Steam ----------------------------------------------------------------------

/// The launch options EVE needs in Steam. Nothing here is editable and
/// nothing is stored: the page exists so the line can be read and copied
/// without hunting through the README.
fn steam_page(state: &State) -> Element<'_, Msg> {
    let steps = [
        "In Steam, right-click EVE Online → Properties → General.",
        "Paste the line below into Launch Options.",
        "Close Properties and launch each account as usual.",
    ];
    let mut list = Column::new().width(Length::Fill).spacing(9);
    for (i, step) in steps.iter().enumerate() {
        list = list.push(Row::new().spacing(10).align_y(Alignment::Start).push(ui::step_chip(i + 1)).push(ui::prose(*step, ui::BODY, ui::Role::Ink)));
    }
    let steps = widget::container(list).width(Length::Fill).padding([14, 16, 10, 16]).into();

    let command = launch_command(state.steam_full_path, &state.exe_path);
    let copy_label = if state.copied { "Copied" } else { "Copy" };
    let copy = if state.copied {
        widget::button::custom(ui::text(copy_label, ui::BUTTON, Weight::Medium, ui::Role::OnAccent))
            .height(Length::Fixed(ui::SMALL_BUTTON_H))
            .padding([0, 13])
            .class(cosmic::theme::Button::Suggested)
            .into()
    } else {
        ui::accent_button(copy_label, Some(Msg::CopySteamArgs))
    };
    let code = widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(12)
            .align_y(Alignment::Center)
            .push(widget::container(ui::mono(command, ui::CODE, Weight::Normal, ui::Role::Ink)).width(Length::Fill))
            .push(copy),
    )
    .width(Length::Fill)
    .padding(ui::CODE_PAD)
    .class(ui::sunken_class());
    let path_toggle = widget::container(
        Row::new()
            .width(Length::Fill)
            .spacing(11)
            .align_y(Alignment::Center)
            .push(ui::label_block("Steam can't find yutani", Some("Flatpak Steam, or a session without your shell's PATH. Uses the full binary path instead.")))
            .push(ui::toggle(state.steam_full_path, Msg::SteamFullPath)),
    )
    .width(Length::Fill)
    .padding(ui::INNER_PAD)
    .class(ui::inner_class());
    let block = widget::container(Column::new().width(Length::Fill).spacing(10).push(code).push(path_toggle)).width(Length::Fill).padding([14, 16]).into();

    pane(
        "Steam",
        "One-time setup so Steam launches EVE through Yutani.",
        vec![
            ui::card(vec![steps, block]),
            ui::info_note("Launch each EVE account as usual afterwards — Yutani picks up every client automatically and gives it the next free hotkey."),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `State` that has not touched the filesystem: `State::new` refreshes
    /// from the real config directory, which a unit test must not read.
    fn test_state() -> State {
        State {
            window: SurfaceId::unique(),
            pages: segmented_button::SingleSelectModel::default(),
            config_error: None,
            layouts: Vec::new(),
            applied_layout: None,
            name_field: String::new(),
            layout_menu: None,
            renaming: None,
            confirm_delete: None,
            active_border_field: String::new(),
            inactive_border_field: String::new(),
            note: None,
            layout_error: None,
            characters: Default::default(),
            copy_phase: CopyPhase::Idle,
            tunnel: Default::default(),
            uninstall_confirm: false,
            copied: false,
            steam_full_path: false,
            exe_path: "/usr/local/bin/yutani".to_string(),
        }
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The layout warning is whatever `current.ron` says *now*: a file the
    /// user fixed (or deleted) by hand stops the warning, because saving
    /// resumes at the same moment.
    #[test]
    fn the_layout_warning_reads_the_current_ron_it_is_given() {
        let dir = tmpdir("settings-layout-error");
        let path = dir.join("current.ron");
        let mut state = test_state();

        // No file yet: nothing is wrong, positions will be saved.
        state.refresh_layout_from(&path);
        assert_eq!(state.layout_error, None);

        // It parses.
        layout::Layout::default().save_to(&path).unwrap();
        state.refresh_layout_from(&path);
        assert_eq!(state.layout_error, None);

        // Hand-edited into something unparseable.
        std::fs::write(&path, "(thumbs: ").unwrap();
        state.refresh_layout_from(&path);
        assert!(state.layout_error.as_deref().unwrap().contains("cannot parse"));

        // Deleted: back to "nothing is wrong".
        std::fs::remove_file(&path).unwrap();
        state.refresh_layout_from(&path);
        assert_eq!(state.layout_error, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }


    /// A note left by an earlier action is about the page that is going
    /// away, or about a file that has just been re-read: either way it is
    /// stale and must not linger.
    #[test]
    fn switching_page_or_rechecking_drops_a_stale_note() {
        let mut state = test_state();
        let mut entity = None;
        state.pages.insert().text("Display").data(Page::Display).with_id(|e| entity = Some(e));
        assert!(clears_note(&Msg::Page(entity.unwrap())));
        assert!(clears_note(&Msg::Recheck));
        assert!(!clears_note(&Msg::SaveAs));
        assert!(!clears_note(&Msg::Commit));
    }

    /// The one string this page exists to hand over. Pinned exactly: a typo
    /// in Steam's launch options is a game that starts outside Yutani (or
    /// not at all), and the user cannot see which word is wrong.
    #[test]
    fn the_steam_launch_arguments_are_exactly_what_eve_needs() {
        assert_eq!(
            yutani::STEAM_LAUNCH_ARGS,
            "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 yutani launch -- %command%"
        );
        // The alternative for a Steam that cannot find `yutani` on PATH is
        // the same line with the documented install path spelled out.
        assert_eq!(
            yutani::STEAM_LAUNCH_ARGS_ABSOLUTE,
            "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%"
        );
        assert_eq!(
            yutani::STEAM_LAUNCH_ARGS_ABSOLUTE,
            yutani::STEAM_LAUNCH_ARGS.replace(" yutani launch", " /usr/local/bin/yutani launch")
        );
    }

    /// The tab strip is built from `PAGES`, so the list is the window: a
    /// page missing from it has no tab, and a tab with no page cannot be
    /// rendered.
    #[test]
    fn a_page_can_be_named_over_ipc() {
        assert_eq!(Page::from_name("layouts"), Some(Page::Layouts));
        assert_eq!(Page::from_name(" Behaviour "), Some(Page::Behavior));
        assert_eq!(Page::from_name("behavior"), Some(Page::Behavior));
        assert_eq!(Page::from_name("nope"), None);
    }

    #[test]
    fn every_page_has_a_tab_in_the_documented_order() {
        assert_eq!(labels(&PAGES), vec!["Display", "Behavior", "Layouts", "Characters", "Tunnel", "Steam"]);
        // The handoff's three frame rates and three prefixes are offered,
        // and each maps back into the full tables.
        assert_eq!(FPS_CHOICES.map(|i| FPS[i]), [15, 30, 60]);
        assert_eq!(PREFIX_CHOICES.map(|i| PREFIXES[i].0), ["Ctrl + Alt", "Super", "Ctrl + Shift"]);
        for page in [Page::Display, Page::Behavior, Page::Layouts, Page::Characters, Page::Tunnel, Page::Steam] {
            assert!(page_index(page).is_some(), "{page:?}");
        }
        assert_eq!(page_index(Page::Characters), Some(page_index(Page::Layouts).unwrap() + 1));
        // A `.conf` dropped on the window selects this tab by index, so the
        // index has to be the one the tab strip actually has.
        assert_eq!(page_index(Page::Tunnel), Some(page_index(Page::Characters).unwrap() + 1));
        assert_eq!(page_index(Page::Steam), Some(PAGES.len() - 1), "Steam stays last");
    }

    /// Copying writes the clipboard and leaves "copied" behind; it is not a
    /// config field, not a slider, and must not be swallowed by the
    /// note-clearing rule — the note is the only feedback the press gives.
    #[test]
    fn copying_the_steam_arguments_is_not_a_config_field_and_keeps_its_note() {
        let mut c = Config::default();
        assert_eq!(apply_config_field(&mut c, &Msg::CopySteamArgs), Ok(false));
        // The Characters page works on EVE's files, not on `config.ron`:
        // none of its messages may report a config change either.
        assert_eq!(apply_config_field(&mut c, &Msg::SourceCharacter(1)), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::CopyAccount(true)), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::CopyCharacters), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::RestoreBackup), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::Names(Names::new(), None)), Ok(false));
        // The Tunnel page drives systemd and `/etc/yutani`; not one of its
        // messages is a `config.ron` field either, so none of them may
        // report a config change (or be written back to the file).
        assert_eq!(apply_config_field(&mut c, &Msg::FilesDropped(vec![PathBuf::from("/tmp/x.conf")])), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::TunnelConfPath("/tmp/x.conf".into())), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::BrowseTunnelConf), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::TunnelConfChosen(None)), Ok(false));
        assert_eq!(
            apply_config_field(&mut c, &Msg::TunnelConfChosen(Some(PathBuf::from("/tmp/x.conf")))),
            Ok(false)
        );
        assert_eq!(apply_config_field(&mut c, &Msg::InstallTunnel), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::UninstallTunnel), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::TunnelConnect), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::TunnelDisconnect), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::RefreshTunnel), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::TunnelStatus(Box::default())), Ok(false));
        for action in [TunnelAction::Install, TunnelAction::Uninstall, TunnelAction::Connect, TunnelAction::Disconnect] {
            assert_eq!(apply_config_field(&mut c, &Msg::TunnelDone(action, Ok(()))), Ok(false));
            assert_eq!(apply_config_field(&mut c, &Msg::TunnelDone(action, Err("no".into()))), Ok(false));
        }
        assert_eq!(c, Config::default());
        assert!(!clears_note(&Msg::CopySteamArgs));
        assert!(!is_live_only(&Msg::CopySteamArgs));
    }

    #[test]
    fn colour_fields_accept_hex_and_clearing_the_active_one_means_theme_accent() {
        assert_eq!(parse_optional_color("  "), Ok(None));
        assert_eq!(parse_optional_color(" #ff8800 "), Ok(Some("#ff8800".to_string())));
        assert_eq!(parse_optional_color("#00000080"), Ok(Some("#00000080".to_string())));
        assert!(parse_optional_color("red").is_err());
        assert_eq!(parse_required_color("#404040"), Ok("#404040".to_string()));
        assert!(parse_required_color("").is_err());
        assert!(parse_required_color("#gg0000").is_err());
    }

    #[test]
    fn display_fields_land_in_the_config_and_say_whether_it_changed() {
        let mut c = Config::default();
        assert_eq!(apply_config_field(&mut c, &Msg::ThumbWidth(320)), Ok(true));
        assert_eq!(c.thumb_width, 320);
        // The same value again is not a change, so nothing is applied or written.
        assert_eq!(apply_config_field(&mut c, &Msg::ThumbWidth(320)), Ok(false));
        // Out-of-range values from a misbehaving control are clamped, never stored raw.
        assert_eq!(apply_config_field(&mut c, &Msg::ThumbWidth(9999)), Ok(true));
        assert_eq!(c.thumb_width, 1600);
        assert_eq!(apply_config_field(&mut c, &Msg::BorderPx(99)), Ok(true));
        assert_eq!(c.border_px, 16);
        assert_eq!(apply_config_field(&mut c, &Msg::CornerRadius(99)), Ok(true));
        assert_eq!(c.corner_radius, 64);
        assert_eq!(apply_config_field(&mut c, &Msg::Zoom(9.0)), Ok(true));
        assert_eq!(c.zoom_factor, 4.0);
        // The slider's f32 arithmetic is rounded to the step, so the file
        // never gets a 1.3000001.
        assert_eq!(apply_config_field(&mut c, &Msg::Zoom(1.3000001)), Ok(true));
        assert_eq!(c.zoom_factor, 1.3);
        // Opacity spec §1.5.
        assert_eq!(apply_config_field(&mut c, &Msg::Opacity(80)), Ok(true));
        assert_eq!(c.thumb_opacity, 80);
        assert_eq!(apply_config_field(&mut c, &Msg::Opacity(80)), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::Opacity(5)), Ok(true));
        assert_eq!(c.thumb_opacity, 20);
        assert_eq!(apply_config_field(&mut c, &Msg::Opacity(200)), Ok(true));
        assert_eq!(c.thumb_opacity, 100);
        assert_eq!(apply_config_field(&mut c, &Msg::ShowNames(false)), Ok(true));
        assert!(!c.show_names);
        assert_eq!(apply_config_field(&mut c, &Msg::ActiveBorder("#ff8800".into())), Ok(true));
        assert_eq!(c.active_border.as_deref(), Some("#ff8800"));
        assert_eq!(apply_config_field(&mut c, &Msg::ActiveBorder(String::new())), Ok(true));
        assert_eq!(c.active_border, None);
        // A bad value is a note, not a write: the config is untouched.
        let before = c.clone();
        assert!(apply_config_field(&mut c, &Msg::InactiveBorder("nope".into())).is_err());
        assert_eq!(c, before);
        // Window-level messages are not config fields.
        assert_eq!(apply_config_field(&mut c, &Msg::Close), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::Commit), Ok(false));
    }

    #[test]
    fn sliders_apply_live_but_only_their_release_writes_the_file() {
        assert!(is_live_only(&Msg::ThumbWidth(300)));
        assert!(is_live_only(&Msg::Zoom(1.5)));
        assert!(is_live_only(&Msg::Opacity(80)));
        assert!(!is_live_only(&Msg::BorderPx(2)));
        assert!(!is_live_only(&Msg::ShowNames(false)));
        assert!(!is_live_only(&Msg::ActiveBorder("#ff8800".into())));
    }

    #[test]
    fn every_enum_value_has_a_dropdown_entry() {
        for m in [Mode::Floating, Mode::Dock] {
            assert!(index_of(&MODES, &m).is_some(), "{m:?}");
        }
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            assert!(index_of(&EDGES, &e).is_some(), "{e:?}");
        }
        for v in [Visibility::Always, Visibility::EveFocusedOnly] {
            assert!(index_of(&VISIBILITIES, &v).is_some(), "{v:?}");
        }
        assert_eq!(FPS.len(), FPS_LABELS.len());
        assert!(FPS.contains(&Config::default().fps));
        // The default prefix is offered, and round-trips through the table.
        let default_prefix = Config::default().shortcuts.focus_prefix;
        let i = prefix_index(&default_prefix).expect("the default prefix is in the list");
        assert_eq!(prefix_at(i), Some(default_prefix));
        assert_eq!(prefix_at(PREFIXES.len()), None);
        // A hand-edited chord the list does not offer simply selects nothing.
        assert_eq!(prefix_index(&[Modifier::Shift]), None);
    }

    #[test]
    fn behavior_fields_land_in_the_config() {
        let mut c = Config::default();
        let floating = index_of(&MODES, &Mode::Floating).unwrap();
        assert_eq!(apply_config_field(&mut c, &Msg::Mode(floating)), Ok(true));
        assert_eq!(c.mode, Mode::Floating);
        let left = index_of(&EDGES, &Edge::Left).unwrap();
        assert_eq!(apply_config_field(&mut c, &Msg::DockEdge(left)), Ok(true));
        assert_eq!(c.dock_edge, Edge::Left);
        assert_eq!(apply_config_field(&mut c, &Msg::Fps(0)), Ok(true));
        assert_eq!(c.fps, 10);
        let always = index_of(&VISIBILITIES, &Visibility::Always).unwrap();
        assert_eq!(apply_config_field(&mut c, &Msg::Visibility(always)), Ok(true));
        assert_eq!(c.visibility, Visibility::Always);
        assert_eq!(apply_config_field(&mut c, &Msg::HideActive(true)), Ok(true));
        assert_eq!(apply_config_field(&mut c, &Msg::SnapGrid(false)), Ok(true));
        assert_eq!(apply_config_field(&mut c, &Msg::SnapEdges(false)), Ok(true));
        assert!(c.hide_active && !c.snap_grid && !c.snap_edges);
        assert_eq!(apply_config_field(&mut c, &Msg::Prefix(1)), Ok(true));
        assert_eq!(c.shortcuts.focus_prefix, prefix_at(1).unwrap());
        // An index no table entry has is a note, not a panic and not a write.
        let before = c.clone();
        assert!(apply_config_field(&mut c, &Msg::Fps(9)).is_err());
        assert!(apply_config_field(&mut c, &Msg::Mode(9)).is_err());
        assert!(apply_config_field(&mut c, &Msg::Prefix(9)).is_err());
        assert_eq!(c, before);
    }

    /// The broken-config guard is whatever `config.ron` says *now*: the
    /// window's cached answer is up to a watcher debounce old.
    #[test]
    fn the_broken_config_guard_reads_the_file_it_is_given() {
        let dir = std::env::temp_dir().join(format!("yutani-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        let mut state = test_state();
        // No file at all: defaults are in force and writing is fine.
        state.refresh_from(&path);
        assert_eq!(state.config_error, None);
        std::fs::write(&path, "(fps: 30)").unwrap();
        state.refresh_from(&path);
        assert_eq!(state.config_error, None);
        std::fs::write(&path, "(fps: ").unwrap();
        state.refresh_from(&path);
        assert!(state.config_error.as_deref().unwrap().contains("cannot parse"));
        // And it clears again once the user fixes the file.
        std::fs::write(&path, "(fps: 60)").unwrap();
        state.refresh_from(&path);
        assert_eq!(state.config_error, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The handoff's copy, derived from real state: the frame-rate help,
    /// the shortcut chips from the prefix, the launch line, the confirm and
    /// done sentences, the Tunnel sublabel.
    #[test]
    fn the_derived_copy_follows_the_handoff() {
        assert!(fps_help(60).starts_with("Smoothest"));
        assert!(fps_help(30).starts_with("A good balance"));
        assert!(fps_help(15).starts_with("Cheapest"));
        let chips = shortcut_chips(&Config::default().shortcuts);
        assert_eq!(chips[0], ("Focus client 1 – 9", "Ctrl + Alt + 1 … 9".to_string()));
        assert_eq!(chips[1].1, "Ctrl + Alt + →");
        assert_eq!(chips[2].1, "Ctrl + Alt + ←");
        assert_eq!(chips[3], ("Show / hide thumbnails", "Ctrl + Alt + T".to_string()));
        assert_eq!(launch_command(false, "/opt/yutani"), yutani::STEAM_LAUNCH_ARGS);
        assert_eq!(launch_command(true, "/opt/yutani"), "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /opt/yutani launch -- %command%");
        assert_eq!(
            copy_confirm_text("Ishukone", 3, false),
            "This replaces the interface settings of 3 characters with Ishukone’s. Their current files are backed up first, and can be restored below."
        );
        assert!(copy_confirm_text("A", 1, true).starts_with("This replaces the interface and account settings of 1 character with A’s."));
        assert_eq!(copy_done_text("Ishukone", 3), "Ishukone’s settings copied to 3 characters. Restart any running client to see them.");
        let mut t = TunnelStatus::default();
        assert_eq!(tunnel_sublabel(None), "Not installed");
        assert_eq!(tunnel_sublabel(Some(&t)), "Not installed");
        t.installed = true;
        assert_eq!(tunnel_sublabel(Some(&t)), "Installed · idle");
        t.connected = true;
        t.location = "London".into();
        assert_eq!(tunnel_sublabel(Some(&t)), "Connected · London");
    }

    /// Reset frame: the handoff's defaults, and the accent for the focused
    /// border (today's default). Choosing a folder is a config field.
    #[test]
    fn reset_frame_and_the_profile_folder_are_config_fields() {
        let mut c = Config { border_px: 5, corner_radius: 20, active_border: Some("#ff0000".into()), inactive_border: "#123456".into(), ..Config::default() };
        assert_eq!(apply_config_field(&mut c, &Msg::ResetFrame), Ok(true));
        assert_eq!((c.border_px, c.corner_radius, c.active_border.as_deref(), c.inactive_border.as_str()), (1, 8, None, Config::default().inactive_border.as_str()));
        assert_eq!(apply_config_field(&mut c, &Msg::ResetFrame), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::ProfileDirChosen(Some(PathBuf::from("/mnt/eve/settings_Default")))), Ok(true));
        assert_eq!(c.eve_settings_dir.as_deref(), Some("/mnt/eve/settings_Default"));
        assert_eq!(apply_config_field(&mut c, &Msg::ProfileDirChosen(None)), Ok(false));
        // Transient UI messages are not config fields.
        for m in [Msg::AskCopy, Msg::CancelCopy, Msg::AskUninstall, Msg::CancelUninstall, Msg::CopyReset, Msg::SteamFullPath(true), Msg::LayoutMenu("x".into()), Msg::LayoutCancel, Msg::CentreVertically] {
            assert_eq!(apply_config_field(&mut c, &m), Ok(false));
        }
    }

    /// Spec §2.1: disabled, with the reason, in dock mode and with nothing
    /// shown.
    #[test]
    fn centre_vertically_is_disabled_in_dock_mode_and_with_nothing_shown() {
        assert_eq!(centre_blocker(Mode::Dock, true), Some("The dock is already centred along its edge."));
        assert_eq!(centre_blocker(Mode::Floating, false), Some("No thumbnails are showing."));
        assert_eq!(centre_blocker(Mode::Floating, true), None);
    }

    /// libcosmic draws the header bar itself, so the toplevel must be
    /// undecorated; the handoff's 900×724, resizable down to 760×560.
    #[test]
    fn the_settings_window_is_undecorated_and_resizable() {
        let s = window_settings("io.github.yutani");
        assert_eq!(s.size, cosmic::iced::Size::new(900.0, 724.0));
        assert_eq!(s.min_size, Some(cosmic::iced::Size::new(760.0, 560.0)));
        assert!(s.resizable);
        assert!(!s.decorations);
        assert!(s.exit_on_close_request);
        assert_eq!(s.platform_specific.application_id, "io.github.yutani");
    }
}
