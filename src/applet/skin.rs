//! The Nostromo skin (handoff v6 "Design tokens"): one phosphor on black
//! glass, amber when something wants attention, red for Quit and the bad
//! end of a gauge. A fixed palette by design — the popover does not follow
//! the COSMIC accent or light/dark preference. The panel button stays
//! COSMIC (`theme.rs`).

use cosmic::iced::border::Radius;
use cosmic::iced::widget::container;
use cosmic::iced::{Background, Border, Color, Padding, Shadow, Vector};
use cosmic::widget::button;

use crate::applet::fonts::Face;

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

// ---- palette ---------------------------------------------------------------

pub const BG: Color = rgba(0x05, 0x09, 0x07, 0xff);
pub const CARD: Color = rgba(0x08, 0x11, 0x09, 0xff);
pub const LINE: Color = rgba(0x17, 0x3b, 0x22, 0xff);
pub const LINE_2: Color = rgba(0x24, 0x52, 0x33, 0xff);
pub const PHOSPHOR: Color = rgba(0x7c, 0xe3, 0x8b, 0xff);
pub const DIM: Color = rgba(0x4f, 0x9c, 0x5c, 0xff);
pub const DIMMER: Color = rgba(0x3c, 0x7a, 0x47, 0xff);
pub const WHITE: Color = rgba(0xe9, 0xf5, 0xe6, 0xff);
pub const AMBER: Color = rgba(0xff, 0xb0, 0x00, 0xff);
pub const RED: Color = rgba(0xff, 0x4a, 0x3d, 0xff);

pub const HOVER: Color = rgba(0x7c, 0xe3, 0x8b, 0x12);
pub const OVERFLOW_OPEN: Color = rgba(0x7c, 0xe3, 0x8b, 0x1a);
pub const LAUNCH_GLOW: Color = rgba(0x7c, 0xe3, 0x8b, 0x40);
pub const FOCUSED_ROW: Color = rgba(0xff, 0xb0, 0x00, 0x14);
pub const HEADER_PLATE: Color = rgba(0xff, 0xb0, 0x00, 0x1f);
pub const NOTICE_BORDER: Color = rgba(0xff, 0xb0, 0x00, 0x80);
pub const QUIT_HOVER: Color = rgba(0xff, 0x4a, 0x3d, 0x1a);
/// Scanline: one row in three. The handoff's `#00000059` is drawn for a
/// 2× screen; at 1× a 35 % row lands on whole rows of the 8–9 px type and
/// erases strokes (seen on the panel 2026-10-07), so it is 12 % here.
pub const SCANLINE: Color = rgba(0, 0, 0, 0x1f);
/// The top edge's phosphor line at 70 %.
pub const TOP_LINE: Color = rgba(0x7c, 0xe3, 0x8b, 0xb3);

/// The text colours the view model speaks in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Phosphor,
    Dim,
    Dimmer,
    White,
    Amber,
    Red,
    Bg,
}

impl Ink {
    pub fn color(self) -> Color {
        match self {
            Ink::Phosphor => PHOSPHOR,
            Ink::Dim => DIM,
            Ink::Dimmer => DIMMER,
            Ink::White => WHITE,
            Ink::Amber => AMBER,
            Ink::Red => RED,
            Ink::Bg => BG,
        }
    }
}

// ---- geometry --------------------------------------------------------------

pub const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

/// libcosmic's popup width; the content fills it.
pub const POPOVER_WIDTH: u32 = 360;
pub const POPOVER_RADIUS: f32 = 4.0;
pub const CARD_RADIUS: f32 = 2.0;
pub const CHIP_RADIUS: f32 = 1.0;

pub const HEADER_PAD: Padding = pad(12.0, 14.0, 10.0, 14.0);
pub const HEADER_GAP: f32 = 10.0;
pub const PLATE_PX: f32 = 26.0;
pub const PLATE_MARK_PX: f32 = 17.0;
pub const HEADER_LED_PX: f32 = 6.0;

pub const SECTION_HEAD_PAD: Padding = pad(0.0, 14.0, 6.0, 14.0);
pub const SECTION_HEAD_GAP: f32 = 8.0;
/// Index chip `3 5 2`.
pub const SECTION_CHIP_PAD: Padding = pad(3.0, 5.0, 2.0, 5.0);
/// Every card's margin: `0 10 10`.
pub const CARD_MARGIN: Padding = pad(0.0, 10.0, 10.0, 10.0);
pub const BRACKET_PX: f32 = 8.0;

