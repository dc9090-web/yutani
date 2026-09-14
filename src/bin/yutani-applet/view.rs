//! Rendering (redesign spec §2, handoff "Screen 1"). Every size comes from
//! `yutani::applet::theme`, every colour from the COSMIC theme through it,
//! and every string from `yutani::applet::display::Popover`.

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::font::Weight;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, Column, Row};
use cosmic::{Element, theme as cosmic_theme};

use yutani::applet::display::{AccountRow, Popover, Primary, ThumbsButton};
use yutani::applet::icon::icon_state;
use yutani::applet::menu::MenuRow;
use yutani::applet::theme;

use crate::app::{Applet, Msg, Note, close_popup_message, open_popup_message};

// ---- text helpers ---------------------------------------------------------

/// The colour roles text takes, resolved against the COSMIC theme inside
/// the widget's style (`Text::Custom` takes a plain `fn`, hence an enum
/// rather than a closure).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Ink,
    Secondary,
    Tertiary,
    Success,
    Download,
    Destructive,
    /// The state colour: success when on, muted ink when idle.
    State(bool),
}

fn text_style(color: Color) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style { color: Some(color), ..Default::default() }
}

impl Role {
    fn class(self) -> cosmic_theme::Text {
        cosmic_theme::Text::Custom(match self {
            Role::Ink => |t| text_style(theme::ink(t.cosmic())),
            Role::Secondary => |t| text_style(theme::secondary(t.cosmic())),
            Role::Tertiary => |t| text_style(theme::tertiary(t.cosmic())),
            Role::Success => |t| text_style(theme::success(t.cosmic())),
            Role::Download => |t| text_style(theme::download(t.cosmic())),
            Role::Destructive => |t| text_style(theme::destructive(t.cosmic())),
            Role::State(true) => |t| text_style(theme::state_color(t.cosmic(), true)),
            Role::State(false) => |t| text_style(theme::state_color(t.cosmic(), false)),
        })
    }
}

fn weighted(font: cosmic::font::Font, weight: Weight) -> cosmic::font::Font {
    cosmic::font::Font { weight, ..font }
}

/// UI text in a theme role.
fn ui<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, weight: Weight, role: Role) -> Element<'a, Msg> {
    widget::text(content).size(size).font(weighted(cosmic::font::default(), weight)).class(role.class()).into()
}

/// Mono text — every number, so digits do not jitter between ticks.
fn mono<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, weight: Weight, role: Role) -> Element<'a, Msg> {
    widget::text(content).size(size).font(weighted(cosmic::font::mono(), weight)).class(role.class()).into()
}

/// Text in a fixed colour (the violet).
fn fixed<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, weight: Weight, mono_font: bool, color: Color) -> Element<'a, Msg> {
    let font = if mono_font { cosmic::font::mono() } else { cosmic::font::default() };
    widget::text(content).size(size).font(weighted(font, weight)).class(cosmic_theme::Text::Color(color)).into()
}

/// A section label: 10.5 / 600, uppercase, secondary.
fn section_label<'a>(text: &str) -> Element<'a, Msg> {
    ui(text.to_uppercase(), theme::SECTION_LABEL_SIZE, Weight::Semibold, Role::Secondary)
}

fn hairline<'a>(padding: cosmic::iced::Padding) -> Element<'a, Msg> {
    widget::container(widget::container(widget::space().width(Length::Fill).height(Length::Fixed(1.0))).class(theme::hairline_class()))
        .width(Length::Fill)
        .padding(padding)
        .into()
}

fn card<'a>(content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    widget::container(widget::container(content).width(Length::Fill).class(theme::card_class()))
        .width(Length::Fill)
        .padding(theme::CARD_MARGIN)
        .into()
}

// ---- panel button ---------------------------------------------------------

