//! Rendering. Every colour and size comes from `yutani::applet::theme`;
//! every string comes from `yutani::applet::display::Display`.

use cosmic::iced::font::Weight;
use cosmic::iced::{Alignment, Color, Length, Padding};
use cosmic::widget::{self, Column, Row};
use cosmic::{Element, theme as cosmic_theme};

use yutani::applet::display::Display;
use yutani::applet::icon::icon_state;
use yutani::applet::theme;
use yutani::assets;

use crate::app::{Applet, Msg, close_popup_message, open_popup_message};

// ---- section geometry ----
// The handoff's per-section `padding` / `margin` shorthands, in its own
// order (top, right, bottom, left). They describe *this* arrangement of
// sections rather than a reusable token, so they live next to the only
// code that may use them; every value is on the handoff's 1·2·4·6·8·10·12
// spacing scale.

const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

/// Header — `10px 10px 2px`.
const HEADER_PAD: Padding = pad(10.0, 10.0, 2.0, 10.0);
/// The divider above the accounts band — `6px 10px 2px`.
const DIVIDER_ABOVE_BAND: Padding = pad(6.0, 10.0, 2.0, 10.0);
/// Accounts band — `4px 12px 6px`.
const BAND_PAD: Padding = pad(4.0, 12.0, 6.0, 12.0);
/// The divider above the traffic tiles — `2px 10px 4px`.
const DIVIDER_ABOVE_TILES: Padding = pad(2.0, 10.0, 4.0, 10.0);
/// Traffic tiles — `2px 10px`.
const TILES_PAD: Padding = pad(2.0, 10.0, 2.0, 10.0);
/// The interface chip — `4px 8px`.
const CHIP_PAD: Padding = pad(4.0, 8.0, 4.0, 8.0);
/// One traffic tile — `11px 13px`.
const TILE_PAD: Padding = pad(11.0, 13.0, 11.0, 13.0);
/// Between the accounts count and its label.
const COUNT_GAP: u16 = 8;
/// Between a tile's arrow glyph and its label.
const TILE_LABEL_GAP: u16 = 6;
/// The map pin is the one non-square glyph (handoff: 10×12).
const PIN_SIZE: (u16, u16) = (10, 12);

/// Line box as a factor of the font size. libcosmic's `monotext` preset
/// pins an *absolute* 20 px line height, which at 10.5–22 px text would
/// wreck every gap in the popup, so each helper sets its own.
const LINE: f32 = 1.3;
/// The accounts count is `line-height: 1` in the handoff — its 22 px
/// digits set the height of the whole band.
const LINE_TIGHT: f32 = 1.0;

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
        .line_height(LINE)
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
        .line_height(LINE)
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
        .line_height(LINE)
        .class(cosmic_theme::Text::Color(color))
        .into()
}

/// The accounts count: mono, 500, line-height 1.
fn count<'a>(content: String, color: Color) -> Element<'a, Msg> {
    widget::text::monotext(content)
        .size(theme::COUNT_SIZE)
        .line_height(LINE_TIGHT)
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
fn divider<'a>(padding: Padding) -> Element<'a, Msg> {
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
        .push(mono("·", theme::STATUS_SIZE, theme::SEPARATOR))
        .push(glyph(assets::PIN, PIN_SIZE.0, PIN_SIZE.1))
        .push(mono(d.location.clone(), theme::STATUS_SIZE, theme::TEXT_SECONDARY));

    let titles = Column::new()
        .spacing(theme::HEADER_COLUMN_GAP)
        .push(ui_medium("WireGuard", theme::TITLE_SIZE, theme::TEXT_PRIMARY))
        .push(status_row);

    let chip = widget::container(mono(d.iface.clone(), theme::CHIP_SIZE, theme::TEXT_FAINT))
        .padding(CHIP_PAD)
        .class(theme::chip_class());

    Row::new()
        .width(Length::Fill)
        .spacing(theme::HEADER_GAP)
        .align_y(Alignment::Center)
        .padding(HEADER_PAD)
        .push(
            widget::icon(widget::icon::from_svg_bytes(assets::Y_SYMBOLIC).symbolic(true))
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
        .spacing(COUNT_GAP)
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
        .padding(BAND_PAD)
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
        .spacing(TILE_LABEL_GAP)
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
    .padding(TILE_PAD)
    .width(Length::FillPortion(1))
    .class(theme::tile_class())
    .into()
}

fn tiles<'a>(d: &Display) -> Element<'a, Msg> {
    Row::new()
        .width(Length::Fill)
        .spacing(theme::TILE_GAP)
        .padding(TILES_PAD)
        .push(tile(
            "UPLOAD",
            assets::ARROW_UP,
            theme::ACCENT_UP,
            d.up_total.clone(),
            d.up_rate.clone(),
        ))
        .push(tile(
            "DOWNLOAD",
            assets::ARROW_DOWN,
            theme::ACCENT_DOWN,
            d.down_total.clone(),
            d.down_rate.clone(),
        ))
        .into()
}

/// The popup's contents. libcosmic's `popup_container` supplies the
/// surface, blur, radius and shadow around this.
pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let d = state.display();
    Column::new()
        .width(Length::Fill)
        .spacing(theme::POPUP_PADDING)
        .padding(theme::POPUP_PADDING)
        .push(header(&d))
        .push(divider(DIVIDER_ABOVE_BAND))
        .push(accounts_band(&d))
        .push(divider(DIVIDER_ABOVE_TILES))
        .push(tiles(&d))
        .into()
}
