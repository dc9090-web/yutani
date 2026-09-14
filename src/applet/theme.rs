//! The popover's look (redesign spec §2, handoff "Design tokens"): sizes
//! and paddings as designed, colours from the COSMIC theme so the popover
//! follows the user's accent and light/dark preference. The one fixed
//! colour is the Yutani violet of the header badge, which the handoff
//! gives no theme role.

use cosmic::iced::border::Radius;
use cosmic::iced::widget::container;
use cosmic::iced::{Background, Border, Color, Padding};
use cosmic::widget::button;

// ---- geometry -------------------------------------------------------------

/// The popover's width. The handoff draws 340; libcosmic's applet popup
/// container pins its content to 360 (`popup_container`'s autosize
/// limits), so the content fills that and every inset is as designed.
pub const POPOVER_WIDTH: u32 = 360;
pub const CARD_RADIUS: f32 = 11.0;
pub const TILE_RADIUS: f32 = 9.0;
pub const ROW_RADIUS: f32 = 7.0;
pub const PRIMARY_RADIUS: f32 = 10.0;
pub const MENU_RADIUS: f32 = 8.0;
pub const SMALL_BUTTON_RADIUS: f32 = 6.0;
pub const CHIP_RADIUS: f32 = 4.0;
pub const BADGE_RADIUS: f32 = 8.0;
pub const BADGE_PX: f32 = 26.0;
/// The mark inside the header plate: 18 of the plate's 26 px, so the
/// glyph's own 32-unit margins leave it optically centred.
pub const BADGE_MARK_PX: f32 = 18.0;
pub const PRIMARY_HEIGHT: f32 = 38.0;
pub const OVERFLOW_PX: f32 = 38.0;
pub const ACCOUNT_ROW_HEIGHT: f32 = 30.0;
pub const MENU_ROW_HEIGHT: f32 = 31.0;
pub const TOGGLE_PX: f32 = 22.0;
pub const STATUS_DOT_PX: f32 = 8.0;
pub const STATUS_GLOW_PX: f32 = 4.0;
pub const GRAPH_HEIGHT: f32 = 64.0;
pub const GRAPH_GAP: f32 = 1.5;
/// Half the graph less the axis; a bar is at least 1 px of it.
pub const GRAPH_HALF: f32 = 31.5;
pub const INDEX_CHIP_WIDTH: f32 = 16.0;

const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

/// Header — `13px 16px 11px`.
pub const HEADER_PAD: Padding = pad(13.0, 16.0, 11.0, 16.0);
/// A card's margins — `0 12px 12px`.
pub const CARD_MARGIN: Padding = pad(0.0, 12.0, 12.0, 12.0);
/// The accounts card's header row — `10px 13px 8px`.
pub const CARD_HEADER_PAD: Padding = pad(10.0, 13.0, 8.0, 13.0);
/// The account rows' block — `0 6px 6px`.
pub const ACCOUNT_LIST_PAD: Padding = pad(0.0, 6.0, 6.0, 6.0);
/// One account row — `0 7px`.
pub const ACCOUNT_ROW_PAD: Padding = pad(0.0, 7.0, 0.0, 7.0);
/// The stopped copy — `2px 13px 13px`.
pub const STOPPED_PAD: Padding = pad(2.0, 13.0, 13.0, 13.0);
/// The accounts card's footer — `8px 13px 10px`.
pub const CARD_FOOTER_PAD: Padding = pad(8.0, 13.0, 10.0, 13.0);
/// The small footer button — `3px 8px`.
pub const SMALL_BUTTON_PAD: Padding = pad(3.0, 8.0, 3.0, 8.0);
/// The divider above the tunnel section — `0 16px 11px`.
pub const DIVIDER_PAD: Padding = pad(0.0, 16.0, 11.0, 16.0);
/// The tunnel section label row — `0 16px 8px`.
pub const SECTION_LABEL_PAD: Padding = pad(0.0, 16.0, 8.0, 16.0);
/// The tunnel status line — `0 16px 12px`.
pub const TUNNEL_LINE_PAD: Padding = pad(0.0, 16.0, 12.0, 16.0);
/// The throughput card's header — `11px 13px 8px`.
pub const GRAPH_HEADER_PAD: Padding = pad(11.0, 13.0, 8.0, 13.0);
/// The graph — `0 13px 10px`.
pub const GRAPH_PAD: Padding = pad(0.0, 13.0, 10.0, 13.0);
/// The graph's footer — `0 13px 11px`.
pub const GRAPH_FOOTER_PAD: Padding = pad(0.0, 13.0, 11.0, 13.0);
/// One fact tile — `9px 11px`.
pub const TILE_PAD: Padding = pad(9.0, 11.0, 9.0, 11.0);
/// The menu — `6px 6px 8px`.
pub const MENU_PAD: Padding = pad(6.0, 6.0, 8.0, 6.0);
/// One menu row — `0 10px`.
pub const MENU_ROW_PAD: Padding = pad(0.0, 10.0, 0.0, 10.0);
/// A note under the action row.
pub const NOTE_PAD: Padding = pad(0.0, 16.0, 10.0, 16.0);

