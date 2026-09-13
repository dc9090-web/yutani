//! The settings window (spec §6). Opened by `yutani settings` and by the
//! applet's Preferences… row, both through the IPC `settings` request; a
//! second request raises the window that is already open.
//!
//! Every change applies live (`App::apply_config`) and is written back to
//! `config.ron` — except while that file fails to parse, when the window
//! shows what is wrong and writes nothing at all (spec §9: "never overwrite
//! the user's file").

use std::path::Path;

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::iced::window::Id as SurfaceId;
use cosmic::widget;
use cosmic::widget::segmented_button;

use yutani::eve_settings::names::Names;

use crate::model::config::{Config, Edge, Mode, Modifier, Visibility, config_path, parse_color, resolve_keysym};
use crate::model::layout;

/// The pages of spec §6, plus the Steam one: the launch options EVE needs,
/// which are not a setting at all — they are a string to copy into Steam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Display,
    Behavior,
    Layouts,
    Characters,
    Steam,
}

/// The tab strip, in order. `State::new` builds the segmented control from
/// this, so the list *is* the window: a page missing here has no tab.
pub const PAGES: [(&str, Page); 5] = [
    ("Display", Page::Display),
    ("Behavior", Page::Behavior),
    ("Layouts", Page::Layouts),
    ("Characters", Page::Characters),
    ("Steam", Page::Steam),
];

pub const MODES: [(&str, Mode); 2] = [("Floating", Mode::Floating), ("Dock", Mode::Dock)];
pub const EDGES: [(&str, Edge); 4] =
    [("Top", Edge::Top), ("Bottom", Edge::Bottom), ("Left", Edge::Left), ("Right", Edge::Right)];
pub const FPS: [u32; 4] = [10, 15, 30, 60];
pub const FPS_LABELS: [&str; 4] = ["10", "15", "30", "60"];
pub const VISIBILITIES: [(&str, Visibility); 2] =
    [("Always", Visibility::Always), ("Only while EVE has focus", Visibility::EveFocusedOnly)];

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

/// A `next`/`prev` binding key: an xkb keysym name (what COSMIC's shortcut
/// file stores), distinct from the other one, and not a digit 1-9 —
/// `focus 1..9` takes those. The same rule `Config::validate` enforces,
/// repeated here so the field can refuse before anything is written: an
/// invalid name fails cosmic-comp's whole custom-shortcut map.
pub fn parse_shortcut_key(text: &str, other: &str) -> Result<String, String> {
    let key = text.trim();
    if key.is_empty() {
        return Err("a shortcut key cannot be empty".into());
    }
    if key.eq_ignore_ascii_case(other) {
        return Err(format!("{key:?} is already the other shortcut key"));
    }
    if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() && key != "0" {
        return Err("1-9 are taken by focus 1..9".into());
    }
    if resolve_keysym(key).is_none() {
        return Err(format!("{key:?} is not an xkb keysym name (try \"Right\", \"Tab\", \"a\", \"F12\")"));
    }
    Ok(key.to_string())
}

/// Whether the *Install shortcuts* button is pressable. It writes bindings
/// built from `config.shortcuts`, which only ever holds accepted values — so
/// while a key field is refused, pressing Install would install a binding
/// other than the one on screen. Disabled until both fields parse.
/// (*Uninstall* never reads the keys, so it stays pressable.)
pub fn install_enabled(next_field: &str, prev_field: &str) -> bool {
    parse_shortcut_key(next_field, prev_field).is_ok() && parse_shortcut_key(prev_field, next_field).is_ok()
}

