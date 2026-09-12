//! The settings window (spec §6). Opened by `yutani settings` and by the
//! applet's Preferences… row, both through the IPC `settings` request; a
//! second request raises the window that is already open.
//!
//! Every change applies live (`App::apply_config`) and is written back to
//! `config.ron` — except while that file fails to parse, when the window
//! shows what is wrong and writes nothing at all (spec §9: "never overwrite
//! the user's file").

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::iced::window::Id as SurfaceId;
use cosmic::widget;

use crate::model::config::{Config, parse_color};

/// Everything the settings window owns. `None` on `App` while it is closed.
pub struct State {
    pub window: SurfaceId,
    /// Set when `config.ron` exists but does not parse: the window shows
    /// the reason instead of the pages and writes nothing.
    pub config_error: Option<String>,
    /// Text fields are kept as typed, so a half-typed value never reaches
    /// the config; they are seeded when the window opens and are not
    /// re-seeded by a later hand edit (which would eat what is being typed).
    pub active_border_field: String,
    pub inactive_border_field: String,
    /// One line of feedback under the page.
    pub note: Option<String>,
}

impl State {
    pub fn new(window: SurfaceId, config: &Config) -> Self {
        let mut state = Self {
            window,
            config_error: None,
            active_border_field: config.active_border.clone().unwrap_or_default(),
            inactive_border_field: config.inactive_border.clone(),
            note: None,
        };
        state.refresh();
        state
    }

    /// Re-read what lives outside `Config`: whether `config.ron` parses.
    pub fn refresh(&mut self) {
        self.config_error = Config::try_load().err();
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

/// Apply one settings change to `config`. `Ok(true)` = the config changed
/// and must be applied live and written; `Ok(false)` = the message was not
/// a config field; `Err` = a message for the note line, nothing changed.
pub fn apply_config_field(config: &mut Config, msg: &Msg) -> Result<bool, String> {
    let before = config.clone();
    match msg {
        Msg::ThumbWidth(v) => config.thumb_width = (*v).clamp(80, 1600),
        Msg::Zoom(v) => config.zoom_factor = (*v).clamp(1.0, 4.0),
        Msg::BorderPx(v) => config.border_px = (*v).min(16),
        Msg::CornerRadius(v) => config.corner_radius = (*v).min(64),
        Msg::ShowNames(v) => config.show_names = *v,
        Msg::ActiveBorder(text) => config.active_border = parse_optional_color(text)?,
        Msg::InactiveBorder(text) => config.inactive_border = parse_required_color(text)?,
        _ => return Ok(false),
    }
    Ok(*config != before)
}

/// The whole window: header bar, the page (or the broken-config notice),
/// and the note line.
pub fn view<'a>(state: &'a State, config: &'a Config, focused: bool) -> Element<'a, super::Msg> {
    let body: Element<'a, Msg> = match &state.config_error {
        Some(error) => broken_config(error),
        None => display_page(state, config),
    };
    let note: Element<'a, Msg> = match &state.note {
        Some(note) => widget::text::caption(note.as_str()).into(),
        None => widget::text::caption(
            "Changes apply at once and are written to ~/.config/yutani/config.ron. \
             Comments and unknown keys in that file are not preserved.",
        )
        .into(),
    };
    let page = widget::container(widget::column::with_children(vec![body, note]).spacing(12))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