pub const TILE_GAP: u16 = 8;
pub const ACTION_GAP: u16 = 8;
pub const HEADER_GAP: u16 = 9;
pub const ACCOUNT_ROW_GAP: u16 = 9;
pub const STATUS_GAP: u16 = 8;

// ---- typography (px) ------------------------------------------------------

pub const TITLE_SIZE: f32 = 14.0;
pub const STATE_WORD_SIZE: f32 = 12.0;
pub const SECTION_LABEL_SIZE: f32 = 10.5;
pub const COUNT_SIZE: f32 = 12.0;
pub const ACCOUNT_NAME_SIZE: f32 = 12.5;
pub const INDEX_SIZE: f32 = 10.5;
pub const FOCUSED_SIZE: f32 = 10.5;
pub const STOPPED_SIZE: f32 = 12.0;
pub const STOPPED_HELP_SIZE: f32 = 11.5;
pub const HINT_SIZE: f32 = 10.5;
pub const SMALL_BUTTON_SIZE: f32 = 11.0;
pub const EVE_ONLY_SIZE: f32 = 11.0;
pub const TUNNEL_NAME_SIZE: f32 = 14.0;
pub const LOCATION_SIZE: f32 = 12.5;
pub const UPTIME_SIZE: f32 = 11.0;
pub const RATE_LABEL_SIZE: f32 = 10.5;
pub const RATE_SIZE: f32 = 20.0;
pub const RATE_UNIT_SIZE: f32 = 11.0;
pub const GRAPH_FOOTER_SIZE: f32 = 10.5;
pub const TILE_LABEL_SIZE: f32 = 10.5;
pub const TILE_VALUE_SIZE: f32 = 12.0;
pub const PRIMARY_SIZE: f32 = 13.0;
pub const OVERFLOW_SIZE: f32 = 15.0;
pub const MENU_SIZE: f32 = 13.0;
pub const MENU_HINT_SIZE: f32 = 11.0;
pub const NOTE_SIZE: f32 = 11.0;

/// The panel icon's opacity in the daemon-offline / not-installed state:
/// the redesign's 40 %, "present but inactive".
pub const DIM_OPACITY: f32 = 0.40;
/// Disabled text, as a factor on its colour's alpha.
pub const DISABLED_ALPHA: f32 = 0.45;
/// The status badge's ring, in the panel's own background.
pub const BADGE_RING_PX: f32 = 1.5;

// ---- the one fixed colour -------------------------------------------------