/// Everything the settings window owns. `None` on `App` while it is closed.
pub struct State {
    pub window: SurfaceId,
    /// The tab strip ([`PAGES`]); its active entity carries a `Page`.
    pub pages: segmented_button::SingleSelectModel,
    /// Set when `config.ron` exists but does not parse: the Display and
    /// Behavior pages are replaced by the reason and nothing is written.
    /// The Layouts page still works — it writes layout files, not this one.
    pub config_error: Option<String>,
    /// Saved layout names, refreshed after every Layouts-page action.
    pub layouts: Vec<String>,
    /// The Save-as / Rename target.
    pub name_field: String,
    /// Text fields are kept as typed, so a half-typed value never reaches
    /// the config; they are seeded when the window opens and are not
    /// re-seeded by a later hand edit (which would eat what is being typed).
    pub active_border_field: String,
    pub inactive_border_field: String,
    pub next_field: String,
    pub prev_field: String,
    /// One line of feedback under the page.
    pub note: Option<String>,
    /// Set when `current.ron` exists but does not parse: auto-save is
    /// suspended until it does (spec §10), so every page says so and the
    /// Layouts page shows the reason.
    pub layout_error: Option<String>,
    /// The Characters page's own state (`super::characters`): EVE's files,
    /// not ours, so it is refreshed by `App::refresh_characters`.
    pub characters: super::characters::State,
}

impl State {
    pub fn new(window: SurfaceId, config: &Config) -> Self {
        let mut pages = segmented_button::SingleSelectModel::default();
        for (label, page) in PAGES {
            pages.insert().text(label).data(page);
        }
        pages.activate_position(0);
        let mut state = Self {
            window,
            pages,
            config_error: None,
            layouts: Vec::new(),
            name_field: String::new(),
            active_border_field: config.active_border.clone().unwrap_or_default(),
            inactive_border_field: config.inactive_border.clone(),
            next_field: config.shortcuts.next.clone(),
            prev_field: config.shortcuts.prev.clone(),
            note: None,
            layout_error: None,
            characters: Default::default(),
        };
        state.refresh();
        state
    }

