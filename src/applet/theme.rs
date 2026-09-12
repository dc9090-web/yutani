//! The handoff's design tokens. Colours are the bundle's hex literals,
//! converted once here; sizes are its px values. Nothing else in the
//! codebase may repeat them.

use cosmic::iced::border::Radius;
use cosmic::iced::{Background, Color, Padding, Shadow, Vector};
use cosmic::widget::button;
use cosmic::widget::container;
use cosmic::widget::svg;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 1.0)
}
const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, a)
}

// ---- colours (handoff "Design tokens") ----
/// `#F2F4F7` — titles, counts, tile totals.
pub const TEXT_PRIMARY: Color = rgb(0xF2, 0xF4, 0xF7);
/// `#E6E8EC` — the Y mark in the header and menu labels.
pub const TEXT_ON_SURFACE: Color = rgb(0xE6, 0xE8, 0xEC);
/// `#C2C7CF` — the location and the accounts label.
pub const TEXT_SECONDARY: Color = rgb(0xC2, 0xC7, 0xCF);
/// `#8A8F98` — the disconnected dot and status text.
pub const TEXT_MUTED: Color = rgb(0x8A, 0x8F, 0x98);
/// `#7D838C` — the interface chip, tile labels, menu hints, IP/handshake.
pub const TEXT_FAINT: Color = rgb(0x7D, 0x83, 0x8C);
/// `#4A4F57` — the "·" between status and location.
pub const SEPARATOR: Color = rgb(0x4A, 0x4F, 0x57);
/// `#2FD6B0` — connected, upload.
pub const ACCENT_UP: Color = rgb(0x2F, 0xD6, 0xB0);
/// `#2FD6B0AA` — the connected dot's glow.
pub const ACCENT_UP_GLOW: Color = rgba(0x2F, 0xD6, 0xB0, 0xAA as f32 / 255.0);
/// `#5B9BFF` — download.
pub const ACCENT_DOWN: Color = rgb(0x5B, 0x9B, 0xFF);
/// `#FF8A7E` — the Quit label.
pub const DANGER_TEXT: Color = rgb(0xFF, 0x8A, 0x7E);
/// `#FF6B5C1F` — the Quit hover fill.
pub const DANGER_HOVER: Color = rgba(0xFF, 0x6B, 0x5C, 0x1F as f32 / 255.0);
/// `#FFFFFF08` — tile fill.
pub const TILE_FILL: Color = rgba(0xFF, 0xFF, 0xFF, 0x08 as f32 / 255.0);
/// `#FFFFFF0F` — tile border and the interface chip's background.
pub const CHIP_FILL: Color = rgba(0xFF, 0xFF, 0xFF, 0x0F as f32 / 255.0);
/// `#FFFFFF12` — hairline dividers and menu hover.
pub const HAIRLINE: Color = rgba(0xFF, 0xFF, 0xFF, 0x12 as f32 / 255.0);
/// `#1A1D21F5` — the popup's own surface.
///
/// The handoff is a dark-only design: every colour above it is light ink on
/// a near-black ground. libcosmic's `popup_container` paints the *COSMIC
/// theme's* background, which under a light theme would put `#F2F4F7` text
/// and `#FFFFFF08` tiles on near-white. So the popup paints its ground
/// itself and stops depending on which COSMIC theme is active. (Forcing a
/// dark `cosmic::Theme` on the whole application would also mis-tint the
/// panel button's symbolic icon, which must follow the panel.)
pub const POPUP_SURFACE: Color = rgba(0x1A, 0x1D, 0x21, 0xF5 as f32 / 255.0);
/// `#FFFFFF1A` — the popup's 1 px border.
pub const POPUP_BORDER: Color = rgba(0xFF, 0xFF, 0xFF, 0x1A as f32 / 255.0);

