//! Building blocks for the settings window (redesign spec §3, handoff
//! "Screen 2"): the handoff's geometry as constants, colours from the
//! COSMIC theme (through `yutani::applet::theme`'s roles), and the widgets
//! every pane is made of — cards, rows, pills, toggles, steppers, swatches,
//! chips, tinted panels, the sidebar. Nothing here knows what a setting is.

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::border::Radius;
use cosmic::iced::font::Weight;
use cosmic::iced::widget::container;
use cosmic::iced::{Alignment, Background, Border, Color, Length, Padding};
use cosmic::widget::{self, Column, Row, button};
use cosmic::{Element, theme as cosmic_theme};

use yutani::applet::theme as roles;

type Cosmic = cosmic::cosmic_theme::Theme;

// ---- geometry ----------------------------------------------------------------

pub const WINDOW_W: f32 = 900.0;
pub const WINDOW_H: f32 = 724.0;
pub const MIN_W: f32 = 760.0;
pub const MIN_H: f32 = 560.0;
pub const SIDEBAR_W: f32 = 204.0;
pub const CARD_RADIUS: f32 = 11.0;
pub const INNER_RADIUS: f32 = 9.0;
pub const CONTROL_RADIUS: f32 = 7.0;
pub const ITEM_RADIUS: f32 = 8.0;
pub const CHIP_RADIUS: f32 = 6.0;
pub const STEP_CHIP_RADIUS: f32 = 4.0;
pub const NOTE_RADIUS: f32 = 10.0;
pub const SWATCH_PX: f32 = 24.0;
pub const SWATCH_RADIUS: f32 = 6.0;
pub const TOGGLE_PX: f32 = 22.0;
pub const STEPPER_HEIGHT: f32 = 28.0;
pub const STEPPER_BUTTON_W: f32 = 30.0;
pub const STEPPER_VALUE_W: f32 = 40.0;
pub const SMALL_BUTTON_H: f32 = 30.0;
pub const BUTTON_H: f32 = 32.0;
pub const PRIMARY_H: f32 = 36.0;
pub const SLIDER_W: f32 = 200.0;
pub const VALUE_W: f32 = 52.0;
pub const DOT_PX: f32 = 7.0;
pub const SECTION_LABEL_GAP: u16 = 9;
pub const HEADING_GAP: u16 = 3;
pub const PANE_GAP: u16 = 22;

const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

pub const CONTENT_PAD: Padding = pad(20.0, 24.0, 28.0, 24.0);
pub const SIDEBAR_PAD: Padding = pad(10.0, 8.0, 10.0, 8.0);
pub const SIDEBAR_ITEM_PAD: Padding = pad(8.0, 10.0, 8.0, 10.0);
pub const SIDEBAR_FOOTER_PAD: Padding = pad(9.0, 10.0, 9.0, 10.0);
/// A settings row — `12px 16px`.
pub const ROW_PAD: Padding = pad(12.0, 16.0, 12.0, 16.0);
/// A tighter row — `11px 16px`.
pub const ROW_TIGHT_PAD: Padding = pad(11.0, 16.0, 11.0, 16.0);
/// A card footer with a button — `10px 16px`.
pub const CARD_FOOTER_PAD: Padding = pad(10.0, 16.0, 10.0, 16.0);
/// An inner card — `10px 12px`.
pub const INNER_PAD: Padding = pad(10.0, 12.0, 10.0, 12.0);
/// A described choice card (Floating / Docked) — `10px 12px`.
pub const CHOICE_CARD_PAD: Padding = pad(10.0, 12.0, 10.0, 12.0);
/// A pill — `6px 12px`.
pub const PILL_PAD: Padding = pad(6.0, 12.0, 6.0, 12.0);
/// A key chip — `4px 9px`.
pub const CHIP_PAD: Padding = pad(4.0, 9.0, 4.0, 9.0);
/// A numbered step chip — `2px 6px`.
pub const STEP_CHIP_PAD: Padding = pad(2.0, 6.0, 2.0, 6.0);
/// An info note — `12px 14px`.
pub const NOTE_PAD: Padding = pad(12.0, 14.0, 12.0, 14.0);
/// A tinted panel — `12px 13px`.
pub const PANEL_PAD: Padding = pad(12.0, 13.0, 12.0, 13.0);
/// The preview strip — `16px 16px 14px`.
pub const PREVIEW_PAD: Padding = pad(16.0, 16.0, 14.0, 16.0);
/// The code block — `12px 13px`.
pub const CODE_PAD: Padding = pad(12.0, 13.0, 12.0, 13.0);
/// A layout row — `12px 14px`.
pub const LAYOUT_ROW_PAD: Padding = pad(12.0, 14.0, 12.0, 14.0);
/// The layouts footer — `13px 14px`.
pub const LAYOUT_FOOTER_PAD: Padding = pad(13.0, 14.0, 13.0, 14.0);
/// A drop zone — `18px 16px`.
pub const DROP_PAD: Padding = pad(18.0, 16.0, 18.0, 16.0);