/// The panel button: the mark, tinted by the panel, with the state's badge
/// over its bottom-right corner (`IconState::badge`).
pub fn panel_button(state: &Applet) -> Element<'_, Msg> {
    let clients = state.status.as_ref().map_or(0, |s| s.clients.len());
    let icon = icon_state(state.status.as_ref().map(|s| &s.tunnel), clients, state.pending());
    let (w, h) = state.core.applet.suggested_size(true);
    let mark = widget::icon(widget::icon::from_svg_bytes(icon.bytes(h)).symbolic(true))
        .class(theme::mark_class())
        .width(Length::Fixed(f32::from(w)))
        .height(Length::Fixed(f32::from(h)))
        .opacity(icon.opacity());
    let content: Element<'_, Msg> = if let Some(badge) = icon.badge() {
        let d = theme::badge_px(f32::from(h));
        let badge = widget::container(widget::space().width(Length::Fixed(d)).height(Length::Fixed(d)))
            .class(theme::badge_class(d, badge));
        cosmic::iced::widget::stack([
            mark.into(),
            widget::container(badge).width(Length::Fill).height(Length::Fill).align_x(Horizontal::Right).align_y(Vertical::Bottom).into(),
        ])
        .into()
    } else {
        mark.into()
    };
    let open = state.popup;
    state
        .core
        .applet
        .button_from_element(content, true)
        .on_press_with_rectangle(move |offset, bounds| match open {
            Some(id) => close_popup_message(id),
            None => open_popup_message(bounds, offset),
        })
        .into()
}

// ---- 1. header --------------------------------------------------------------

fn header<'a>(p: &Popover) -> Element<'a, Msg> {
    let badge = widget::container(fixed("Y", theme::BADGE_GLYPH_SIZE, Weight::Semibold, true, theme::VIOLET))
        .width(Length::Fixed(theme::BADGE_PX))
        .height(Length::Fixed(theme::BADGE_PX))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(theme::plate_class());
    let left = Row::new()
        .spacing(theme::HEADER_GAP)
        .align_y(Alignment::Center)
        .push(badge)
        .push(ui("Yutani", theme::TITLE_SIZE, Weight::Semibold, Role::Ink));
    let right = Row::new()
        .spacing(7)
        .align_y(Alignment::Center)
        .push(ui(p.state_word, theme::STATE_WORD_SIZE, Weight::Medium, Role::State(p.running)))
        .push(widget::toggler(p.running).on_toggle(Msg::ToggleService).size(theme::TOGGLE_PX));
    Row::new()
        .width(Length::Fill)
        .padding(theme::HEADER_PAD)
        .align_y(Alignment::Center)
        .push(left)
        .push(widget::space().width(Length::Fill))
        .push(right)
        .into()
}

// ---- 2. accounts card -------------------------------------------------------

fn account_row<'a>(row: &AccountRow) -> Element<'a, Msg> {
    let chip = widget::container(fixed_or_ink_index(row))
        .width(Length::Fixed(theme::INDEX_CHIP_WIDTH))
        .padding([2, 0])
        .align_x(Horizontal::Center)
        .class(theme::index_chip_class(row.focused));
    let mut content = Row::new()
        .width(Length::Fill)
        .spacing(theme::ACCOUNT_ROW_GAP)
        .align_y(Alignment::Center)
        .push(chip)
        .push(widget::container(ui(row.name.clone(), theme::ACCOUNT_NAME_SIZE, Weight::Normal, Role::Ink)).width(Length::Fill).clip(true));
    if row.focused {
        content = content.push(fixed("focused", theme::FOCUSED_SIZE, Weight::Normal, true, theme::VIOLET));
    }
    widget::button::custom(content)
        .width(Length::Fill)
        .height(Length::Fixed(theme::ACCOUNT_ROW_HEIGHT))
        .padding(theme::ACCOUNT_ROW_PAD)
        .class(theme::account_row_class(row.focused))
        .on_press(Msg::Press(yutani::applet::Action::Focus(row.index)))
        .into()
}

/// The index chip's digit: accent-tinted on the focused row, secondary
/// otherwise.
fn fixed_or_ink_index<'a>(row: &AccountRow) -> Element<'a, Msg> {
    let role = if row.focused { Role::Ink } else { Role::Secondary };
    mono(row.index.to_string(), theme::INDEX_SIZE, Weight::Normal, role)
}