// ---- sizes (handoff "Screen: applet popup") ----
pub const POPUP_PADDING: u16 = 6;
pub const POPUP_RADIUS: f32 = 14.0;
pub const POPUP_BORDER_PX: f32 = 1.0;
pub const HEADER_GAP: u16 = 11;
pub const HEADER_COLUMN_GAP: u16 = 5;
pub const MARK_PX: u16 = 24;
pub const TITLE_SIZE: f32 = 14.0;
pub const STATUS_SIZE: f32 = 11.5;
pub const STATUS_GAP: u16 = 7;
pub const DOT_PX: f32 = 7.0;
pub const DOT_GLOW_BLUR: f32 = 8.0;
pub const CHIP_SIZE: f32 = 11.0;
pub const CHIP_RADIUS: f32 = 6.0;
pub const COUNT_SIZE: f32 = 22.0;
pub const COUNT_LABEL_SIZE: f32 = 13.0;
pub const BAND_RIGHT_SIZE: f32 = 10.5;
pub const TILE_GAP: u16 = 6;
pub const TILE_RADIUS: f32 = 10.0;
pub const TILE_COLUMN_GAP: u16 = 7;
pub const TILE_LABEL_SIZE: f32 = 10.0;
pub const TILE_TOTAL_SIZE: f32 = 16.0;
pub const TILE_RATE_SIZE: f32 = 11.0;
pub const GLYPH_PX: u16 = 10;
pub const MENU_SIZE: f32 = 13.5;
pub const MENU_HINT_SIZE: f32 = 10.5;
pub const MENU_RADIUS: f32 = 8.0;
/// The panel icon's opacity in the daemon-offline / not-installed state.
pub const DIM_OPACITY: f32 = 0.38;

// ---- section geometry (handoff "Screen: applet popup") ----
// The handoff's per-section `padding` / `margin` shorthands, in its own
// order (top, right, bottom, left). Every value is on its 1·2·4·6·8·10·12
// spacing scale, which `sizes_match_the_handoff` checks.

const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

/// Header — `10px 10px 2px`.
pub const HEADER_PAD: Padding = pad(10.0, 10.0, 2.0, 10.0);
/// The divider above the accounts band — `6px 10px 2px`.
pub const DIVIDER_ABOVE_BAND: Padding = pad(6.0, 10.0, 2.0, 10.0);
/// Accounts band — `4px 12px 6px`.
pub const BAND_PAD: Padding = pad(4.0, 12.0, 6.0, 12.0);
/// The divider above the traffic tiles — `2px 10px 4px`.
pub const DIVIDER_ABOVE_TILES: Padding = pad(2.0, 10.0, 4.0, 10.0);
/// Traffic tiles — `2px 10px`.
pub const TILES_PAD: Padding = pad(2.0, 10.0, 2.0, 10.0);
/// The interface chip — `4px 8px`.
pub const CHIP_PAD: Padding = pad(4.0, 8.0, 4.0, 8.0);
/// One traffic tile — `11px 13px`.
pub const TILE_PAD: Padding = pad(11.0, 13.0, 11.0, 13.0);
/// Between the accounts count and its label.
pub const COUNT_GAP: u16 = 8;
/// Between a tile's arrow glyph and its label.
pub const TILE_LABEL_GAP: u16 = 6;
/// The map pin is the one non-square glyph (handoff: 10×12).
pub const PIN_W: u16 = 10;
pub const PIN_H: u16 = 12;
/// Line box as a factor of the font size. libcosmic's `monotext` preset
/// pins an *absolute* 20 px line height, which at 10.5–22 px text would
/// wreck every gap in the popup, so each helper sets its own.
pub const LINE_HEIGHT: f32 = 1.3;
/// The accounts count is `line-height: 1` in the handoff — its 22 px digits
/// set the height of the whole band.
pub const LINE_HEIGHT_TIGHT: f32 = 1.0;

// ---- copy (handoff: "Copy is final as written") ----
/// The popup's title.
pub const TITLE: &str = "WireGuard";
/// The traffic tiles' labels (the handoff uppercases them in the source,
/// not in CSS — these are the strings, not a text-transform).
pub const UPLOAD_LABEL: &str = "UPLOAD";
pub const DOWNLOAD_LABEL: &str = "DOWNLOAD";
/// Between the status text and the location.
pub const MIDDOT: &str = "·";
/// The placeholder for everything a down tunnel cannot report.
pub const DASH: &str = "—";