/// Yutani violet, the header badge's glyph (handoff token `#b9a6f5`).
pub const VIOLET: Color = rgb(0xb9, 0xa6, 0xf5);
/// The badge's plate and its border, from the mock.
pub const VIOLET_PLATE: Color = rgb(0x29, 0x2a, 0x3d);
pub const VIOLET_PLATE_BORDER: Color = rgb(0x4a, 0x4a, 0x73);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}

// ---- theme roles ----------------------------------------------------------

pub fn with_alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

pub fn dimmed(color: Color) -> Color {
    Color { a: color.a * DISABLED_ALPHA, ..color }
}

type Cosmic = cosmic::cosmic_theme::Theme;

/// Text primary.
pub fn ink(c: &Cosmic) -> Color {
    c.on_bg_color().into()
}
/// Text secondary: `Text::Default` at 70 %.
pub fn secondary(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.70)
}
/// Text tertiary / captions: `Text::Default` at 55 %.
pub fn tertiary(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.55)
}
pub fn success(c: &Cosmic) -> Color {
    c.success_color().into()
}
pub fn accent(c: &Cosmic) -> Color {
    c.accent_color().into()
}
pub fn destructive(c: &Cosmic) -> Color {
    c.destructive_color().into()
}
pub fn warning(c: &Cosmic) -> Color {
    c.warning_color().into()
}
/// The download series: accent-adjacent — the accent itself here.
pub fn download(c: &Cosmic) -> Color {
    accent(c)
}
/// Card / list surface.
pub fn card_bg(c: &Cosmic) -> Color {
    c.background(false).component.base.into()
}
/// Card edge.
pub fn card_border(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.10)
}
/// Row divider.
pub fn divider(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.08)
}
/// Control background and border.
pub fn control_bg(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.07)
}
pub fn control_border(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.14)
}
/// Row hover.
pub fn hover(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.09)
}
/// The panel background, for the status badge's cut-out ring.
pub fn panel_bg(c: &Cosmic) -> Color {
    c.bg_color().into()
}

/// The panel mark's ink: pure white on a dark theme (the theme's `on_bg`
/// is a soft grey that reads dull at 16 px), the theme's `on_bg` on a
/// light one.
pub const MARK_ON_DARK: Color = Color::WHITE;

fn mark_ink(c: &Cosmic) -> Color {
    if c.is_dark { MARK_ON_DARK } else { ink(c) }
}

/// The header plate's mark: the Yutani violet.
pub fn violet_mark_class() -> cosmic::theme::Svg {
    cosmic::theme::Svg::custom(|_| cosmic::iced::widget::svg::Style { color: Some(VIOLET) })
}

/// The SVG class for the tinted panel mark.
pub fn mark_class() -> cosmic::theme::Svg {
    cosmic::theme::Svg::custom(|theme| cosmic::iced::widget::svg::Style { color: Some(mark_ink(theme.cosmic())) })
}

/// The badge's diameter for a panel icon `icon_px` tall: the handoff's
/// 8 px at the standard 24 px icon, scaled with it, never below 7 or
/// above 10.
pub fn badge_px(icon_px: f32) -> f32 {
    (icon_px / 3.0).round().clamp(7.0, 10.0)
}

/// The status badge (redesign spec §1): a filled dot in the theme's
/// success or warning colour with a ring cut out in the panel background,
/// or — while an action settles — a hollow ring of ink around that same
/// background.
pub fn badge_class(diameter: f32, badge: super::icon::Badge) -> cosmic::theme::Container<'static> {
    use super::icon::Badge;
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        let panel = panel_bg(c);
        let (fill, ring) = match badge {
            Badge::Connected => (success(c), panel),
            Badge::Attention => (c.warning_color().into(), panel),
            Badge::Sync => (panel, mark_ink(c)),
        };
        container::Style {
            background: Some(Background::Color(fill)),
            border: Border { radius: Radius::from(diameter / 2.0), width: BADGE_RING_PX, color: ring },
            ..Default::default()
        }
    })
}

// ---- container classes ----------------------------------------------------

