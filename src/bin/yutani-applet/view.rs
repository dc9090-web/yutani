//! Rendering. Every colour and size comes from `yutani::applet::theme`;
//! every string comes from `yutani::applet::display::Display`.

use cosmic::iced::font::Weight;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, Column, Row};
use cosmic::{Element, theme as cosmic_theme};

use yutani::applet::display::Display;
use yutani::applet::icon::icon_state;
use yutani::applet::theme;
use yutani::assets;

use crate::app::{Applet, Msg, close_popup_message, open_popup_message};

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

/// The Y mark in the panel: the state's icon, tinted by the panel theme
/// (`symbolic(true)` + `applet::style()`'s `icon_color`), dimmed to 38 %
/// when there is no daemon or no tunnel.
pub fn panel_button(state: &Applet) -> Element<'_, Msg> {
    let icon = icon_state(state.status.as_ref().map(|s| &s.tunnel), state.pending());
    let (w, h) = state.core.applet.suggested_size(true);
    let mark = widget::icon(widget::icon::from_svg_bytes(icon.bytes()).symbolic(true))
        .width(Length::Fixed(f32::from(w)))
        .height(Length::Fixed(f32::from(h)))
        .opacity(icon.opacity());
    let open = state.popup;
    state
        .core
        .applet
        .button_from_element(mark, true)
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
    // The dot only glows while connected (handoff: no glow when down).
    let dot = widget::container(
        widget::space()
            .width(Length::Fixed(theme::DOT_PX))
            .height(Length::Fixed(theme::DOT_PX)),
    )
    .class(theme::dot_class(dot_color, d.connected));

    let status_row = Row::new()
        .spacing(theme::STATUS_GAP)
        .align_y(Alignment::Center)
        .push(dot)
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
            widget::icon(widget::icon::from_svg_bytes(assets::Y_SYMBOLIC))
                .class(theme::svg_class(theme::TEXT_ON_SURFACE))
                .width(Length::Fixed(f32::from(theme::MARK_PX)))
                .height(Length::Fixed(f32::from(theme::MARK_PX))),
        )
        .push(titles)
        .push(widget::space().width(Length::Fill))
        .push(chip)
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
        .spacing(theme::HEADER_COLUMN_GAP)
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

/// The popup's contents, on the handoff's own surface.
///
/// libcosmic's `popup_container` supplies the shell surface, the blur and
/// the shadow, but it paints the *COSMIC theme's* background — which under
/// a light theme would leave this dark-only palette unreadable. So the
/// content sits on `popup_surface_class()`, which covers it.
pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let d = state.display();
    let content = Column::new()
        .width(Length::Fill)
        .spacing(theme::POPUP_PADDING)
        .padding(theme::POPUP_PADDING)
        .push(header(&d))
        .push(divider(theme::DIVIDER_ABOVE_BAND))
        .push(accounts_band(&d))
        .push(divider(theme::DIVIDER_ABOVE_TILES))
        .push(tiles(&d));
    widget::container(content).width(Length::Fill).class(theme::popup_surface_class()).into()
}
