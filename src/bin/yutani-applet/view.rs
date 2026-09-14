//! Rendering. Every colour and size comes from `yutani::applet::theme`;
//! every string comes from `yutani::applet::display::Display`.

use cosmic::iced::font::Weight;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, Column, Row};
use cosmic::{Element, theme as cosmic_theme};

use yutani::applet::display::{Display, Service};
use yutani::applet::icon::icon_state;
use yutani::applet::menu::{MenuRow, RowKind};
use yutani::applet::theme;
use yutani::assets;

use crate::app::{Applet, Msg, Note, close_popup_message, open_popup_message};

/// The handoff's "500" weight (Space Grotesk / JetBrains Mono Medium) for
/// the title, the accounts count and the tile labels.
fn medium(font: cosmic::font::Font) -> cosmic::font::Font {
    cosmic::font::Font { weight: Weight::Medium, ..font }
}

/// UI-font text in one of the handoff's colours.
fn ui<'a>(
    content: impl Into<std::borrow::Cow<'a, str>> + 'a,
    size: f32,
    color: Color,
) -> Element<'a, Msg> {
    widget::text(content)
        .size(size)
        .line_height(theme::LINE_HEIGHT)
        .class(cosmic_theme::Text::Color(color))
        .into()
}

/// A menu row's label. Unlike [`ui`], it clips rather than wraps: a client
/// name is arbitrary user text, and a header like "Accounts…" is fixed
/// English that never needs a second line either — so no row label should
/// ever grow the popup's height by wrapping. It clips instead, at whatever
/// width the trailing hint/count leaves it.
fn row_label<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, color: Color) -> Element<'a, Msg> {
    widget::text(content)
        .size(theme::MENU_SIZE)
        .line_height(theme::LINE_HEIGHT)
        .class(cosmic_theme::Text::Color(color))
        .wrapping(cosmic::iced::widget::text::Wrapping::None)
        .into()
}

/// UI-font text at the handoff's 500 weight.
fn ui_medium<'a>(
    content: impl Into<std::borrow::Cow<'a, str>> + 'a,
    size: f32,
    color: Color,
) -> Element<'a, Msg> {
    widget::text(content)
        .size(size)
        .line_height(theme::LINE_HEIGHT)
        .font(medium(cosmic::font::default()))
        .class(cosmic_theme::Text::Color(color))
        .into()
}

/// Monospace, tabular text — every number, id and rate (handoff: critical,
/// live counters must not jitter).
fn mono<'a>(
    content: impl Into<std::borrow::Cow<'a, str>> + 'a,
    size: f32,
    color: Color,
) -> Element<'a, Msg> {
    widget::text::monotext(content)
        .size(size)
        .line_height(theme::LINE_HEIGHT)
        .class(cosmic_theme::Text::Color(color))
        .into()
}

/// The accounts count: mono, 500, line-height 1.
fn count<'a>(content: String, color: Color) -> Element<'a, Msg> {
    widget::text::monotext(content)
        .size(theme::COUNT_SIZE)
        .line_height(theme::LINE_HEIGHT_TIGHT)
        .font(medium(cosmic::font::mono()))
        .class(cosmic_theme::Text::Color(color))
        .into()
}

/// A round status dot. It glows only where the handoff says it does — the
/// header while connected — so the menu's account dots stay quiet.
fn dot<'a>(color: Color, glow: bool) -> Element<'a, Msg> {
    widget::container(
        widget::space()
            .width(Length::Fixed(theme::DOT_PX))
            .height(Length::Fixed(theme::DOT_PX)),
    )
    .class(theme::dot_class(color, glow))
    .into()
}

fn glyph<'a>(bytes: &'static [u8], w: u16, h: u16) -> Element<'a, Msg> {
    widget::icon(widget::icon::from_svg_bytes(bytes))
        .width(Length::Fixed(f32::from(w)))
        .height(Length::Fixed(f32::from(h)))
        .into()
}

/// A hairline rule with the handoff's margins. `theme::hairline` fills its
/// container, so the container has to be told to fill the popup.
fn divider<'a>(padding: cosmic::iced::Padding) -> Element<'a, Msg> {
    widget::container(theme::hairline::<Msg>())
        .width(Length::Fill)
        .padding(padding)
        .into()
}