    /// Re-read what lives outside `Config`: the saved layout names, whether
    /// `config.ron` currently parses, and whether `current.ron` does.
    pub fn refresh(&mut self) {
        self.layouts = layout::list_names();
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
    BorderPx(u32),
    CornerRadius(u32),
    ShowNames(bool),
    ActiveBorder(String),
    InactiveBorder(String),
    /// A tab was pressed.
    Page(segmented_button::Entity),
    Mode(usize),
    DockEdge(usize),
    Fps(usize),
    Visibility(usize),
    HideActive(bool),
    SnapGrid(bool),
    SnapEdges(bool),
    Prefix(usize),
    NextKey(String),
    PrevKey(String),
    InstallShortcuts,
    UninstallShortcuts,
    /// Put [`yutani::STEAM_LAUNCH_ARGS`] on the clipboard.
    CopySteamArgs,
    /// Characters page: dropdown index of the character to copy from.
    SourceCharacter(usize),
    /// Characters page: also copy the account-wide file.
    CopyAccount(bool),
    /// Characters page: dropdown index of the account to copy from.
    SourceAccount(usize),
    /// Characters page: do the copy (refused while a client is running).
    CopyCharacters,
    /// Characters page: put the newest backup back.
    RestoreBackup,
    /// Characters page: re-read the profile, the backups and the names.
    RefreshCharacters,
    /// A names lookup finished: what is known, and the error if any id is still unnamed.
    Names(Names, Option<String>),
    /// The Save-as / Rename name field.
    Name(String),
    SaveAs,
    Apply(String),
    Rename(String),
    Delete(String),
}

/// The settings window: an ordinary xdg-toplevel. Undecorated because
/// libcosmic draws the header bar itself (`widget::header_bar`), and
/// `exit_on_close_request: true` so the compositor's own close gesture
/// closes it and the app only reacts to `Closed`.
pub fn window_settings(app_id: &str) -> cosmic::iced::window::Settings {
    cosmic::iced::window::Settings {
        size: cosmic::iced::Size::new(620.0, 700.0),
        min_size: Some(cosmic::iced::Size::new(420.0, 380.0)),
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
    matches!(msg, Msg::ThumbWidth(_) | Msg::Zoom(_))
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
        Msg::BorderPx(v) => config.border_px = (*v).min(16),
        Msg::CornerRadius(v) => config.corner_radius = (*v).min(64),
        Msg::ShowNames(v) => config.show_names = *v,
        Msg::ActiveBorder(text) => config.active_border = parse_optional_color(text)?,
        Msg::InactiveBorder(text) => config.inactive_border = parse_required_color(text)?,
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
        Msg::NextKey(text) => {
            let key = parse_shortcut_key(text, &config.shortcuts.prev)?;
            config.shortcuts.next = key;
        }
        Msg::PrevKey(text) => {
            let key = parse_shortcut_key(text, &config.shortcuts.next)?;
            config.shortcuts.prev = key;
        }
        _ => return Ok(false),
    }
    Ok(*config != before)
}

/// The whole window: header bar, the page (or the broken-config notice),
/// and the note line.
pub fn view<'a>(
    state: &'a State,
    config: &'a Config,
    focused: bool,
    clients_running: bool,
) -> Element<'a, super::Msg> {
    let page = state.pages.active_data::<Page>().copied().unwrap_or(Page::Display);
    let body: Element<'a, Msg> = match (page, &state.config_error) {
        // Layout files are not `config.ron`; this page works either way.
        (Page::Layouts, _) => layouts_page(state),
        // Nor does the Steam page read or write anything: it is a fixed
        // string and a Copy button, and it is exactly the page someone
        // whose config is broken may still need.
        (Page::Steam, _) => steam_page(),
        // Nor is EVE's profile directory `config.ron`: this page copies
        // CCP's files and is just as usable while ours does not parse.
        (Page::Characters, _) => super::characters::view(&state.characters, clients_running),
        (_, Some(error)) => broken_config(error),
        (Page::Display, None) => display_page(state, config),
        (Page::Behavior, None) => behavior_page(state, config),
    };
    let mut notes: Vec<Element<'a, Msg>> = Vec::new();
    // A suspended auto-save is worth saying on every page, not only on the
    // Layouts one that explains it: the user is about to drag a thumbnail.
    if state.layout_error.is_some() {
        notes.push(
            widget::text::caption("current.ron cannot be read; thumbnail positions are not being saved.").into(),
        );
    }
    notes.push(match (&state.note, &state.config_error) {
        (Some(note), _) => widget::text::caption(note.as_str()).into(),
        // Nothing is written while the file cannot be read, so the usual
        // "changes are written to …" line would be a lie.
        (None, Some(_)) => widget::text::caption(
            "config.ron is not written while it cannot be read. Saved layouts are separate files and still work.",
        )
        .into(),
        (None, None) => widget::text::caption(
            "Changes apply at once and are written to ~/.config/yutani/config.ron. \
             Comments and unknown keys in that file are not preserved.",
        )
        .into(),
    });
    let note: Element<'a, Msg> = widget::column::with_children(notes).spacing(4).into();
    // Three pages now, the longest of which does not fit 700 px.
    let scrolled: Element<'a, Msg> = widget::scrollable(body).height(Length::Fill).into();
    let tabs: Element<'a, Msg> =
        widget::segmented_control::horizontal(&state.pages).on_activate(Msg::Page).into();
    let page = widget::container(widget::column::with_children(vec![tabs, scrolled, note]).spacing(12))
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill);
    let content: Element<'a, Msg> = widget::container(widget::column::with_children(vec![
        widget::header_bar()
            .title("Yutani Settings")
            .on_close(Msg::Close)
            .on_drag(Msg::Drag)
            .focused(focused)
            .into(),
        page.into(),
    ]))
    .class(cosmic::theme::Container::WindowBackground)
    .width(Length::Fill)
    .height(Length::Fill)
    .into();
    content.map(super::Msg::Settings)
}

/// Shown instead of the pages while `config.ron` does not parse: nothing is
/// editable, because the only safe thing to do with the user's broken file
/// is leave it alone (spec §9/§10).
fn broken_config(error: &str) -> Element<'_, Msg> {
    widget::settings::view_column(vec![
        widget::settings::section()
            .title("config.ron cannot be read")
            .add(widget::text::body(error))
            .add(widget::text::body(
                "Yutani is running with default settings. Nothing here can be changed until the file parses — \
                 it is never overwritten. Fix or delete it, then press Re-check.",
            ))
            .add(widget::settings::item_row(vec![
                widget::button::standard("Re-check").on_press(Msg::Recheck).into(),
            ]))
            .into(),
    ])
    .into()
}