fn surface(radius: f32, bg: fn(&Cosmic) -> Color, edge: Option<fn(&Cosmic) -> Color>) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        container::Style {
            background: Some(Background::Color(bg(c))),
            border: Border {
                radius: Radius::from(radius),
                width: if edge.is_some() { 1.0 } else { 0.0 },
                color: edge.map_or(Color::TRANSPARENT, |e| e(c)),
            },
            ..Default::default()
        }
    })
}

/// A card: the card surface, 1 px edge, radius 11.
pub fn card_class() -> cosmic::theme::Container<'static> {
    surface(CARD_RADIUS, card_bg, Some(card_border))
}

/// A fact tile: the card surface, radius 9.
pub fn tile_class() -> cosmic::theme::Container<'static> {
    surface(TILE_RADIUS, card_bg, Some(card_border))
}

/// The header badge's violet plate.
pub fn plate_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(VIOLET_PLATE)),
        border: Border { radius: Radius::from(BADGE_RADIUS), width: 1.0, color: VIOLET_PLATE_BORDER },
        ..Default::default()
    })
}

/// An account row's index chip: the accent surface on the focused row, the
/// control surface otherwise.
pub fn index_chip_class(focused: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        container::Style {
            background: Some(Background::Color(if focused { with_alpha(accent(c), 0.28) } else { control_bg(c) })),
            border: Border { radius: Radius::from(CHIP_RADIUS), ..Default::default() },
            ..Default::default()
        }
    })
}

/// A 1 px hairline in the divider colour.
pub fn hairline_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|theme| container::Style {
        background: Some(Background::Color(divider(theme.cosmic()))),
        ..Default::default()
    })
}

/// A round dot in `color`; `on` picks the theme's success colour, else a
/// muted ink.
pub fn dot_class(on: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        container::Style {
            background: Some(Background::Color(state_color(c, on))),
            border: Border { radius: Radius::from(STATUS_DOT_PX / 2.0), ..Default::default() },
            ..Default::default()
        }
    })
}

/// The dot's 4 px glow ring: its colour at 50 %.
pub fn glow_class(on: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        container::Style {
            background: Some(Background::Color(with_alpha(state_color(c, on), 0.5))),
            border: Border { radius: Radius::from(STATUS_DOT_PX / 2.0 + STATUS_GLOW_PX), ..Default::default() },
            ..Default::default()
        }
    })
}

/// The colour a state carries: success when on, a muted ink when idle.
pub fn state_color(c: &Cosmic, on: bool) -> Color {
    if on { success(c) } else { with_alpha(ink(c), 0.45) }
}

/// One bar of the graph, in the upload or download series colour.
pub fn bar_class(upload: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        container::Style {
            background: Some(Background::Color(if upload { success(c) } else { download(c) })),
            border: Border { radius: Radius::from(1.0), ..Default::default() },
            ..Default::default()
        }
    })
}

/// The graph's 1 px centre axis.
pub fn axis_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|theme| container::Style {
        background: Some(Background::Color(with_alpha(ink(theme.cosmic()), 0.16))),
        ..Default::default()
    })
}

// ---- button classes -------------------------------------------------------

fn button_style(text: Color, fill: Option<Color>, edge: Option<Color>, radius: f32) -> button::Style {
    button::Style {
        background: fill.map(Background::Color),
        border_radius: Radius::from(radius),
        border_width: if edge.is_some() { 1.0 } else { 0.0 },
        border_color: edge.unwrap_or(Color::TRANSPARENT),
        text_color: Some(text),
        icon_color: Some(text),
        ..Default::default()
    }
}