/// The Y mark in the panel: the state's icon, tinted pure white on a dark
/// panel and the theme's ink on a light one (`symbolic(true)` +
/// `theme::mark_class`), dimmed to 38 % when there is no daemon or no
/// tunnel. The Active state is the same
/// tinted mark with a blue dot laid over its bottom-right corner
/// (`IconState::badge`): the Y stays the panel's ink, the dot is the news.
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
        // The stack takes the mark's size; the badge layer fills it and
        // parks the dot in the corner.
        let d = theme::badge_px(f32::from(h));
        let badge = widget::container(widget::space().width(Length::Fixed(d)).height(Length::Fixed(d)))
            .class(theme::badge_class(d, badge));
        cosmic::iced::widget::stack([
            mark.into(),
            widget::container(badge)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(cosmic::iced::alignment::Horizontal::Right)
                .align_y(cosmic::iced::alignment::Vertical::Bottom)
                .into(),
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

/// Header: Y mark, "WireGuard", the status dot + text + location, and the
/// interface chip.
///
/// Every string is cloned rather than borrowed: `Display` is built inside
/// `popup`, so a borrowed `Element` could not outlive it. Do not "optimise"
/// these clones away — they are what makes the returned element `'a`-free.
fn header<'a>(d: &Display) -> Element<'a, Msg> {
    let dot_color = if d.connected { theme::ACCENT_UP } else { theme::TEXT_MUTED };
    let status_row = Row::new()
        .spacing(theme::STATUS_GAP)
        .align_y(Alignment::Center)
        // The dot only glows while connected (handoff: no glow when down).
        .push(dot(dot_color, d.connected))
        .push(mono(d.status_text, theme::STATUS_SIZE, dot_color))
        .push(mono(theme::MIDDOT, theme::STATUS_SIZE, theme::SEPARATOR))
        .push(glyph(assets::PIN, theme::PIN_W, theme::PIN_H))
        .push(mono(d.location.clone(), theme::STATUS_SIZE, theme::TEXT_SECONDARY));

    let titles = Column::new()
        .spacing(theme::HEADER_COLUMN_GAP)
        .push(ui_medium(theme::TITLE, theme::TITLE_SIZE, theme::TEXT_PRIMARY))
        .push(status_row);

    let chip = widget::container(mono(d.iface.clone(), theme::CHIP_SIZE, theme::TEXT_FAINT))
        .padding(theme::CHIP_PAD)
        .class(theme::chip_class());

    Row::new()
        .width(Length::Fill)
        .spacing(theme::HEADER_GAP)
        .align_y(Alignment::Center)
        .padding(theme::HEADER_PAD)
        // Explicit `#E6E8EC`, not `symbolic(true)`: the mark sits on the
        // popup's own dark surface, so it must not follow the COSMIC
        // theme's icon colour the way the panel button does.
        .push(
            widget::icon(widget::icon::from_svg_bytes(assets::YUTANI_SYMBOLIC))
                .class(theme::svg_class(theme::TEXT_ON_SURFACE))
                .width(Length::Fixed(f32::from(theme::MARK_PX)))
                .height(Length::Fixed(f32::from(theme::MARK_PX))),
        )
        .push(titles)
        .push(widget::space().width(Length::Fill))
        .push(chip)
        .into()
}

/// One row of the services band: dot, name, and the note against the far
/// edge. Green or red, no glow — the header's dot keeps the glow.
fn service_row<'a>(s: Service) -> Element<'a, Msg> {
    let color = if s.up { theme::SERVICE_UP } else { theme::SERVICE_DOWN };
    Row::new()
        .width(Length::Fill)
        .spacing(theme::MENU_ROW_GAP)
        .align_y(Alignment::Center)
        .push(dot(color, false))
        .push(ui(s.name, theme::SERVICE_LABEL_SIZE, theme::TEXT_ON_SURFACE))
        .push(widget::space().width(Length::Fill))
        .push(mono(s.note, theme::MENU_HINT_SIZE, color))
        .into()
}

/// Services band: Yutani and WireGuard, each with its dot (2026-09-14
/// spec §1). Shown in every state — offline is exactly when "Yutani: not
/// running" is the news.
fn services_band<'a>(d: &Display) -> Element<'a, Msg> {
    Column::new()
        .width(Length::Fill)
        .spacing(theme::SERVICES_ROW_GAP)
        .padding(theme::SERVICES_PAD)
        .push(service_row(d.services[0]))
        .push(service_row(d.services[1]))
        .into()
}

/// Accounts band: the count and its label on the left, the tunnel IP and
/// the handshake age on the right.
fn accounts_band<'a>(d: &Display) -> Element<'a, Msg> {
    let left = Row::new()
        .spacing(theme::COUNT_GAP)
        .align_y(Alignment::Center)
        .push(count(d.accounts.to_string(), theme::TEXT_PRIMARY))
        .push(ui(d.accounts_label, theme::COUNT_LABEL_SIZE, theme::TEXT_SECONDARY));
    let right = Column::new()
        .spacing(theme::BAND_COLUMN_GAP)
        .align_x(Alignment::End)
        .push(mono(d.address.clone(), theme::BAND_RIGHT_SIZE, theme::TEXT_FAINT))
        .push(mono(d.handshake.clone(), theme::BAND_RIGHT_SIZE, theme::TEXT_FAINT));
    Row::new()
        .width(Length::Fill)
        .padding(theme::BAND_PAD)
        .align_y(Alignment::Center)
        .push(left)
        .push(widget::space().width(Length::Fill))
        .push(right)
        .into()
}

/// One traffic tile. No hover state — these are display only.
fn tile<'a>(
    label: &'static str,
    arrow: &'static [u8],
    accent: Color,
    total: String,
    rate: String,
) -> Element<'a, Msg> {
    let label_row = Row::new()
        .spacing(theme::TILE_LABEL_GAP)
        .align_y(Alignment::Center)
        .push(glyph(arrow, theme::GLYPH_PX, theme::GLYPH_PX))
        .push(ui_medium(label, theme::TILE_LABEL_SIZE, theme::TEXT_FAINT));
    widget::container(
        Column::new()
            .spacing(theme::TILE_COLUMN_GAP)
            .push(label_row)
            .push(mono(total, theme::TILE_TOTAL_SIZE, theme::TEXT_PRIMARY))
            .push(mono(rate, theme::TILE_RATE_SIZE, accent)),
    )
    .padding(theme::TILE_PAD)
    .width(Length::FillPortion(1))
    .class(theme::tile_class())
    .into()
}