// ---- typography (px) ---------------------------------------------------------

pub const HEADING: f32 = 17.0;
pub const SUBHEAD: f32 = 12.5;
pub const SECTION_LABEL: f32 = 11.0;
pub const ROW_LABEL: f32 = 13.0;
pub const ROW_HELP: f32 = 11.5;
pub const BODY: f32 = 12.5;
pub const VALUE: f32 = 12.0;
pub const CAPTION: f32 = 10.5;
pub const SIDEBAR_NAME: f32 = 13.0;
pub const SIDEBAR_SUB: f32 = 11.0;
pub const BUTTON: f32 = 12.5;
pub const PRIMARY: f32 = 13.0;
pub const PILL: f32 = 12.0;
pub const CHIP: f32 = 12.0;
pub const STEP_CHIP: f32 = 11.0;
pub const NOTE: f32 = 12.0;
pub const SAVED: f32 = 11.0;
pub const CHOICE_NAME: f32 = 13.0;
pub const CHOICE_DESC: f32 = 11.5;
pub const LAYOUT_NAME: f32 = 13.5;
pub const LAYOUT_META: f32 = 11.0;
pub const STATE_HEADLINE: f32 = 14.0;
pub const STATE_SUB: f32 = 11.5;
pub const FACT_VALUE: f32 = 11.5;
pub const CODE: f32 = 12.0;

// ---- text ----------------------------------------------------------------------

/// The colour roles text takes, resolved against the COSMIC theme inside
/// the widget's style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Ink,
    Secondary,
    Tertiary,
    Accent,
    Success,
    Warning,
    Destructive,
    /// Text on the accent surface (a suggested button).
    OnAccent,
    /// Text on the destructive surface.
    OnDestructive,
    /// A disabled control's label: ink at [`yutani::applet::theme::DISABLED_ALPHA`].
    Disabled,
}

fn text_style(color: Color) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style { color: Some(color), ..Default::default() }
}

impl Role {
    pub fn class(self) -> cosmic_theme::Text {
        cosmic_theme::Text::Custom(match self {
            Role::Ink => |t| text_style(roles::ink(t.cosmic())),
            Role::Secondary => |t| text_style(roles::secondary(t.cosmic())),
            Role::Tertiary => |t| text_style(roles::tertiary(t.cosmic())),
            Role::Accent => |t| text_style(roles::accent(t.cosmic())),
            Role::Success => |t| text_style(roles::success(t.cosmic())),
            Role::Warning => |t| text_style(t.cosmic().warning_color().into()),
            Role::Destructive => |t| text_style(roles::destructive(t.cosmic())),
            Role::OnAccent => |t| text_style(t.cosmic().on_accent_color().into()),
            Role::OnDestructive => |t| text_style(t.cosmic().on_destructive_color().into()),
            Role::Disabled => |t| text_style(roles::dimmed(roles::ink(t.cosmic()))),
        })
    }
}

fn weighted(font: cosmic::font::Font, weight: Weight) -> cosmic::font::Font {
    cosmic::font::Font { weight, ..font }
}

pub fn text<'a, M: 'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, weight: Weight, role: Role) -> Element<'a, M> {
    widget::text(content).size(size).font(weighted(cosmic::font::default(), weight)).class(role.class()).into()
}

pub fn mono<'a, M: 'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, weight: Weight, role: Role) -> Element<'a, M> {
    widget::text(content).size(size).font(weighted(cosmic::font::mono(), weight)).class(role.class()).into()
}

/// Body text that wraps.
pub fn prose<'a, M: 'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, role: Role) -> Element<'a, M> {
    widget::text(content).size(size).font(cosmic::font::default()).class(role.class()).width(Length::Fill).into()
}