fn accounts_card<'a>(p: &Popover) -> Element<'a, Msg> {
    let header = Row::new()
        .width(Length::Fill)
        .padding(theme::CARD_HEADER_PAD)
        .align_y(Alignment::End)
        .push(section_label("Accounts connected"))
        .push(widget::space().width(Length::Fill))
        .push(mono(p.count.clone(), theme::COUNT_SIZE, Weight::Normal, Role::State(p.running)));
    let body: Element<'a, Msg> = if p.running {
        Column::with_children(p.accounts.iter().map(account_row).collect::<Vec<_>>())
            .width(Length::Fill)
            .padding(theme::ACCOUNT_LIST_PAD)
            .into()
    } else {
        Column::new()
            .width(Length::Fill)
            .spacing(3)
            .padding(theme::STOPPED_PAD)
            .push(ui("Service stopped", theme::STOPPED_SIZE, Weight::Normal, Role::Secondary))
            .push(ui("Thumbnails, hotkeys and tunnel routing are inactive.", theme::STOPPED_HELP_SIZE, Weight::Normal, Role::Tertiary))
            .into()
    };
    let thumbs = widget::button::custom(ui(p.thumbs.label(), theme::SMALL_BUTTON_SIZE, Weight::Normal, Role::Ink))
        .padding(theme::SMALL_BUTTON_PAD)
        .class(theme::small_button_class(p.thumbs == ThumbsButton::Hide))
        .on_press_maybe(p.thumbs.action().map(Msg::Press));
    let footer = Row::new()
        .width(Length::Fill)
        .padding(theme::CARD_FOOTER_PAD)
        .spacing(8)
        .align_y(Alignment::Center)
        .push(mono(p.hint.clone(), theme::HINT_SIZE, Weight::Normal, Role::Tertiary))
        .push(widget::space().width(Length::Fill))
        .push(thumbs);
    card(Column::new().width(Length::Fill).push(header).push(body).push(hairline([0, 0].into())).push(footer))
}

// ---- 3–4. tunnel section -----------------------------------------------------

fn tunnel_section<'a>(p: &Popover) -> Element<'a, Msg> {
    let label = Row::new()
        .width(Length::Fill)
        .padding(theme::SECTION_LABEL_PAD)
        .align_y(Alignment::End)
        .push(section_label("WireGuard tunnel"))
        .push(widget::space().width(Length::Fill))
        .push(ui("EVE traffic only", theme::EVE_ONLY_SIZE, Weight::Normal, Role::Tertiary));
    let dot = widget::container(
        widget::container(widget::space().width(Length::Fixed(theme::STATUS_DOT_PX)).height(Length::Fixed(theme::STATUS_DOT_PX)))
            .class(theme::dot_class(p.tunnel.on)),
    )
    .padding(theme::STATUS_GLOW_PX)
    .class(theme::glow_class(p.tunnel.on));
    let left = Row::new()
        .spacing(theme::STATUS_GAP)
        .align_y(Alignment::Center)
        .push(dot)
        .push(ui("WireGuard", theme::TUNNEL_NAME_SIZE, Weight::Semibold, Role::Ink))
        .push(ui(p.tunnel.location.clone(), theme::LOCATION_SIZE, Weight::Normal, Role::Secondary));
    let line = Row::new()
        .width(Length::Fill)
        .padding(theme::TUNNEL_LINE_PAD)
        .align_y(Alignment::Center)
        .push(left)
        .push(widget::space().width(Length::Fill))
        .push(mono(p.tunnel.uptime.clone(), theme::UPTIME_SIZE, Weight::Normal, Role::State(p.tunnel.on)));
    Column::new().width(Length::Fill).push(hairline(theme::DIVIDER_PAD)).push(label).push(line).into()
}

// ---- 5. throughput card ------------------------------------------------------