pub const ROW_PAD: Padding = pad(8.0, 12.0, 8.0, 12.0);
pub const ROW_GAP: f32 = 10.0;
pub const HAIRLINE_INSET: f32 = 12.0;
pub const LED_PX: f32 = 8.0;
pub const TOGGLE_W: f32 = 38.0;
pub const TOGGLE_H: f32 = 19.0;
pub const KNOB_PX: f32 = 13.0;

pub const ACCOUNT_LIST_PAD: f32 = 5.0;
pub const ACCOUNT_ROW_HEIGHT: f32 = 28.0;
pub const ACCOUNT_ROW_PAD: Padding = pad(0.0, 8.0, 0.0, 8.0);
pub const INDEX_CHIP_PX: f32 = 17.0;
pub const EMPTY_PAD: Padding = pad(10.0, 12.0, 11.0, 12.0);

pub const SCOPE_HEIGHT: f32 = 104.0;
pub const SCOPE_INSET: f32 = 12.0;
pub const CORNER_MARK_PX: f32 = 7.0;
pub const PING_LEFT_W: f32 = 64.0;
pub const PING_RIGHT_W: f32 = 74.0;
pub const FACTS_PAD: Padding = pad(7.0, 12.0, 8.0, 12.0);

pub const HOST_PAD: Padding = pad(8.0, 12.0, 9.0, 12.0);
pub const HOST_ROW_GAP: f32 = 6.0;
pub const GAUGE_CELLS: usize = 20;
pub const GAUGE_CELL_H: f32 = 7.0;
pub const GAUGE_GAP: f32 = 2.0;
pub const GAUGE_LABEL_W: f32 = 26.0;
pub const GAUGE_VALUE_W: f32 = 32.0;
pub const GAUGE_DETAIL_W: f32 = 54.0;

pub const NOTICE_STRIPE_H: f32 = 4.0;
pub const NOTICE_PAD: Padding = pad(7.0, 11.0, 8.0, 11.0);

pub const ACTION_GAP: f32 = 8.0;
pub const PRIMARY_HEIGHT: f32 = 36.0;
pub const PRIMARY_PAD_X: f32 = 14.0;
pub const OVERFLOW_PX: f32 = 36.0;

pub const MENU_PAD: f32 = 5.0;
pub const MENU_ROW_HEIGHT: f32 = 28.0;
pub const MENU_ROW_PAD: Padding = pad(0.0, 10.0, 0.0, 10.0);

pub const FOOTER_PAD: Padding = pad(7.0, 14.0, 8.0, 14.0);
pub const CURSOR_W: f32 = 6.0;
pub const CURSOR_H: f32 = 9.0;

// ---- type --------------------------------------------------------------------

/// A text style: size in px, face, tracking in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub size: f32,
    pub face: Face,
    pub track: f32,
}

const fn ty(size: f32, face: Face, track: f32) -> Type {
    Type { size, face, track }
}