fn display_page<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let width = widget::settings::item(
        format!("Thumbnail width: {} px", config.thumb_width),
        widget::slider(80..=1600, config.thumb_width, Msg::ThumbWidth)
            .step(10u32)
            .on_release(Msg::Commit)
            .width(Length::Fixed(260.0)),
    );
    let zoom = widget::settings::item(
        format!("Hover zoom: {:.1}x", config.zoom_factor),
        widget::slider(1.0..=4.0, config.zoom_factor, Msg::Zoom)
            .step(0.1f32)
            .on_release(Msg::Commit)
            .width(Length::Fixed(260.0)),
    );
    let names =
        widget::settings::item("Show character names", widget::toggler(config.show_names).on_toggle(Msg::ShowNames));
    let border = widget::settings::item(
        "Border width (px)",
        widget::spin_button(config.border_px.to_string(), config.border_px, 1, 0, 16, Msg::BorderPx),
    );
    let radius = widget::settings::item(
        "Corner radius (px)",
        widget::spin_button(config.corner_radius.to_string(), config.corner_radius, 1, 0, 64, Msg::CornerRadius),
    );
    let active = widget::settings::item(
        "Active border colour",
        widget::text_input("theme accent", state.active_border_field.as_str())
            .on_input(Msg::ActiveBorder)
            .width(Length::Fixed(160.0)),
    );
    let inactive = widget::settings::item(
        "Inactive border colour",
        widget::text_input("#404040", state.inactive_border_field.as_str())
            .on_input(Msg::InactiveBorder)
            .width(Length::Fixed(160.0)),
    );
    widget::settings::view_column(vec![
        widget::settings::section().title("Thumbnails").add(width).add(zoom).add(names).into(),
        widget::settings::section().title("Frame").add(border).add(radius).add(active).add(inactive).into(),
    ])
    .into()
}

fn behavior_page<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let mode =
        widget::settings::item("Mode", widget::dropdown(labels(&MODES), index_of(&MODES, &config.mode), Msg::Mode));
    let edge = widget::settings::item(
        "Dock edge",
        widget::dropdown(labels(&EDGES), index_of(&EDGES, &config.dock_edge), Msg::DockEdge),
    );
    let fps = widget::settings::item(
        "Capture frame rate",
        widget::dropdown(FPS_LABELS.to_vec(), FPS.iter().position(|f| *f == config.fps), Msg::Fps),
    );
    let visibility = widget::settings::item(
        "Show thumbnails",
        widget::dropdown(labels(&VISIBILITIES), index_of(&VISIBILITIES, &config.visibility), Msg::Visibility),
    );
    let hide_active = widget::settings::item(
        "Hide the focused client's own thumbnail",
        widget::toggler(config.hide_active).on_toggle(Msg::HideActive),
    );
    let snap_grid = widget::settings::item(
        "Snap to a 32 px grid while dragging",
        widget::toggler(config.snap_grid).on_toggle(Msg::SnapGrid),
    );
    let snap_edges = widget::settings::item(
        "Snap flush against other thumbnails",
        widget::toggler(config.snap_edges).on_toggle(Msg::SnapEdges),
    );
    let prefix = widget::settings::item(
        "Shortcut prefix",
        widget::dropdown(labels(&PREFIXES), prefix_index(&config.shortcuts.focus_prefix), Msg::Prefix),
    );
    let next = widget::settings::item(
        "Next client key",
        widget::text_input("Right", state.next_field.as_str()).on_input(Msg::NextKey).width(Length::Fixed(160.0)),
    );
    let prev = widget::settings::item(
        "Previous client key",
        widget::text_input("Left", state.prev_field.as_str()).on_input(Msg::PrevKey).width(Length::Fixed(160.0)),
    );
    let pressable = install_enabled(&state.next_field, &state.prev_field);
    let buttons = widget::settings::item_row(vec![
        widget::button::standard("Install shortcuts")
            .on_press_maybe(pressable.then_some(Msg::InstallShortcuts))
            .into(),
        widget::button::destructive("Uninstall shortcuts")
            .on_press(Msg::UninstallShortcuts)
            .into(),
    ]);
    widget::settings::view_column(vec![
        widget::settings::section().title("Arrangement").add(mode).add(edge).add(fps).into(),
        widget::settings::section().title("Visibility").add(visibility).add(hide_active).into(),
        widget::settings::section().title("Dragging").add(snap_grid).add(snap_edges).into(),
        widget::settings::section()
            .title("Keyboard shortcuts")
            .add(prefix)
            .add(next)
            .add(prev)
            .add(buttons)
            .into(),
    ])
    .into()
}