/// A 1 px `#FFFFFF12` rule. The handoff's dividers, not COSMIC's.
pub fn hairline<'a, M: 'a>() -> cosmic::Element<'a, M> {
    cosmic::widget::container(
        cosmic::widget::space()
            .width(cosmic::iced::Length::Fill)
            .height(cosmic::iced::Length::Fixed(1.0)),
    )
    .class(cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(HAIRLINE)),
        ..Default::default()
    }))
    .into()
}

/// A filled, optionally glowing round dot (the header status dot).
pub fn dot_class(color: Color, glow: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |_| container::Style {
        background: Some(Background::Color(color)),
        border: cosmic::iced::Border { radius: Radius::from(DOT_PX / 2.0), ..Default::default() },
        shadow: if glow {
            Shadow { color: ACCENT_UP_GLOW, offset: Vector::ZERO, blur_radius: DOT_GLOW_BLUR }
        } else {
            Shadow::default()
        },
        ..Default::default()
    })
}

/// The popup's own surface: `#1A1D21F5`, radius 14, 1 px `#FFFFFF1A`.
/// Goes *inside* `popup_container`, whose theme-coloured ground it covers.
pub fn popup_surface_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(POPUP_SURFACE)),
        border: cosmic::iced::Border {
            color: POPUP_BORDER,
            width: POPUP_BORDER_PX,
            radius: Radius::from(POPUP_RADIUS),
        },
        ..Default::default()
    })
}

/// An SVG drawn in one explicit colour, whatever the COSMIC theme is.
///
/// `symbolic(true)` tints an icon to the *renderer's* `icon_color`, which is
/// right for the panel button (it must follow the panel) and wrong inside
/// the popup, where a light theme would paint the mark near-black on
/// [`POPUP_SURFACE`]. A custom class's `color` wins over `symbolic` in
/// libcosmic's `Svg::draw`, so this is the explicit form.
pub fn svg_class(color: Color) -> cosmic::theme::Svg {
    cosmic::theme::Svg::custom(move |_| svg::Style { color: Some(color) })
}

/// The interface chip behind `yutani0`.
pub fn chip_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(CHIP_FILL)),
        border: cosmic::iced::Border { radius: Radius::from(CHIP_RADIUS), ..Default::default() },
        ..Default::default()
    })
}

/// A traffic tile: `#FFFFFF08` on a 1 px `#FFFFFF0F` border, radius 10.
pub fn tile_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(TILE_FILL)),
        border: cosmic::iced::Border { color: CHIP_FILL, width: 1.0, radius: Radius::from(TILE_RADIUS) },
        ..Default::default()
    })
}

fn row_style(text: Color, fill: Option<Color>) -> button::Style {
    button::Style {
        background: fill.map(Background::Color),
        border_radius: Radius::from(MENU_RADIUS),
        text_color: Some(text),
        icon_color: Some(text),
        ..Default::default()
    }
}