pub const WORDMARK: Type = ty(12.0, Face::Display, 0.24);
pub const SUBLINE: Type = ty(8.5, Face::Mono, 0.14);
pub const SUBLINE_JP: Type = ty(8.5, Face::Jp, 0.14);
pub const STATE_WORD: Type = ty(10.0, Face::MonoBold, 0.14);
pub const SECTION_INDEX: Type = ty(7.5, Face::Display, 0.12);
pub const SECTION_LABEL: Type = ty(10.0, Face::MonoBold, 0.16);
pub const SECTION_META: Type = ty(8.5, Face::Mono, 0.10);
pub const COUNT: Type = ty(10.5, Face::MonoBold, 0.0);
pub const ROW_TITLE: Type = ty(11.5, Face::MonoBold, 0.06);
pub const ROW_SUB: Type = ty(8.5, Face::Mono, 0.06);
pub const ACCOUNT_NAME: Type = ty(11.5, Face::Mono, 0.04);
pub const INDEX_CHIP: Type = ty(9.5, Face::MonoBold, 0.0);
pub const FOCUSED_TAG: Type = ty(8.5, Face::MonoBold, 0.16);
pub const EMPTY_TITLE: Type = ty(10.5, Face::MonoBold, 0.08);
pub const EMPTY_BODY: Type = ty(9.5, Face::Mono, 0.04);
pub const READOUT_LABEL: Type = ty(8.5, Face::MonoBold, 0.16);
pub const READOUT: Type = ty(16.0, Face::MonoBold, 0.0);
pub const PING_VALUE: Type = ty(20.0, Face::MonoBold, 0.0);
pub const UNIT: Type = ty(8.5, Face::Mono, 0.08);
pub const SCOPE_FOOT: Type = ty(8.5, Face::Mono, 0.08);
pub const PING_STATS: Type = ty(8.0, Face::Mono, 0.0);
pub const QUALITY_WORD: Type = ty(8.5, Face::MonoBold, 0.0);
pub const FACTS: Type = ty(8.5, Face::Mono, 0.04);
pub const FACTS_KEY: Type = ty(8.5, Face::MonoBold, 0.04);
pub const GAUGE_LABEL: Type = ty(9.0, Face::MonoBold, 0.12);
pub const GAUGE_VALUE: Type = ty(11.0, Face::MonoBold, 0.0);
pub const GAUGE_DETAIL: Type = ty(8.5, Face::Mono, 0.0);
pub const NOTICE: Type = ty(9.5, Face::Mono, 0.04);
pub const PRIMARY: Type = ty(11.5, Face::MonoBold, 0.14);
pub const PRIMARY_SUB: Type = ty(8.5, Face::Mono, 0.12);
pub const LOG: Type = ty(9.5, Face::Mono, 0.04);
pub const MENU: Type = ty(10.5, Face::Mono, 0.08);
pub const FOOTER: Type = ty(8.5, Face::Mono, 0.16);
pub const OVERFLOW: Type = ty(15.0, Face::Mono, 0.0);

pub const ALL_TYPES: [Type; 34] = [
    WORDMARK, SUBLINE, SUBLINE_JP, STATE_WORD, SECTION_INDEX, SECTION_LABEL, SECTION_META, COUNT, ROW_TITLE, ROW_SUB,
    ACCOUNT_NAME, INDEX_CHIP, FOCUSED_TAG, EMPTY_TITLE, EMPTY_BODY, READOUT_LABEL, READOUT, PING_VALUE, UNIT, SCOPE_FOOT,
    PING_STATS, QUALITY_WORD, FACTS, FACTS_KEY, GAUGE_LABEL, GAUGE_VALUE, GAUGE_DETAIL, NOTICE, PRIMARY, PRIMARY_SUB, LOG,
    MENU, FOOTER, OVERFLOW,
];

// ---- container classes ---------------------------------------------------------

fn boxed(bg: Option<Color>, edge: Option<Color>, radius: f32) -> container::Style {
    container::Style {
        background: bg.map(Background::Color),
        border: Border { radius: Radius::from(radius), width: if edge.is_some() { 1.0 } else { 0.0 }, color: edge.unwrap_or(Color::TRANSPARENT) },
        ..Default::default()
    }
}

fn class(style: container::Style) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |_| style)
}

/// The popover's glass: `BG`, 1 px `LINE` edge, radius 4, deep shadow.
pub fn popover_class() -> cosmic::theme::Container<'static> {
    class(container::Style {
        shadow: Shadow { color: rgba(0, 0, 0, 0xff), offset: Vector::new(0.0, 30.0), blur_radius: 70.0 },
        ..boxed(Some(BG), Some(LINE), POPOVER_RADIUS)
    })
}

pub fn card_class() -> cosmic::theme::Container<'static> {
    class(boxed(Some(CARD), Some(LINE), CARD_RADIUS))
}

/// An outline card (the launch log): no fill.
pub fn log_class() -> cosmic::theme::Container<'static> {
    class(boxed(None, Some(LINE), CARD_RADIUS))
}

/// A small chip: optional fill, optional 1 px border, radius 1.
pub fn chip_class(fill: Option<Color>, edge: Option<Color>) -> cosmic::theme::Container<'static> {
    class(boxed(fill, edge, CHIP_RADIUS))
}