fn tiles<'a>(d: &Display) -> Element<'a, Msg> {
    Row::new()
        .width(Length::Fill)
        .spacing(theme::TILE_GAP)
        .padding(theme::TILES_PAD)
        .push(tile(
            theme::UPLOAD_LABEL,
            assets::ARROW_UP,
            theme::ACCENT_UP,
            d.up_total.clone(),
            d.up_rate.clone(),
        ))
        .push(tile(
            theme::DOWNLOAD_LABEL,
            assets::ARROW_DOWN,
            theme::ACCENT_DOWN,
            d.down_total.clone(),
            d.down_rate.clone(),
        ))
        .into()
}

/// One menu row. It is taken by value: its strings become the element's, so
/// nothing borrows the list `menu` built them from.
///
/// A row with no action gets no message, which is what makes libcosmic
/// treat the button as disabled — but every label here carries an explicit
/// colour, so libcosmic's `disabled` text style never reaches it and the
/// row has to be dimmed here as well as declared inert.
fn menu_row<'a>(row: MenuRow) -> Element<'a, Msg> {
    let (ink, hover, pressed) = match row.kind {
        RowKind::Danger => (theme::DANGER_TEXT, theme::DANGER_HOVER, theme::DANGER_HOVER),
        _ => (theme::TEXT_ON_SURFACE, theme::HAIRLINE, theme::ACTIVE_FILL),
    };
    let (label_ink, hint_ink) = if row.disabled() {
        (theme::dimmed(ink), theme::dimmed(theme::TEXT_FAINT))
    } else {
        (ink, theme::TEXT_FAINT)
    };

    let mut content = Row::new()
        .width(Length::Fill)
        .spacing(theme::MENU_ROW_GAP)
        .align_y(Alignment::Center);
    if let RowKind::Account { active } = row.kind {
        // The focused account's dot is the accent; the others are muted.
        // No glow — that belongs to the header's connected state alone.
        let marker = if active { theme::ACCENT_UP } else { theme::TEXT_MUTED };
        content = content.push(dot(marker, false));
    }
    content = content
        .push(row_label(row.label, label_ink))
        // Hints and counts are right-aligned against the row's far edge.
        .push(widget::space().width(Length::Fill));
    if let Some(hint) = row.hint {
        content = content.push(mono(hint, theme::MENU_HINT_SIZE, hint_ink));
    }
    if let Some(trailing) = row.trailing {
        content = content.push(mono(trailing, theme::MENU_HINT_SIZE, hint_ink));
    }

    // Character rows are indented a step; the action rows keep the
    // header's left edge.
    let padding = if matches!(row.kind, RowKind::Account { .. }) {
        theme::MENU_ACCOUNT_PAD
    } else {
        theme::MENU_ROW_PAD
    };
    widget::button::custom(content)
        .width(Length::Fill)
        .padding(padding)
        .class(theme::menu_row_class(ink, hover, pressed))
        .on_press_maybe(row.action.map(Msg::Press))
        .into()
}