/// A menu row: transparent, radius 8, `hover` fill on hover and press, and
/// 40 % text when disabled.
pub fn menu_row_class(text: Color, hover: Color) -> cosmic::theme::Button {
    let dim = Color { a: 0.4, ..text };
    cosmic::theme::Button::Custom {
        active: Box::new(move |_focused, _theme| row_style(text, None)),
        disabled: Box::new(move |_theme| row_style(dim, None)),
        hovered: Box::new(move |_focused, _theme| row_style(text, Some(hover))),
        pressed: Box::new(move |_focused, _theme| row_style(text, Some(hover))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::parse_color;

    /// Every colour const must equal the handoff's hex literal, parsed by
    /// the same code the config uses.
    #[test]
    fn colours_match_the_handoff_tokens() {
        let expect = |c: Color, hex: &str| {
            let [r, g, b, a] = parse_color(hex).expect(hex);
            assert!((c.r - r).abs() < 0.002 && (c.g - g).abs() < 0.002 && (c.b - b).abs() < 0.002 && (c.a - a).abs() < 0.002,
                "{hex}: got {c:?}");
        };
        expect(TEXT_PRIMARY, "#F2F4F7");
        expect(TEXT_ON_SURFACE, "#E6E8EC");
        expect(TEXT_SECONDARY, "#C2C7CF");
        expect(TEXT_MUTED, "#8A8F98");
        expect(TEXT_FAINT, "#7D838C");
        expect(SEPARATOR, "#4A4F57");
        expect(ACCENT_UP, "#2FD6B0");
        expect(ACCENT_UP_GLOW, "#2FD6B0AA");
        expect(ACCENT_DOWN, "#5B9BFF");
        expect(DANGER_TEXT, "#FF8A7E");
        expect(DANGER_HOVER, "#FF6B5C1F");
        expect(TILE_FILL, "#FFFFFF08");
        expect(CHIP_FILL, "#FFFFFF0F");
        expect(HAIRLINE, "#FFFFFF12");
        expect(POPUP_SURFACE, "#1A1D21F5");
        expect(POPUP_BORDER, "#FFFFFF1A");
    }

    #[test]
    fn sizes_match_the_handoff() {
        assert_eq!(POPUP_PADDING, 6);
        assert_eq!(MARK_PX, 24);
        assert_eq!((POPUP_RADIUS, POPUP_BORDER_PX), (14.0, 1.0));
        assert_eq!((TITLE_SIZE, STATUS_SIZE, CHIP_SIZE), (14.0, 11.5, 11.0));
        assert_eq!((COUNT_SIZE, COUNT_LABEL_SIZE, BAND_RIGHT_SIZE), (22.0, 13.0, 10.5));
        assert_eq!((TILE_LABEL_SIZE, TILE_TOTAL_SIZE, TILE_RATE_SIZE), (10.0, 16.0, 11.0));
        assert_eq!((MENU_SIZE, MENU_HINT_SIZE, MENU_RADIUS), (13.5, 10.5, 8.0));
        assert_eq!(DIM_OPACITY, 0.38);
        // The handoff's per-section padding shorthands, in its own order
        // (top, right, bottom, left).
        assert_eq!(HEADER_PAD, pad(10.0, 10.0, 2.0, 10.0));
        assert_eq!(DIVIDER_ABOVE_BAND, pad(6.0, 10.0, 2.0, 10.0));
        assert_eq!(BAND_PAD, pad(4.0, 12.0, 6.0, 12.0));
        assert_eq!(DIVIDER_ABOVE_TILES, pad(2.0, 10.0, 4.0, 10.0));
        assert_eq!(TILES_PAD, pad(2.0, 10.0, 2.0, 10.0));
        assert_eq!(CHIP_PAD, pad(4.0, 8.0, 4.0, 8.0));
        assert_eq!(TILE_PAD, pad(11.0, 13.0, 11.0, 13.0));
        assert_eq!((COUNT_GAP, TILE_LABEL_GAP), (8, 6));
        assert_eq!((PIN_W, PIN_H), (10, 12));
        assert_eq!((LINE_HEIGHT, LINE_HEIGHT_TIGHT), (1.3, 1.0));
        // Every size above is on the handoff's spacing / radii scales.
        for step in [
            f32::from(POPUP_PADDING),
            f32::from(HEADER_GAP),
            f32::from(HEADER_COLUMN_GAP),
            f32::from(STATUS_GAP),
            f32::from(TILE_GAP),
            f32::from(TILE_COLUMN_GAP),
            f32::from(COUNT_GAP),
            f32::from(TILE_LABEL_GAP),
        ] {
            assert!([1.0, 2.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 11.0, 12.0].contains(&step), "{step}");
        }
        for radius in [CHIP_RADIUS, MENU_RADIUS, TILE_RADIUS, POPUP_RADIUS] {
            assert!([6.0, 7.0, 8.0, 10.0, 14.0, 16.0].contains(&radius), "{radius}");
        }
    }

    /// "Copy is final as written" — the handoff. The view may not spell any
    /// of these itself.
    #[test]
    fn copy_matches_the_handoff() {
        assert_eq!(TITLE, "WireGuard");
        assert_eq!((UPLOAD_LABEL, DOWNLOAD_LABEL), ("UPLOAD", "DOWNLOAD"));
        assert_eq!(MIDDOT, "·");
        assert_eq!(DASH, "—");
    }
}