/// A square LED; `glow` adds the handoff's blur-8 shadow at 67 %.
pub fn led_class(color: Color, glow: bool) -> cosmic::theme::Container<'static> {
    class(container::Style {
        shadow: if glow { Shadow { color: Color { a: 0.67, ..color }, offset: Vector::ZERO, blur_radius: 8.0 } } else { Shadow::default() },
        ..boxed(Some(color), None, CHIP_RADIUS)
    })
}

/// One gauge cell.
pub fn cell_class(color: Color) -> cosmic::theme::Container<'static> {
    class(boxed(Some(color), None, CHIP_RADIUS))
}

/// A solid 1 px line.
pub fn hairline_class(color: Color) -> cosmic::theme::Container<'static> {
    class(boxed(Some(color), None, 0.0))
}

/// The header plate: amber 12 % fill, 1 px amber, radius 2.
pub fn plate_class() -> cosmic::theme::Container<'static> {
    class(boxed(Some(HEADER_PLATE), Some(AMBER), CARD_RADIUS))
}

/// The Steam notice: amber 50 % border, radius 2, no fill.
pub fn notice_class() -> cosmic::theme::Container<'static> {
    class(boxed(None, Some(NOTICE_BORDER), CARD_RADIUS))
}

/// The ready launch button's glow, worn by a wrapper container
/// (cosmic buttons have no shadow).
pub fn launch_glow_class() -> cosmic::theme::Container<'static> {
    class(container::Style { shadow: Shadow { color: LAUNCH_GLOW, offset: Vector::ZERO, blur_radius: 14.0 }, ..boxed(None, None, CARD_RADIUS) })
}

// ---- button classes --------------------------------------------------------------

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

fn states(rest: button::Style, hover: button::Style, disabled: button::Style) -> cosmic::theme::Button {
    let (r, h, d) = (rest, hover, disabled);
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, _| r),
        disabled: Box::new(move |_| d),
        hovered: Box::new(move |_, _| h),
        pressed: Box::new(move |_, _| h),
    }
}

/// An account row: amber 8 % when focused, phosphor 7 % under the pointer.
pub fn account_row_class(focused: bool) -> cosmic::theme::Button {
    let rest = focused.then_some(FOCUSED_ROW);
    states(
        button_style(PHOSPHOR, rest, None, CHIP_RADIUS),
        button_style(PHOSPHOR, Some(rest.unwrap_or(HOVER)), None, CHIP_RADIUS),
        button_style(DIMMER, rest, None, CHIP_RADIUS),
    )
}

/// A menu row; Quit is red with a red 10 % hover.
pub fn menu_row_class(danger: bool) -> cosmic::theme::Button {
    let (ink, hover) = if danger { (RED, QUIT_HOVER) } else { (PHOSPHOR, HOVER) };
    states(button_style(ink, None, None, CHIP_RADIUS), button_style(ink, Some(hover), None, CHIP_RADIUS), button_style(DIMMER, None, None, CHIP_RADIUS))
}

/// The primary button's three looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimaryLook {
    /// Phosphor fill, `BG` text.
    Ready,
    /// Phosphor outline and text.
    Busy,
    /// `LINE` outline, dimmer text, no press.
    Inert,
}

pub fn primary_class(look: PrimaryLook) -> cosmic::theme::Button {
    let s = match look {
        PrimaryLook::Ready => button_style(BG, Some(PHOSPHOR), Some(PHOSPHOR), CARD_RADIUS),
        PrimaryLook::Busy => button_style(PHOSPHOR, None, Some(PHOSPHOR), CARD_RADIUS),
        PrimaryLook::Inert => button_style(DIMMER, None, Some(LINE), CARD_RADIUS),
    };
    states(s, s, s)
}

/// The `⋯` button: `LINE_2` border at rest; open = phosphor border + 10 % fill.
pub fn overflow_class(open: bool) -> cosmic::theme::Button {
    let rest = if open { button_style(PHOSPHOR, Some(OVERFLOW_OPEN), Some(PHOSPHOR), CARD_RADIUS) } else { button_style(PHOSPHOR, None, Some(LINE_2), CARD_RADIUS) };
    let hover = button_style(PHOSPHOR, Some(if open { OVERFLOW_OPEN } else { HOVER }), Some(if open { PHOSPHOR } else { LINE_2 }), CARD_RADIUS);
    states(rest, hover, rest)
}