/// The last `err …` reply as a one-line note, in the danger ink at the hint
/// size so it reads as an aside rather than another row — or, muted, what
/// a slow action is still doing.
fn note_line<'a>(note: &Note) -> Element<'a, Msg> {
    let ink = if note.progress { theme::TEXT_MUTED } else { theme::DANGER_TEXT };
    widget::container(mono(note.text.clone(), theme::MENU_HINT_SIZE, ink))
        .width(Length::Fill)
        .padding(theme::NOTE_PAD)
        .into()
}

/// The expanded client rows, in their own scroll area.
///
/// `popup_container` caps the popup at 1000 px and caps it by *clipping*:
/// without this, a long enough account list would push Preferences…, the
/// thumbnails row and Quit straight off the bottom of the popup with no
/// way to reach them. Only the client rows scroll — the Accounts… header
/// stays outside, so what is scrolling is always labelled.
fn accounts_list(rows: Vec<Element<'_, Msg>>) -> Element<'_, Msg> {
    widget::container(widget::scrollable(
        Column::with_children(rows).width(Length::Fill).spacing(theme::MENU_GAP),
    ))
    .width(Length::Fill)
    .max_height(theme::ACCOUNTS_LIST_MAX_PX)
    .into()
}

/// The menu group: the rows for the current state, the hairline the handoff
/// puts above Quit, and the error note under whichever row earned it
/// (spec §7). A note from a failed poll belongs to no row — it goes above
/// the Quit divider, at the foot of the ordinary rows, rather than under
/// the danger row it has nothing to do with. (If there is no Quit row to
/// anchor to — the offline state — it falls back to the very end.)
///
/// The expanded client rows are a contiguous run in the middle of that
/// list, and they are collected into [`accounts_list`] rather than pushed
/// into the group — everything else keeps its place around them.
fn menu(state: &Applet) -> Element<'_, Msg> {
    let rows = yutani::applet::menu::rows(state.status.as_ref());
    let note = state.visible_note();
    let mut group: Vec<Element<'_, Msg>> = Vec::new();
    let mut clients: Vec<Element<'_, Msg>> = Vec::new();
    let mut placed = false;
    for row in rows {
        let is_client = matches!(row.kind, RowKind::Account { .. });
        // The run has ended: fold it into the group before this row.
        if !is_client && !clients.is_empty() {
            group.push(accounts_list(std::mem::take(&mut clients)));
        }
        if row.kind == RowKind::Danger {
            if let Some(note) = note.filter(|n| n.action.is_none()) {
                group.push(note_line(note));
                placed = true;
            }
            group.push(divider(theme::DIVIDER_ABOVE_MENU));
        }
        let owns_note = note.is_some_and(|n| n.action.is_some() && n.action == row.action);
        let target = if is_client { &mut clients } else { &mut group };
        target.push(menu_row(row));
        if let Some(note) = note.filter(|_| owns_note) {
            target.push(note_line(note));
            placed = true;
        }
    }
    if !clients.is_empty() {
        group.push(accounts_list(std::mem::take(&mut clients)));
    }
    if let Some(note) = note.filter(|_| !placed) {
        group.push(note_line(note));
    }
    Column::with_children(group).width(Length::Fill).spacing(theme::MENU_GAP).into()
}

/// The popup's contents, on the handoff's own surface.
///
/// libcosmic's `popup_container` supplies the shell surface, the blur and
/// the shadow, but it paints the *COSMIC theme's* background — which under
/// a light theme would leave this dark-only palette unreadable. So the
/// content sits on `popup_surface_class()`, which covers it.
pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let d = state.display();
    let mut content = Column::new()
        .width(Length::Fill)
        .spacing(theme::POPUP_PADDING)
        .padding(theme::POPUP_PADDING)
        .push(header(&d))
        .push(divider(theme::DIVIDER_ABOVE_BAND))
        .push(services_band(&d));
    // With no daemon there is nothing more to read: the services band has
    // just said so, then straight to the single "Start Yutani" row. The
    // accounts band and the tiles would be a screenful of dashes and zeroes.
    if d.online {
        content = content
            .push(divider(theme::DIVIDER_ABOVE_BAND))
            .push(accounts_band(&d))
            .push(divider(theme::DIVIDER_ABOVE_TILES))
            .push(tiles(&d));
    }
    content = content.push(divider(theme::DIVIDER_ABOVE_MENU)).push(menu(state));
    widget::container(content).width(Length::Fill).class(theme::popup_surface_class()).into()
}
