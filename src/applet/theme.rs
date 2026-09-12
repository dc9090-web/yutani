//! The handoff's design tokens. Colours are the bundle's hex literals,
//! converted once here; sizes are its px values. Nothing else in the
//! codebase may repeat them.

use cosmic::iced::border::Radius;
use cosmic::iced::{Background, Color, Shadow, Vector};
use cosmic::widget::button;
use cosmic::widget::container;

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

// ---- sizes (handoff "Screen: applet popup") ----
pub const POPUP_PADDING: u16 = 6;
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
    }

    #[test]
    fn sizes_match_the_handoff() {
        assert_eq!(POPUP_PADDING, 6);
        assert_eq!(MARK_PX, 24);
        assert_eq!((TITLE_SIZE, STATUS_SIZE, CHIP_SIZE), (14.0, 11.5, 11.0));
        assert_eq!((COUNT_SIZE, COUNT_LABEL_SIZE, BAND_RIGHT_SIZE), (22.0, 13.0, 10.5));
        assert_eq!((TILE_LABEL_SIZE, TILE_TOTAL_SIZE, TILE_RATE_SIZE), (10.0, 16.0, 11.0));
        assert_eq!((MENU_SIZE, MENU_HINT_SIZE, MENU_RADIUS), (13.5, 10.5, 8.0));
        assert_eq!(DIM_OPACITY, 0.38);
    }
}