/// An account row: transparent (or the accent-tinted surface when focused),
/// the hover fill under the pointer, radius 7.
pub fn account_row_class(focused: bool) -> cosmic::theme::Button {
    let rest = move |c: &Cosmic| focused.then(|| with_alpha(accent(c), 0.14));
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, t| button_style(ink(t.cosmic()), rest(t.cosmic()), None, ROW_RADIUS)),
        disabled: Box::new(move |t| button_style(dimmed(ink(t.cosmic())), None, None, ROW_RADIUS)),
        hovered: Box::new(move |_, t| {
            let c = t.cosmic();
            button_style(ink(c), Some(rest(c).unwrap_or(hover(c))), None, ROW_RADIUS)
        }),
        pressed: Box::new(move |_, t| button_style(ink(t.cosmic()), Some(hover(t.cosmic())), None, ROW_RADIUS)),
    }
}

/// The small footer button: control surface and border when `active`
/// (thumbnails shown), transparent otherwise; dim and inert when disabled.
pub fn small_button_class(active: bool) -> cosmic::theme::Button {
    let fill = move |c: &Cosmic| active.then(|| control_bg(c));
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, t| {
            let c = t.cosmic();
            button_style(ink(c), fill(c), Some(control_border(c)), SMALL_BUTTON_RADIUS)
        }),
        disabled: Box::new(|t| {
            let c = t.cosmic();
            button_style(dimmed(ink(c)), None, Some(with_alpha(control_border(c), 0.5)), SMALL_BUTTON_RADIUS)
        }),
        hovered: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(ink(c), Some(hover(c)), Some(control_border(c)), SMALL_BUTTON_RADIUS)
        }),
        pressed: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(ink(c), Some(hover(c)), Some(control_border(c)), SMALL_BUTTON_RADIUS)
        }),
    }
}

/// The inert primary button: muted surface, dim text, no hover.
pub fn inert_primary_class() -> cosmic::theme::Button {
    let style = |t: &cosmic::Theme| {
        let c = t.cosmic();
        button_style(dimmed(ink(c)), Some(with_alpha(ink(c), 0.04)), Some(card_border(c)), PRIMARY_RADIUS)
    };
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, t| style(t)),
        disabled: Box::new(style),
        hovered: Box::new(move |_, t| style(t)),
        pressed: Box::new(move |_, t| style(t)),
    }
}

/// The `⋯` overflow button: control surface and border, radius 10.
pub fn overflow_class(open: bool) -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, t| {
            let c = t.cosmic();
            button_style(ink(c), Some(if open { hover(c) } else { control_bg(c) }), Some(control_border(c)), PRIMARY_RADIUS)
        }),
        disabled: Box::new(|t| {
            let c = t.cosmic();
            button_style(dimmed(ink(c)), Some(control_bg(c)), Some(control_border(c)), PRIMARY_RADIUS)
        }),
        hovered: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(ink(c), Some(hover(c)), Some(control_border(c)), PRIMARY_RADIUS)
        }),
        pressed: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(ink(c), Some(hover(c)), Some(control_border(c)), PRIMARY_RADIUS)
        }),
    }
}