/// A pane's heading and subhead.
pub fn heading<'a, M: 'a>(title: &'a str, sub: &'a str) -> Element<'a, M> {
    Column::new()
        .spacing(HEADING_GAP)
        .push(text(title, HEADING, Weight::Semibold, Role::Ink))
        .push(text(sub, SUBHEAD, Weight::Normal, Role::Secondary))
        .into()
}

/// `FRAME`, `ARRANGEMENT`: 11 / 600, uppercase, secondary.
pub fn section_label<'a, M: 'a>(label: &str) -> Element<'a, M> {
    text(label.to_uppercase(), SECTION_LABEL, Weight::Semibold, Role::Secondary)
}

/// A labelled section: the label over its card.
pub fn section<'a, M: 'a>(label: &str, body: impl Into<Element<'a, M>>) -> Element<'a, M> {
    Column::new().width(Length::Fill).spacing(SECTION_LABEL_GAP).push(section_label(label)).push(body).into()
}

// ---- surfaces ------------------------------------------------------------------

fn surface(radius: f32, bg: fn(&Cosmic) -> Color, edge: Option<fn(&Cosmic) -> Color>) -> cosmic_theme::Container<'static> {
    cosmic_theme::Container::custom(move |theme| {
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

/// Sunken surface (code block, drop zone).
fn sunken_bg(c: &Cosmic) -> Color {
    roles::with_alpha(roles::ink(c), 0.04)
}
/// The sidebar's own ground.
fn sidebar_bg(c: &Cosmic) -> Color {
    c.primary_container_color().into()
}

pub fn card_class() -> cosmic_theme::Container<'static> {
    surface(CARD_RADIUS, roles::card_bg, Some(roles::card_border))
}
pub fn inner_class() -> cosmic_theme::Container<'static> {
    surface(INNER_RADIUS, roles::control_bg, Some(roles::card_border))
}
pub fn sunken_class() -> cosmic_theme::Container<'static> {
    surface(INNER_RADIUS, sunken_bg, Some(roles::card_border))
}
pub fn sidebar_class() -> cosmic_theme::Container<'static> {
    cosmic_theme::Container::custom(|theme| container::Style {
        background: Some(Background::Color(sidebar_bg(theme.cosmic()))),
        ..Default::default()
    })
}
pub fn sidebar_footer_class() -> cosmic_theme::Container<'static> {
    surface(ITEM_RADIUS, roles::control_bg, Some(roles::card_border))
}
pub fn hairline_class() -> cosmic_theme::Container<'static> {
    cosmic_theme::Container::custom(|theme| container::Style {
        background: Some(Background::Color(roles::divider(theme.cosmic()))),
        ..Default::default()
    })
}

/// A 1 px divider between rows.
pub fn hairline<'a, M: 'a>() -> Element<'a, M> {
    widget::container(widget::space().width(Length::Fill).height(Length::Fixed(1.0))).class(hairline_class()).into()
}

/// A card of rows with dividers between them.
pub fn card<'a, M: 'a>(rows: Vec<Element<'a, M>>) -> Element<'a, M> {
    let mut column = Column::new().width(Length::Fill);
    for (i, row) in rows.into_iter().enumerate() {
        if i > 0 {
            column = column.push(hairline());
        }
        column = column.push(row);
    }
    widget::container(column).width(Length::Fill).class(card_class()).into()
}

/// Tints for panels, dots and small buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Success,
    Destructive,
}

fn tint_color(c: &Cosmic, tint: Tint) -> Color {
    match tint {
        Tint::Accent => roles::accent(c),
        Tint::Success => roles::success(c),
        Tint::Destructive => roles::destructive(c),
    }
}

/// A tinted surface: the colour at 12 % with a 40 % edge.
pub fn panel_class(tint: Tint, radius: f32) -> cosmic_theme::Container<'static> {
    cosmic_theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        let color = tint_color(c, tint);
        container::Style {
            background: Some(Background::Color(roles::with_alpha(color, 0.12))),
            border: Border { radius: Radius::from(radius), width: 1.0, color: roles::with_alpha(color, 0.40) },
            ..Default::default()
        }
    })
}

/// A tinted panel with content.
pub fn panel<'a, M: 'a>(tint: Tint, content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    widget::container(content).width(Length::Fill).padding(PANEL_PAD).class(panel_class(tint, NOTE_RADIUS)).into()
}