/// The launch options EVE needs in Steam. Nothing here is editable and
/// nothing is stored: the page exists so the line can be read and copied
/// without hunting through the README.
fn steam_page<'a>() -> Element<'a, Msg> {
    let line = widget::settings::item_row(vec![
        widget::text::monotext(yutani::STEAM_LAUNCH_ARGS).width(Length::Fill).into(),
        widget::button::standard("Copy").on_press(Msg::CopySteamArgs).into(),
    ]);
    widget::settings::view_column(vec![
        widget::settings::section()
            .title("EVE Online launch options")
            .add(widget::text::caption(
                "Launch arguments for EVE Online in Steam (Properties → Launch Options):",
            ))
            .add(line)
            .add(widget::text::caption(
                "This assumes Steam can find `yutani` on its PATH. If it cannot (a Flatpak Steam, or a \
                 session that did not inherit your shell's PATH), use the full path instead:",
            ))
            .add(widget::text::monotext(yutani::STEAM_LAUNCH_ARGS_ABSOLUTE))
            .into(),
    ])
    .into()
}

fn layouts_page(state: &State) -> Element<'_, Msg> {
    let named = !state.name_field.trim().is_empty();
    // Not fatal to this page — a named layout is a different file — but the
    // arrangement being saved under that name is the in-memory one, and the
    // auto-save behind it is suspended until `current.ron` parses again.
    let broken_layout = state.layout_error.as_ref().map(|error| {
        widget::settings::section()
            .title("current.ron cannot be read")
            .add(widget::text::body(error.as_str()))
            .add(widget::text::body(
                "Thumbnail positions are not being saved: the file is never overwritten while it cannot be read. \
                 Fix or delete it and saving resumes by itself — press Re-check to confirm.",
            ))
            .add(widget::settings::item_row(vec![
                widget::button::standard("Re-check").on_press(Msg::Recheck).into(),
            ]))
    });
    let mut saved = widget::settings::section().title("Saved layouts");
    if state.layouts.is_empty() {
        saved = saved.add(widget::text::body(
            "None yet. Arrange the thumbnails, type a name below and press Save current as….",
        ));
    }
    for name in &state.layouts {
        saved = saved.add(widget::settings::item(
            name.as_str(),
            widget::settings::item_row(vec![
                widget::button::standard("Apply").on_press(Msg::Apply(name.clone())).into(),
                widget::button::text("Rename").on_press_maybe(named.then(|| Msg::Rename(name.clone()))).into(),
                widget::button::destructive("Delete").on_press(Msg::Delete(name.clone())).into(),
            ]),
        ));
    }
    let save = widget::settings::section().title("Save the current arrangement").add(widget::settings::item_row(vec![
        widget::text_input("Layout name", state.name_field.as_str())
            .on_input(Msg::Name)
            .on_submit(|_| Msg::SaveAs)
            .width(Length::Fill)
            .into(),
        widget::button::suggested("Save current as…").on_press_maybe(named.then_some(Msg::SaveAs)).into(),
    ]));
    let mut rows: Vec<Element<'_, Msg>> = Vec::new();
    rows.extend(broken_layout.map(Into::into));
    rows.push(saved.into());
    rows.push(save.into());
    rows.push(
        widget::text::caption(
            "Rename uses the name typed in the field. Applying a layout copies it to current.ron and moves the \
             thumbnails; a layout that names a disconnected monitor lands on the primary one — and it replaces an \
             unreadable current.ron, because asking for it is asking for that.",
        )
        .into(),
    );
    widget::settings::view_column(rows).into()
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
            name_field: String::new(),
            active_border_field: String::new(),
            inactive_border_field: String::new(),
            next_field: String::new(),
            prev_field: String::new(),
            note: None,
            layout_error: None,
            characters: Default::default(),
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

    /// Install would write `config.shortcuts`, not what is in the fields, so
    /// pressing it while a field is refused would install a binding the
    /// window is not showing. The buttons are disabled instead.
    #[test]
    fn the_shortcut_buttons_are_disabled_while_either_key_field_is_refused() {
        assert!(install_enabled("Right", "Left"));
        assert!(install_enabled("  Tab  ", "Left"));
        assert!(!install_enabled("", "Left"));
        assert!(!install_enabled("Right", ""));
        assert!(!install_enabled("Rihgt", "Left"));
        assert!(!install_enabled("Right", "Lfet"));
        // The two fields naming the same key is refused from either side.
        assert!(!install_enabled("Left", "left"));
        assert!(!install_enabled("3", "Left"));
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
        assert!(!clears_note(&Msg::InstallShortcuts));
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
    fn every_page_has_a_tab_in_the_documented_order() {
        assert_eq!(labels(&PAGES), vec!["Display", "Behavior", "Layouts", "Characters", "Steam"]);
        for page in [Page::Display, Page::Behavior, Page::Layouts, Page::Characters, Page::Steam] {
            assert!(index_of(&PAGES, &page).is_some(), "{page:?}");
        }
        assert_eq!(index_of(&PAGES, &Page::Characters), Some(index_of(&PAGES, &Page::Layouts).unwrap() + 1));
        assert_eq!(index_of(&PAGES, &Page::Steam), Some(PAGES.len() - 1), "Steam stays last");
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
        assert_eq!(apply_config_field(&mut c, &Msg::SourceAccount(1)), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::CopyAccount(true)), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::CopyCharacters), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::RestoreBackup), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::RefreshCharacters), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::Names(Names::new(), None)), Ok(false));
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

    #[test]
    fn shortcut_keys_must_be_keysym_names_distinct_from_the_other_one() {
        assert_eq!(parse_shortcut_key("  Tab  ", "Left"), Ok("Tab".to_string()));
        assert_eq!(parse_shortcut_key("F12", "Left"), Ok("F12".to_string()));
        assert!(parse_shortcut_key("", "Left").is_err());
        assert!(parse_shortcut_key("left", "Left").unwrap_err().contains("other shortcut key"));
        assert!(parse_shortcut_key("3", "Left").unwrap_err().contains("focus"));
        assert!(parse_shortcut_key("Rihgt", "Left").unwrap_err().contains("keysym"));
        let mut c = Config::default();
        assert_eq!(apply_config_field(&mut c, &Msg::NextKey("Tab".into())), Ok(true));
        assert_eq!(c.shortcuts.next, "Tab");
        assert!(apply_config_field(&mut c, &Msg::PrevKey("tab".into())).is_err());
        assert_eq!(c.shortcuts.prev, "Left");
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

    /// libcosmic draws the header bar itself, so the toplevel must be
    /// undecorated; the minimum size keeps the widest settings row readable.
    #[test]
    fn the_settings_window_is_undecorated_and_resizable() {
        let s = window_settings("io.github.yutani");
        assert_eq!(s.size, cosmic::iced::Size::new(620.0, 700.0));
        assert_eq!(s.min_size, Some(cosmic::iced::Size::new(420.0, 380.0)));
        assert!(s.resizable);
        assert!(!s.decorations);
        assert!(s.exit_on_close_request);
        assert_eq!(s.platform_specific.application_id, "io.github.yutani");
    }
}