/// No chrome at all: a hit area around a drawn control (the rockers).
pub fn bare_class() -> cosmic::theme::Button {
    let s = button_style(PHOSPHOR, None, None, 0.0);
    states(s, s, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color) -> (u8, u8, u8, u8) {
        let q = |v: f32| (v * 255.0).round() as u8;
        (q(c.r), q(c.g), q(c.b), q(c.a))
    }

    #[test]
    fn the_palette_is_the_handoffs() {
        assert_eq!(hex(BG), (0x05, 0x09, 0x07, 0xff));
        assert_eq!(hex(CARD), (0x08, 0x11, 0x09, 0xff));
        assert_eq!(hex(LINE), (0x17, 0x3b, 0x22, 0xff));
        assert_eq!(hex(LINE_2), (0x24, 0x52, 0x33, 0xff));
        assert_eq!(hex(PHOSPHOR), (0x7c, 0xe3, 0x8b, 0xff));
        assert_eq!(hex(DIM), (0x4f, 0x9c, 0x5c, 0xff));
        assert_eq!(hex(DIMMER), (0x3c, 0x7a, 0x47, 0xff));
        assert_eq!(hex(WHITE), (0xe9, 0xf5, 0xe6, 0xff));
        assert_eq!(hex(AMBER), (0xff, 0xb0, 0x00, 0xff));
        assert_eq!(hex(RED), (0xff, 0x4a, 0x3d, 0xff));
        assert_eq!(hex(HOVER), (0x7c, 0xe3, 0x8b, 0x12));
        assert_eq!(hex(FOCUSED_ROW), (0xff, 0xb0, 0x00, 0x14));
        assert_eq!(hex(HEADER_PLATE), (0xff, 0xb0, 0x00, 0x1f));
        assert_eq!(hex(NOTICE_BORDER), (0xff, 0xb0, 0x00, 0x80));
        assert_eq!(hex(QUIT_HOVER), (0xff, 0x4a, 0x3d, 0x1a));
        assert_eq!(hex(OVERFLOW_OPEN), (0x7c, 0xe3, 0x8b, 0x1a));
        assert_eq!(hex(LAUNCH_GLOW), (0x7c, 0xe3, 0x8b, 0x40));
    }

    #[test]
    fn geometry_matches_the_handoff() {
        assert_eq!(POPOVER_WIDTH, 360);
        assert_eq!((POPOVER_RADIUS, CARD_RADIUS, CHIP_RADIUS), (4.0, 2.0, 1.0));
        assert_eq!((TOGGLE_W, TOGGLE_H, KNOB_PX), (38.0, 19.0, 13.0));
        assert_eq!((ACCOUNT_ROW_HEIGHT, MENU_ROW_HEIGHT, PRIMARY_HEIGHT, OVERFLOW_PX), (28.0, 28.0, 36.0, 36.0));
        assert_eq!((LED_PX, HEADER_LED_PX, INDEX_CHIP_PX, PLATE_PX, PLATE_MARK_PX), (8.0, 6.0, 17.0, 26.0, 17.0));
        assert_eq!((SCOPE_HEIGHT, GAUGE_CELLS, GAUGE_CELL_H), (104.0, 20, 7.0));
        assert_eq!(HEADER_PAD, pad(12.0, 14.0, 10.0, 14.0));
        assert_eq!(CARD_MARGIN, pad(0.0, 10.0, 10.0, 10.0));
        assert_eq!(SECTION_HEAD_PAD, pad(0.0, 14.0, 6.0, 14.0));
        assert_eq!(ROW_PAD, pad(8.0, 12.0, 8.0, 12.0));
        assert_eq!(MENU_PAD, 5.0);
    }

    /// The terminal's floor is 8 px (ping stats, by design).
    #[test]
    fn nothing_is_set_below_8_px() {
        for t in ALL_TYPES {
            assert!(t.size >= 7.5, "{t:?}");
        }
        assert_eq!(SECTION_INDEX.size, 7.5, "the one exception: Michroma section chips");
        assert_eq!(PING_STATS.size, 8.0);
    }

    #[test]
    fn ink_maps_to_the_palette() {
        assert_eq!(Ink::Phosphor.color(), PHOSPHOR);
        assert_eq!(Ink::Dimmer.color(), DIMMER);
        assert_eq!(Ink::Red.color(), RED);
    }
}