/// A menu row: transparent at rest, the hover fill under the pointer,
/// destructive text for Quit, dim when disabled.
pub fn menu_row_class(danger: bool) -> cosmic::theme::Button {
    let text = move |c: &Cosmic| if danger { destructive(c) } else { ink(c) };
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, t| button_style(text(t.cosmic()), None, None, MENU_RADIUS)),
        disabled: Box::new(move |t| button_style(dimmed(text(t.cosmic())), None, None, MENU_RADIUS)),
        hovered: Box::new(move |_, t| button_style(text(t.cosmic()), Some(hover(t.cosmic())), None, MENU_RADIUS)),
        pressed: Box::new(move |_, t| button_style(text(t.cosmic()), Some(hover(t.cosmic())), None, MENU_RADIUS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handoff's 8 px dot at the standard 24 px icon, scaled with the
    /// icon and clamped: XS panels keep a 7 px dot, huge ones stop at 10.
    #[test]
    fn the_badge_is_a_third_of_the_icon_within_bounds() {
        assert_eq!(badge_px(24.0), 8.0);
        assert_eq!(badge_px(16.0), 7.0);
        assert_eq!(badge_px(10.0), 7.0);
        assert_eq!(badge_px(64.0), 10.0);
        assert_eq!(BADGE_RING_PX, 1.5);
        assert_eq!(DIM_OPACITY, 0.40);
    }

    /// The handoff's geometry, as designed.
    #[test]
    fn sizes_match_the_handoff() {
        assert_eq!(POPOVER_WIDTH, 360, "libcosmic's popup width");
        assert_eq!(TOGGLE_PX, 22.0);
        assert_eq!((CARD_RADIUS, TILE_RADIUS, ROW_RADIUS, PRIMARY_RADIUS, MENU_RADIUS), (11.0, 9.0, 7.0, 10.0, 8.0));
        assert_eq!((BADGE_PX, BADGE_RADIUS, BADGE_MARK_PX), (26.0, 8.0, 18.0));
        assert_eq!((PRIMARY_HEIGHT, OVERFLOW_PX, ACCOUNT_ROW_HEIGHT, MENU_ROW_HEIGHT), (38.0, 38.0, 30.0, 31.0));
        assert_eq!((GRAPH_HEIGHT, GRAPH_GAP, GRAPH_HALF), (64.0, 1.5, 31.5));
        assert_eq!((STATUS_DOT_PX, STATUS_GLOW_PX, INDEX_CHIP_WIDTH), (8.0, 4.0, 16.0));
        assert_eq!(HEADER_PAD, pad(13.0, 16.0, 11.0, 16.0));
        assert_eq!(CARD_MARGIN, pad(0.0, 12.0, 12.0, 12.0));
        assert_eq!(CARD_HEADER_PAD, pad(10.0, 13.0, 8.0, 13.0));
        assert_eq!(CARD_FOOTER_PAD, pad(8.0, 13.0, 10.0, 13.0));
        assert_eq!(DIVIDER_PAD, pad(0.0, 16.0, 11.0, 16.0));
        assert_eq!(GRAPH_HEADER_PAD, pad(11.0, 13.0, 8.0, 13.0));
        assert_eq!(GRAPH_PAD, pad(0.0, 13.0, 10.0, 13.0));
        assert_eq!(TILE_PAD, pad(9.0, 11.0, 9.0, 11.0));
        assert_eq!(MENU_PAD, pad(6.0, 6.0, 8.0, 6.0));
        // Type: the big metric is 20, nothing is smaller than 10.5.
        assert_eq!(RATE_SIZE, 20.0);
        for size in [
            TITLE_SIZE, STATE_WORD_SIZE, SECTION_LABEL_SIZE, COUNT_SIZE, ACCOUNT_NAME_SIZE, INDEX_SIZE, FOCUSED_SIZE,
            HINT_SIZE, SMALL_BUTTON_SIZE, TUNNEL_NAME_SIZE, LOCATION_SIZE, UPTIME_SIZE, RATE_LABEL_SIZE, RATE_UNIT_SIZE,
            GRAPH_FOOTER_SIZE, TILE_LABEL_SIZE, TILE_VALUE_SIZE, PRIMARY_SIZE, MENU_SIZE, MENU_HINT_SIZE, NOTE_SIZE,
        ] {
            assert!(size >= 10.5, "{size}");
        }
    }

    #[test]
    fn the_violet_is_the_handoffs_token_and_alpha_helpers_touch_only_alpha() {
        let v = VIOLET;
        assert_eq!(((v.r * 255.0).round(), (v.g * 255.0).round(), (v.b * 255.0).round()), (0xb9 as f32, 0xa6 as f32, 0xf5 as f32));
        let c = with_alpha(Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 }, 0.5);
        assert_eq!((c.r, c.g, c.b, c.a), (0.1, 0.2, 0.3, 0.5));
        assert!((dimmed(c).a - 0.5 * DISABLED_ALPHA).abs() < 1e-6);
    }
}