/// The info note: an accent `i` glyph and a wrapping line.
pub fn info_note<'a, M: 'a>(body: &'a str) -> Element<'a, M> {
    let glyph = widget::container(mono("i", NOTE, Weight::Semibold, Role::Accent))
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(chip_class(true));
    widget::container(Row::new().spacing(9).align_y(Alignment::Start).push(glyph).push(prose(body, NOTE, Role::Secondary)))
        .width(Length::Fill)
        .padding(NOTE_PAD)
        .class(panel_class(Tint::Accent, NOTE_RADIUS))
        .into()
}

/// A round dot: the tint, or a dim ink when `on` is false.
pub fn dot<'a, M: 'a>(on: bool, tint: Tint, px: f32) -> Element<'a, M> {
    widget::container(widget::space().width(Length::Fixed(px)).height(Length::Fixed(px)))
        .class(cosmic_theme::Container::custom(move |theme| {
            let c = theme.cosmic();
            container::Style {
                background: Some(Background::Color(if on { tint_color(c, tint) } else { roles::with_alpha(roles::ink(c), 0.25) })),
                border: Border { radius: Radius::from(px / 2.0), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}

/// A dot with the 4 px glow ring behind it.
pub fn glowing_dot<'a, M: 'a>(on: bool, tint: Tint, px: f32) -> Element<'a, M> {
    widget::container(dot(on, tint, px))
        .padding(4)
        .class(cosmic_theme::Container::custom(move |theme| {
            let c = theme.cosmic();
            let color = if on { tint_color(c, tint) } else { roles::with_alpha(roles::ink(c), 0.25) };
            container::Style {
                background: Some(Background::Color(roles::with_alpha(color, 0.35))),
                border: Border { radius: Radius::from(px / 2.0 + 4.0), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}

// ---- rows ----------------------------------------------------------------------

/// A row's label and optional help line.
pub fn label_block<'a, M: 'a>(label: &'a str, help: Option<&'a str>) -> Element<'a, M> {
    let mut col = Column::new().spacing(1).width(Length::Fill).push(text(label, ROW_LABEL, Weight::Normal, Role::Ink));
    if let Some(help) = help {
        col = col.push(prose(help, ROW_HELP, Role::Tertiary));
    }
    col.into()
}

/// A settings row: label block on the left, the control on the right.
pub fn row<'a, M: 'a>(label: &'a str, help: Option<&'a str>, control: impl Into<Element<'a, M>>) -> Element<'a, M> {
    widget::container(
        Row::new().width(Length::Fill).spacing(16).align_y(Alignment::Center).push(label_block(label, help)).push(control),
    )
    .width(Length::Fill)
    .padding(ROW_PAD)
    .into()
}

/// A tighter row (`11px 16px`).
pub fn row_tight<'a, M: 'a>(label: &'a str, help: Option<&'a str>, control: impl Into<Element<'a, M>>) -> Element<'a, M> {
    widget::container(
        Row::new().width(Length::Fill).spacing(14).align_y(Alignment::Center).push(label_block(label, help)).push(control),
    )
    .width(Length::Fill)
    .padding(ROW_TIGHT_PAD)
    .into()
}

// ---- controls -------------------------------------------------------------------

pub fn toggle<'a, M: Clone + 'static>(on: bool, msg: impl Fn(bool) -> M + 'a) -> Element<'a, M> {
    widget::toggler(on).on_toggle(msg).size(TOGGLE_PX).into()
}

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

/// A pill or a choice card: the accent surface when selected, the control
/// surface otherwise.
pub fn pill_class(selected: bool, radius: f32) -> cosmic_theme::Button {
    let style = move |c: &Cosmic, hovered: bool| {
        if selected {
            button_style(roles::ink(c), Some(roles::with_alpha(roles::accent(c), 0.28)), Some(roles::with_alpha(roles::accent(c), 0.55)), radius)
        } else {
            button_style(
                roles::ink(c),
                Some(if hovered { roles::hover(c) } else { roles::control_bg(c) }),
                Some(roles::control_border(c)),
                radius,
            )
        }
    };
    cosmic_theme::Button::Custom {
        active: Box::new(move |_, t| style(t.cosmic(), false)),
        disabled: Box::new(move |t| {
            let c = t.cosmic();
            button_style(roles::dimmed(roles::ink(c)), Some(roles::control_bg(c)), Some(roles::control_border(c)), radius)
        }),
        hovered: Box::new(move |_, t| style(t.cosmic(), true)),
        pressed: Box::new(move |_, t| style(t.cosmic(), true)),
    }
}

/// A row of pills, one selected.
pub fn pills<'a, M: Clone + 'a>(options: &[&'a str], selected: Option<usize>, mono_font: bool, msg: impl Fn(usize) -> M + 'a) -> Element<'a, M> {
    let mut row = Row::new().spacing(4).align_y(Alignment::Center);
    for (i, label) in options.iter().enumerate() {
        let content: Element<'a, M> =
            if mono_font { mono(*label, PILL, Weight::Normal, Role::Ink) } else { text(*label, PILL, Weight::Normal, Role::Ink) };
        row = row.push(
            widget::button::custom(content).padding(PILL_PAD).class(pill_class(selected == Some(i), CONTROL_RADIUS)).on_press(msg(i)),
        );
    }
    row.into()
}

/// A described choice card: name over a description, selected in the accent.
pub fn choice_card<'a, M: Clone + 'a>(name: &'a str, desc: &'a str, selected: bool, msg: M) -> Element<'a, M> {
    widget::button::custom(
        Column::new().spacing(2).width(Length::Fill).push(text(name, CHOICE_NAME, Weight::Medium, Role::Ink)).push(prose(desc, CHOICE_DESC, Role::Secondary)),
    )
    .width(Length::FillPortion(1))
    .padding(CHOICE_CARD_PAD)
    .class(pill_class(selected, INNER_RADIUS))
    .on_press(msg)
    .into()
}

/// A −/value/+ stepper. A bound reached disables its button.
pub fn stepper<'a, M: Clone + 'a>(value: String, dec: Option<M>, inc: Option<M>) -> Element<'a, M> {
    let step = |glyph: &'static str, msg: Option<M>| {
        widget::button::custom(
            widget::container(text(glyph, 15.0, Weight::Normal, Role::Ink))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(Length::Fixed(STEPPER_BUTTON_W))
        .height(Length::Fixed(STEPPER_HEIGHT))
        .padding(0)
        .class(pill_class(false, CONTROL_RADIUS))
        .on_press_maybe(msg)
    };
    Row::new()
        .spacing(2)
        .align_y(Alignment::Center)
        .push(step("−", dec))
        .push(
            widget::container(mono(value, BODY, Weight::Normal, Role::Ink))
                .width(Length::Fixed(STEPPER_VALUE_W))
                .align_x(Horizontal::Center),
        )
        .push(step("+", inc))
        .into()
}

/// One colour swatch: a hex, or `None` for the theme accent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub hex: Option<&'static str>,
}

fn swatch_class(color: Option<[f32; 4]>, selected: bool) -> cosmic_theme::Button {
    let style = move |c: &Cosmic| {
        let fill = match color {
            Some([r, g, b, a]) => Color { r, g, b, a },
            None => roles::accent(c),
        };
        let ring = if selected { roles::with_alpha(roles::ink(c), 0.85) } else { roles::control_border(c) };
        button::Style {
            background: Some(Background::Color(fill)),
            border_radius: Radius::from(SWATCH_RADIUS),
            border_width: if selected { 2.0 } else { 1.0 },
            border_color: ring,
            ..Default::default()
        }
    };
    cosmic_theme::Button::Custom {
        active: Box::new(move |_, t| style(t.cosmic())),
        disabled: Box::new(move |t| style(t.cosmic())),
        hovered: Box::new(move |_, t| style(t.cosmic())),
        pressed: Box::new(move |_, t| style(t.cosmic())),
    }
}

/// A row of swatches plus the selected hex (or `accent`).
pub fn swatches<'a, M: Clone + 'a>(options: &[Swatch], selected: Option<&str>, msg: impl Fn(String) -> M + 'a) -> Element<'a, M> {
    let mut row = Row::new().spacing(5).align_y(Alignment::Center);
    let selected_norm = selected.map(str::to_ascii_lowercase);
    for sw in options {
        let is = match (sw.hex, &selected_norm) {
            (None, None) => true,
            (Some(h), Some(s)) => h.eq_ignore_ascii_case(s),
            _ => false,
        };
        let color = sw.hex.and_then(yutani::model::config::parse_color);
        row = row.push(
            widget::button::custom(widget::space().width(Length::Fixed(SWATCH_PX)).height(Length::Fixed(SWATCH_PX)))
                .padding(0)
                .class(swatch_class(color, is))
                .on_press(msg(sw.hex.unwrap_or("").to_string())),
        );
    }
    let readout = selected.map_or_else(|| "accent".to_string(), |s| s.to_ascii_lowercase());
    row = row.push(widget::container(mono(readout, ROW_HELP, Weight::Normal, Role::Secondary)).width(Length::Fixed(62.0)).padding([0, 0, 0, 5]));
    row.into()
}

pub fn chip_class(accent: bool) -> cosmic_theme::Container<'static> {
    cosmic_theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        container::Style {
            background: Some(Background::Color(if accent { roles::with_alpha(roles::accent(c), 0.25) } else { roles::control_bg(c) })),
            border: Border {
                radius: Radius::from(if accent { STEP_CHIP_RADIUS } else { CHIP_RADIUS }),
                width: 1.0,
                color: if accent { roles::with_alpha(roles::accent(c), 0.45) } else { roles::control_border(c) },
            },
            ..Default::default()
        }
    })
}

/// A mono key chip (`Ctrl + Alt + 1 … 9`).
pub fn chip<'a, M: 'a>(label: String) -> Element<'a, M> {
    widget::container(mono(label, CHIP, Weight::Normal, Role::Ink)).padding(CHIP_PAD).class(chip_class(false)).into()
}

/// A numbered step chip in the accent.
pub fn step_chip<'a, M: 'a>(n: usize) -> Element<'a, M> {
    widget::container(mono(n.to_string(), STEP_CHIP, Weight::Normal, Role::Ink)).padding(STEP_CHIP_PAD).class(chip_class(true)).into()
}

/// A step heading: the chip and a title.
pub fn step_title<'a, M: 'a>(n: usize, title: &'a str) -> Element<'a, M> {
    Row::new().spacing(8).align_y(Alignment::Center).push(step_chip(n)).push(text(title, LAYOUT_NAME, Weight::Medium, Role::Ink)).into()
}

// ---- buttons ---------------------------------------------------------------------

/// A button label. libcosmic's suggested / destructive buttons expect
/// their own on-colour ink, which a plain `text` does not inherit, so the
/// role is always explicit.
fn labelled<'a, M: 'a>(label: &'a str, size: f32, weight: Weight, role: Role) -> Element<'a, M> {
    text(label, size, weight, role)
}

fn centred<'a, M: 'a>(content: Element<'a, M>) -> Element<'a, M> {
    widget::container(content).width(Length::Fill).height(Length::Fill).align_x(Horizontal::Center).align_y(Vertical::Center).into()
}

/// The inert treatment: muted surface, dim text, no press.
fn inert_class(radius: f32) -> cosmic_theme::Button {
    let style = move |t: &cosmic::Theme| {
        let c = t.cosmic();
        button_style(roles::dimmed(roles::ink(c)), Some(roles::with_alpha(roles::ink(c), 0.04)), Some(roles::card_border(c)), radius)
    };
    cosmic_theme::Button::Custom {
        active: Box::new(move |_, t| style(t)),
        disabled: Box::new(style),
        hovered: Box::new(move |_, t| style(t)),
        pressed: Box::new(move |_, t| style(t)),
    }
}

/// The accent primary button (`Copy to N other characters`, `Install
/// tunnel`); inert when there is nothing to press.
pub fn primary_button<'a, M: Clone + 'a>(label: &'a str, msg: Option<M>) -> Element<'a, M> {
    let (class, role) = match msg {
        Some(_) => (cosmic_theme::Button::Suggested, Role::OnAccent),
        None => (inert_class(INNER_RADIUS), Role::Disabled),
    };
    widget::button::custom(centred(labelled(label, PRIMARY, Weight::Semibold, role)))
        .height(Length::Fixed(PRIMARY_H))
        .padding([0, 16])
        .class(class)
        .on_press_maybe(msg)
        .into()
}

/// [`primary_button`] with an owned label.
pub fn primary_button_owned<'a, M: Clone + 'a>(label: String, msg: Option<M>) -> Element<'a, M> {
    let (class, role) = match msg {
        Some(_) => (cosmic_theme::Button::Suggested, Role::OnAccent),
        None => (inert_class(INNER_RADIUS), Role::Disabled),
    };
    let content: Element<'a, M> = text(label, PRIMARY, Weight::Semibold, role);
    widget::button::custom(centred(content))
        .height(Length::Fixed(PRIMARY_H))
        .padding([0, 16])
        .class(class)
        .on_press_maybe(msg)
        .into()
}

/// A standard button (`Cancel`, `Browse…`, `Restore`).
pub fn standard_button<'a, M: Clone + 'a>(label: &'a str, msg: Option<M>) -> Element<'a, M> {
    let role = if msg.is_some() { Role::Ink } else { Role::Disabled };
    widget::button::custom(centred(labelled(label, BUTTON, Weight::Normal, role)))
        .height(Length::Fixed(BUTTON_H))
        .padding([0, 13])
        .class(pill_class(false, ITEM_RADIUS))
        .on_press_maybe(msg)
        .into()
}

/// A standard button in the accent surface (`Apply`, `Rename`).
pub fn accent_button<'a, M: Clone + 'a>(label: &'a str, msg: Option<M>) -> Element<'a, M> {
    let role = if msg.is_some() { Role::Ink } else { Role::Disabled };
    widget::button::custom(centred(labelled(label, BUTTON, Weight::Medium, role)))
        .height(Length::Fixed(SMALL_BUTTON_H))
        .padding([0, 14])
        .class(pill_class(true, ITEM_RADIUS))
        .on_press_maybe(msg)
        .into()
}

/// libcosmic's destructive button.
pub fn destructive_button<'a, M: Clone + 'a>(label: &'a str, msg: Option<M>) -> Element<'a, M> {
    let role = if msg.is_some() { Role::OnDestructive } else { Role::Disabled };
    widget::button::custom(centred(labelled(label, BUTTON, Weight::Semibold, role)))
        .height(Length::Fixed(BUTTON_H))
        .padding([0, 12])
        .class(cosmic_theme::Button::Destructive)
        .on_press_maybe(msg)
        .into()
}

/// An outline button in destructive text (`Uninstall…`, `Delete…`).
pub fn destructive_outline_button<'a, M: Clone + 'a>(label: &'a str, msg: Option<M>) -> Element<'a, M> {
    let class = cosmic_theme::Button::Custom {
        active: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(roles::destructive(c), None, Some(roles::with_alpha(roles::destructive(c), 0.5)), ITEM_RADIUS)
        }),
        disabled: Box::new(|t| {
            let c = t.cosmic();
            button_style(roles::dimmed(roles::destructive(c)), None, Some(roles::with_alpha(roles::destructive(c), 0.25)), ITEM_RADIUS)
        }),
        hovered: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(roles::destructive(c), Some(roles::with_alpha(roles::destructive(c), 0.12)), Some(roles::with_alpha(roles::destructive(c), 0.5)), ITEM_RADIUS)
        }),
        pressed: Box::new(|_, t| {
            let c = t.cosmic();
            button_style(roles::destructive(c), Some(roles::with_alpha(roles::destructive(c), 0.18)), Some(roles::with_alpha(roles::destructive(c), 0.5)), ITEM_RADIUS)
        }),
    };
    let role = if msg.is_some() { Role::Destructive } else { Role::Disabled };
    widget::button::custom(centred(labelled(label, BUTTON, Weight::Medium, role)))
        .height(Length::Fixed(BUTTON_H))
        .padding([0, 13])
        .class(class)
        .on_press_maybe(msg)
        .into()
}

/// A small square glyph button (`⋯`, `↻`).
pub fn glyph_button<'a, M: Clone + 'a>(glyph: &'a str, px: f32, msg: Option<M>) -> Element<'a, M> {
    let role = if msg.is_some() { Role::Ink } else { Role::Disabled };
    widget::button::custom(centred(mono(glyph, 13.0, Weight::Normal, role)))
        .width(Length::Fixed(px))
        .height(Length::Fixed(px))
        .padding(0)
        .class(pill_class(false, ITEM_RADIUS))
        .on_press_maybe(msg)
        .into()
}

/// A small text-link-like button (`Dismiss`).
pub fn quiet_button<'a, M: Clone + 'a>(label: &'a str, msg: M) -> Element<'a, M> {
    let class = cosmic_theme::Button::Custom {
        active: Box::new(|_, t| button_style(roles::secondary(t.cosmic()), None, None, CHIP_RADIUS)),
        disabled: Box::new(|t| button_style(roles::dimmed(roles::ink(t.cosmic())), None, None, CHIP_RADIUS)),
        hovered: Box::new(|_, t| button_style(roles::ink(t.cosmic()), Some(roles::hover(t.cosmic())), None, CHIP_RADIUS)),
        pressed: Box::new(|_, t| button_style(roles::ink(t.cosmic()), Some(roles::hover(t.cosmic())), None, CHIP_RADIUS)),
    };
    widget::button::custom(text(label, NOTE, Weight::Normal, Role::Secondary)).padding([3, 8]).class(class).on_press(msg).into()
}

/// A menu item inside an inline menu (`Rename…`, `Duplicate`, `Delete…`).
pub fn menu_item<'a, M: Clone + 'a>(label: &'a str, danger: bool, msg: M) -> Element<'a, M> {
    let role = if danger { Role::Destructive } else { Role::Ink };
    let class = cosmic_theme::Button::Custom {
        active: Box::new(|_, t| button_style(roles::ink(t.cosmic()), None, None, CHIP_RADIUS)),
        disabled: Box::new(|t| button_style(roles::dimmed(roles::ink(t.cosmic())), None, None, CHIP_RADIUS)),
        hovered: Box::new(|_, t| button_style(roles::ink(t.cosmic()), Some(roles::hover(t.cosmic())), None, CHIP_RADIUS)),
        pressed: Box::new(|_, t| button_style(roles::ink(t.cosmic()), Some(roles::hover(t.cosmic())), None, CHIP_RADIUS)),
    };
    widget::button::custom(
        widget::container(text(label, BUTTON, Weight::Normal, role)).width(Length::Fill).height(Length::Fill).align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(SMALL_BUTTON_H))
    .padding([0, 10])
    .class(class)
    .on_press(msg)
    .into()
}

// ---- sidebar ----------------------------------------------------------------------

fn sidebar_item_class(selected: bool) -> cosmic_theme::Button {
    cosmic_theme::Button::Custom {
        active: Box::new(move |_, t| {
            let c = t.cosmic();
            button_style(roles::ink(c), selected.then(|| roles::with_alpha(roles::accent(c), 0.22)), None, ITEM_RADIUS)
        }),
        disabled: Box::new(|t| button_style(roles::dimmed(roles::ink(t.cosmic())), None, None, ITEM_RADIUS)),
        hovered: Box::new(move |_, t| {
            let c = t.cosmic();
            button_style(roles::ink(c), Some(if selected { roles::with_alpha(roles::accent(c), 0.22) } else { roles::hover(c) }), None, ITEM_RADIUS)
        }),
        pressed: Box::new(|_, t| button_style(roles::ink(t.cosmic()), Some(roles::hover(t.cosmic())), None, ITEM_RADIUS)),
    }
}

/// A two-line sidebar item: the pane's name over its live sublabel.
pub fn sidebar_item<'a, M: Clone + 'a>(name: &'a str, sub: String, selected: bool, msg: M) -> Element<'a, M> {
    let sub_role = if selected { Role::Accent } else { Role::Tertiary };
    widget::button::custom(
        Column::new()
            .spacing(1)
            .width(Length::Fill)
            .push(text(name, SIDEBAR_NAME, Weight::Medium, if selected { Role::Ink } else { Role::Ink }))
            .push(text(sub, SIDEBAR_SUB, Weight::Normal, sub_role)),
    )
    .width(Length::Fill)
    .padding(SIDEBAR_ITEM_PAD)
    .class(sidebar_item_class(selected))
    .on_press(msg)
    .into()
}

/// The sidebar's footer card: a success dot and one line.
pub fn sidebar_footer<'a, M: 'a>() -> Element<'a, M> {
    widget::container(
        Row::new()
            .spacing(7)
            .align_y(Alignment::Center)
            .push(dot(true, Tint::Success, 6.0))
            .push(prose("Every change saves as you make it.", SIDEBAR_SUB, Role::Secondary)),
    )
    .width(Length::Fill)
    .padding(SIDEBAR_FOOTER_PAD)
    .class(sidebar_footer_class())
    .into()
}