fn rate_block<'a>(label: &'static str, upload: bool, rate: &(String, &'static str)) -> Element<'a, Msg> {
    let series = if upload { Role::Success } else { Role::Download };
    let numbers = Row::new()
        .spacing(3)
        .align_y(Alignment::End)
        .push(mono(rate.0.clone(), theme::RATE_SIZE, Weight::Medium, Role::Ink))
        .push(mono(rate.1, theme::RATE_UNIT_SIZE, Weight::Normal, Role::Tertiary));
    Column::new()
        .spacing(3)
        .align_x(if upload { Alignment::Start } else { Alignment::End })
        .push(ui(label, theme::RATE_LABEL_SIZE, Weight::Semibold, series))
        .push(numbers)
        .into()
}

fn bar<'a>(value: f32, upload: bool) -> Element<'a, Msg> {
    let h = (value * theme::GRAPH_HALF).round().max(1.0);
    widget::container(widget::space().width(Length::Fill).height(Length::Fixed(h))).width(Length::Fill).class(theme::bar_class(upload)).into()
}

fn graph<'a>(bars: &[(f32, f32)]) -> Element<'a, Msg> {
    let columns = bars.iter().map(|(up, down)| {
        Column::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .push(widget::container(bar(*up, true)).width(Length::Fill).height(Length::Fill).align_y(Vertical::Bottom))
            .push(widget::container(widget::space().width(Length::Fill).height(Length::Fixed(1.0))).class(theme::axis_class()))
            .push(widget::container(bar(*down, false)).width(Length::Fill).height(Length::Fill).align_y(Vertical::Top))
            .into()
    });
    Row::with_children(columns.collect::<Vec<Element<'a, Msg>>>())
        .width(Length::Fill)
        .height(Length::Fixed(theme::GRAPH_HEIGHT))
        .spacing(theme::GRAPH_GAP)
        .into()
}

fn throughput_card<'a>(p: &Popover) -> Element<'a, Msg> {
    let t = &p.throughput;
    let header = Row::new()
        .width(Length::Fill)
        .padding(theme::GRAPH_HEADER_PAD)
        .align_y(Alignment::Start)
        .push(rate_block("↑ Upload", true, &t.up))
        .push(widget::space().width(Length::Fill))
        .push(rate_block("↓ Download", false, &t.down));
    let footer = Row::new()
        .width(Length::Fill)
        .padding(theme::GRAPH_FOOTER_PAD)
        .push(mono(format!("{} sent", t.sent), theme::GRAPH_FOOTER_SIZE, Weight::Normal, Role::Tertiary))
        .push(widget::space().width(Length::Fill))
        .push(mono("60 s", theme::GRAPH_FOOTER_SIZE, Weight::Normal, Role::Tertiary))
        .push(widget::space().width(Length::Fill))
        .push(mono(format!("{} received", t.received), theme::GRAPH_FOOTER_SIZE, Weight::Normal, Role::Tertiary));
    card(
        Column::new()
            .width(Length::Fill)
            .push(header)
            .push(widget::container(graph(&t.bars)).width(Length::Fill).padding(theme::GRAPH_PAD))
            .push(footer),
    )
}

// ---- 6. fact tiles -----------------------------------------------------------

fn tiles<'a>(p: &Popover) -> Element<'a, Msg> {
    let tile = |label: &'static str, value: String| {
        widget::container(
            Column::new()
                .spacing(3)
                .push(section_label(label))
                .push(widget::container(mono(value, theme::TILE_VALUE_SIZE, Weight::Normal, Role::Ink)).width(Length::Fill).clip(true)),
        )
        .width(Length::FillPortion(1))
        .padding(theme::TILE_PAD)
        .class(theme::tile_class())
    };
    let [(l0, v0), (l1, v1)] = &p.tiles;
    widget::container(Row::new().width(Length::Fill).spacing(theme::TILE_GAP).push(tile(l0, v0.clone())).push(tile(l1, v1.clone())))
        .width(Length::Fill)
        .padding(theme::CARD_MARGIN)
        .into()
}

// ---- 7. action row -----------------------------------------------------------

fn action_row<'a>(p: &Popover, menu_open: bool) -> Element<'a, Msg> {
    let label = ui(p.primary.label(), theme::PRIMARY_SIZE, Weight::Semibold, Role::Ink);
    let primary = match p.primary {
        Primary::Inert(_) => widget::button::custom(widget::container(label).width(Length::Fill).align_x(Horizontal::Center))
            .class(theme::inert_primary_class()),
        Primary::Standard(..) => widget::button::custom(widget::container(label).width(Length::Fill).align_x(Horizontal::Center))
            .class(cosmic_theme::Button::Standard),
        Primary::Accent(..) => {
            // The suggested button paints its own on-accent text.
            let label = widget::text(p.primary.label()).size(theme::PRIMARY_SIZE).font(weighted(cosmic::font::default(), Weight::Semibold));
            widget::button::custom(widget::container(label).width(Length::Fill).align_x(Horizontal::Center))
                .class(cosmic_theme::Button::Suggested)
        }
    }
    .width(Length::Fill)
    .height(Length::Fixed(theme::PRIMARY_HEIGHT))
    .on_press_maybe(p.primary.action().map(Msg::Press));
    let overflow = widget::button::custom(
        widget::container(mono("⋯", theme::OVERFLOW_SIZE, Weight::Normal, Role::Ink)).width(Length::Fill).align_x(Horizontal::Center),
    )
    .width(Length::Fixed(theme::OVERFLOW_PX))
    .height(Length::Fixed(theme::OVERFLOW_PX))
    .class(theme::overflow_class(menu_open))
    .on_press(Msg::ToggleMenu);
    widget::container(Row::new().width(Length::Fill).spacing(theme::ACTION_GAP).push(primary).push(overflow))
        .width(Length::Fill)
        .padding(theme::CARD_MARGIN)
        .into()
}

fn note_line<'a>(note: &Note) -> Element<'a, Msg> {
    let role = if note.progress { Role::Tertiary } else { Role::Destructive };
    widget::container(mono(note.text.clone(), theme::NOTE_SIZE, Weight::Normal, role)).width(Length::Fill).padding(theme::NOTE_PAD).into()
}

// ---- 8. menu -----------------------------------------------------------------

fn menu_row<'a>(row: &MenuRow) -> Element<'a, Msg> {
    let role = if row.danger { Role::Destructive } else { Role::Ink };
    let mut content = Row::new()
        .width(Length::Fill)
        .spacing(10)
        .align_y(Alignment::Center)
        .push(ui(row.label, theme::MENU_SIZE, Weight::Normal, role))
        .push(widget::space().width(Length::Fill));
    if let Some(hint) = &row.hint {
        content = content.push(mono(hint.clone(), theme::MENU_HINT_SIZE, Weight::Normal, Role::Tertiary));
    }
    widget::button::custom(content)
        .width(Length::Fill)
        .height(Length::Fixed(theme::MENU_ROW_HEIGHT))
        .padding(theme::MENU_ROW_PAD)
        .class(theme::menu_row_class(row.danger))
        .on_press_maybe(row.action.map(Msg::Press))
        .into()
}

fn menu<'a>(running: bool) -> Element<'a, Msg> {
    let rows = yutani::applet::menu::rows(running);
    Column::new()
        .width(Length::Fill)
        .push(hairline([0, 0].into()))
        .push(
            Column::with_children(rows.iter().map(menu_row).collect::<Vec<_>>())
                .width(Length::Fill)
                .padding(theme::MENU_PAD),
        )
        .into()
}

// ---- the popover ----------------------------------------------------------

/// The popover's contents. libcosmic's `popup_container` supplies the
/// surface, its theme background, blur and shadow; everything here sits on
/// the theme's own colours, so a light COSMIC theme gets a light popover.
pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let p = state.popover();
    let mut content = Column::new()
        .width(Length::Fill)
        .push(header(&p))
        .push(accounts_card(&p))
        .push(tunnel_section(&p))
        .push(throughput_card(&p))
        .push(tiles(&p))
        .push(action_row(&p, state.menu_open));
    if let Some(note) = state.visible_note() {
        content = content.push(note_line(note));
    }
    if state.menu_open {
        content = content.push(menu(p.running));
    }
    widget::container(content).width(Length::Fixed(theme::POPOVER_WIDTH as f32)).into()
}
