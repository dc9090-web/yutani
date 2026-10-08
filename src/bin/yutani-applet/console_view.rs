//! The Nostromo popover (handoff v6 "Screen"): renders `Console` top to
//! bottom. Every size and colour comes from `yutani::applet::skin`, every
//! string from `yutani::applet::console::Console`.

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, Column, Row};
use cosmic::Element;

use yutani::applet::Action;
use yutani::applet::console::{AccountRow, Accounts, Console, ControlRow, Gauge, LaunchButton, Level, LogLine, Network, Step};
use yutani::applet::fonts::{self, advance_em};
use yutani::applet::menu::MenuRow;
use yutani::applet::ping::{Quality, SPARK_H};
use yutani::applet::sand::Drive;
use yutani::applet::skin::{self, Ink, PrimaryLook, Type};

use crate::app::{Applet, Msg, Note};
use crate::widgets::{self, Direction};

// ---- text ----------------------------------------------------------------------

/// Tracked text: iced has no letter-spacing, so each glyph sits in a cell
/// of its own advance plus the tracking — exact for every face, since the
/// advances come from the bundled font files.
fn t<'a>(text: impl AsRef<str>, ty: Type, color: Color) -> Element<'a, Msg> {
    let font = fonts::font(ty.face);
    if ty.track == 0.0 {
        return widget::text(text.as_ref().to_string()).size(ty.size).font(font).class(cosmic::theme::Text::Color(color)).into();
    }
    let cells = text.as_ref().chars().map(|ch| {
        let w = (advance_em(ty.face, ch) + ty.track) * ty.size;
        widget::container(widget::text(ch.to_string()).size(ty.size).font(font).class(cosmic::theme::Text::Color(color)))
            .width(Length::Fixed(w))
            .into()
    });
    Row::with_children(cells.collect::<Vec<Element<'a, Msg>>>()).align_y(Alignment::Center).into()
}

/// An arrow and its word, tracked: B612 Mono has no ▲ or ◄, so every
/// arrow in the popover is drawn (a filled triangle, 0.6 × the text size
/// square) rather than set as text.
fn arrowed<'a>(dir: Direction, word: &'static str, ty: Type, color: Color) -> Element<'a, Msg> {
    let px = (ty.size * 0.6).ceil();
    let arrow = layered(widgets::Arrow { direction: dir, color }, Length::Fixed(px), Length::Fixed(px));
    let row = Row::new().spacing(4).align_y(Alignment::Center);
    if dir == Direction::Down { row.push(t(word, ty, color)).push(arrow) } else { row.push(arrow).push(t(word, ty, color)) }.into()
}

/// A canvas in a renderer layer of its own. On the panel's popup surface
/// only the last canvas drawn into a layer showed up (2026-10-07: the
/// rockers, dotted rules, uplink arrow and scope were missing while the
/// footer cursor — the last canvas — and the stacked brackets drew); the
/// same view in an ordinary window drew every one. A stack draws
/// each child after its first in a layer of its own, so each canvas gets a
/// stack whose first child is an empty space of the same size.
fn layered<'a, P>(program: P, w: Length, h: Length) -> Element<'a, Msg>
where
    P: cosmic::iced::widget::canvas::Program<Msg, cosmic::Theme, cosmic::Renderer> + 'a,
{
    cosmic::iced::widget::stack([widget::space().width(w).height(h).into(), widget::canvas(program).width(w).height(h).into()])
        .width(w)
        .height(h)
        .into()
}

fn ink(i: Ink) -> Color {
    i.color()
}

fn fill_x<'a>() -> Element<'a, Msg> {
    widget::space().width(Length::Fill).into()
}

fn sized(w: f32, h: f32) -> widget::Space {
    widget::space().width(Length::Fixed(w)).height(Length::Fixed(h))
}

fn hairline<'a>(inset: f32) -> Element<'a, Msg> {
    widget::container(widget::container(widget::space().width(Length::Fill).height(Length::Fixed(1.0))).class(skin::hairline_class(skin::LINE)))
        .width(Length::Fill)
        .padding([0.0, inset])
        .into()
}

fn led<'a>(color: Color, glow: bool, px: f32) -> Element<'a, Msg> {
    widget::container(sized(px, px)).class(skin::led_class(color, glow)).into()
}

fn centered<'a>(content: impl Into<Element<'a, Msg>>, x: Horizontal) -> Element<'a, Msg> {
    widget::container(content).width(Length::Fill).height(Length::Fill).align_x(x).align_y(Vertical::Center).into()
}

fn margin<'a>(content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    widget::container(content).width(Length::Fill).padding(skin::CARD_MARGIN).into()
}

/// A card; `brackets` adds the interactive cards' corner marks.
fn card<'a>(content: impl Into<Element<'a, Msg>>, brackets: bool) -> Element<'a, Msg> {
    let body: Element<'a, Msg> = widget::container(content).width(Length::Fill).class(skin::card_class()).into();
    let body = if brackets {
        cosmic::iced::widget::stack([body, widget::canvas(widgets::Brackets).width(Length::Fill).height(Length::Fill).into()]).into()
    } else {
        body
    };
    margin(body)
}

// ---- header ----------------------------------------------------------------------

fn header<'a>(c: &Console) -> Element<'a, Msg> {
    let mark = widget::icon(widget::icon::from_svg_bytes(yutani::assets::YUTANI_SYMBOLIC).symbolic(true))
        .class(cosmic::theme::Svg::custom(|_| cosmic::iced::widget::svg::Style { color: Some(skin::AMBER) }))
        .width(Length::Fixed(skin::PLATE_MARK_PX))
        .height(Length::Fixed(skin::PLATE_MARK_PX));
    let plate = widget::container(mark)
        .width(Length::Fixed(skin::PLATE_PX))
        .height(Length::Fixed(skin::PLATE_PX))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(skin::plate_class());
    let subline = Row::new()
        .align_y(Alignment::Center)
        .push(t("ユタニ重工", skin::SUBLINE_JP, skin::DIM))
        .push(t(" · MULTIBOX SYSTEMS", skin::SUBLINE, skin::DIM));
    let words = Column::new().spacing(2).push(t("YUTANI", skin::WORDMARK, skin::WHITE)).push(subline);
    let state = if c.running { skin::PHOSPHOR } else { skin::DIMMER };
    Row::new()
        .width(Length::Fill)
        .padding(skin::HEADER_PAD)
        .spacing(skin::HEADER_GAP)
        .align_y(Alignment::Center)
        .push(plate)
        .push(words)
        .push(fill_x())
        .push(led(state, c.running, skin::HEADER_LED_PX))
        .push(t(c.state_word, skin::STATE_WORD, state))
        .into()
}

// ---- section header ---------------------------------------------------------------

fn section<'a>(index: &'static str, label: &str, meta: Option<Element<'a, Msg>>) -> Element<'a, Msg> {
    let chip = widget::container(t(index, skin::SECTION_INDEX, skin::BG)).padding(skin::SECTION_CHIP_PAD).class(skin::chip_class(Some(skin::PHOSPHOR), None));
    let mut row = Row::new()
        .width(Length::Fill)
        .padding(skin::SECTION_HEAD_PAD)
        .spacing(skin::SECTION_HEAD_GAP)
        .align_y(Alignment::Center)
        .push(chip)
        .push(t(label, skin::SECTION_LABEL, skin::PHOSPHOR))
        .push(layered(widgets::DottedRule, Length::Fill, Length::Fixed(3.0)));
    if let Some(meta) = meta {
        row = row.push(meta);
    }
    row.into()
}

// ---- 01 control ---------------------------------------------------------------------

fn control_row<'a>(r: &ControlRow) -> Element<'a, Msg> {
    let text = Column::new().spacing(3).push(t(r.title, skin::ROW_TITLE, skin::PHOSPHOR)).push(t(r.sub.clone(), skin::ROW_SUB, ink(r.sub_ink)));
    let rocker = widget::button::custom(layered(widgets::Rocker { state: r.rocker }, Length::Fixed(skin::TOGGLE_W), Length::Fixed(skin::TOGGLE_H)))
        .padding(0)
        .class(skin::bare_class())
        .on_press_maybe(r.press.map(Msg::Press));
    Row::new()
        .width(Length::Fill)
        .padding(skin::ROW_PAD)
        .spacing(skin::ROW_GAP)
        .align_y(Alignment::Center)
        .push(led(ink(r.led), r.glow, skin::LED_PX))
        .push(widget::container(text).width(Length::Fill).clip(true))
        .push(rocker)
        .into()
}

fn control<'a>(c: &Console) -> Element<'a, Msg> {
    card(Column::new().width(Length::Fill).push(control_row(&c.control[0])).push(hairline(skin::HAIRLINE_INSET)).push(control_row(&c.control[1])), true)
}

// ---- 02 accounts ----------------------------------------------------------------------

fn account_row<'a>(r: &AccountRow) -> Element<'a, Msg> {
    let (chip_fill, chip_edge, chip_ink) = if r.focused { (Some(skin::AMBER), None, skin::BG) } else { (None, Some(skin::LINE_2), skin::DIM) };
    let chip = widget::container(t(r.index.to_string(), skin::INDEX_CHIP, chip_ink))
        .width(Length::Fixed(skin::INDEX_CHIP_PX))
        .height(Length::Fixed(skin::INDEX_CHIP_PX))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(skin::chip_class(chip_fill, chip_edge));
    let name_ink = if r.focused { skin::WHITE } else { skin::PHOSPHOR };
    let mut row = Row::new()
        .width(Length::Fill)
        .spacing(skin::ROW_GAP)
        .align_y(Alignment::Center)
        .push(chip)
        .push(widget::container(t(r.name.clone(), skin::ACCOUNT_NAME, name_ink)).width(Length::Fill).clip(true));
    if r.focused {
        row = row.push(arrowed(Direction::Left, "FOCUSED", skin::FOCUSED_TAG, skin::AMBER));
    }
    widget::button::custom(centered(widget::container(row).padding(skin::ACCOUNT_ROW_PAD), Horizontal::Left))
        .width(Length::Fill)
        .height(Length::Fixed(skin::ACCOUNT_ROW_HEIGHT))
        .padding(0)
        .class(skin::account_row_class(r.focused))
        .on_press(Msg::Press(Action::Focus(r.index)))
        .into()
}

fn accounts<'a>(c: &Console) -> Element<'a, Msg> {
    let empty = |title: &'static str, title_ink: Color, body: &'static str, body_ink: Color| -> Element<'a, Msg> {
        Column::new()
            .width(Length::Fill)
            .spacing(3)
            .padding(skin::EMPTY_PAD)
            .push(t(title, skin::EMPTY_TITLE, title_ink))
            .push(widget::text(body).size(skin::EMPTY_BODY.size).font(fonts::font(skin::EMPTY_BODY.face)).class(cosmic::theme::Text::Color(body_ink)))
            .into()
    };
    let body: Element<'a, Msg> = match &c.accounts {
        Accounts::Stopped => empty("SERVICE STOPPED", skin::DIM, "THUMBNAILS, HOTKEYS AND TUNNEL ROUTING ARE INACTIVE.", skin::DIMMER),
        Accounts::Empty => empty("NO EVE CLIENTS RUNNING", skin::PHOSPHOR, "LAUNCH EVE BELOW — IT APPEARS HERE WITH THE NEXT FREE HOTKEY.", skin::DIM),
        Accounts::Rows(rows) => Column::with_children(rows.iter().map(account_row).collect::<Vec<_>>()).width(Length::Fill).spacing(1).padding(skin::ACCOUNT_LIST_PAD).into(),
    };
    card(body, true)
}

// ---- 03 network -------------------------------------------------------------------------

fn readout<'a>(dir: Direction, word: &'static str, value: String, label_ink: Color, label_first: bool) -> Element<'a, Msg> {
    let label = arrowed(dir, word, skin::READOUT_LABEL, label_ink);
    let value = t(value, skin::READOUT, skin::WHITE);
    let unit = t("KB/S", skin::UNIT, skin::DIM);
    let row = Row::new().spacing(6).align_y(Alignment::Center);
    if label_first { row.push(label).push(value).push(unit) } else { row.push(value).push(unit).push(label) }.into()
}

fn scope<'a>(n: &Network) -> Element<'a, Msg> {
    let drive = Drive { live: n.live, up_kbps: n.up_kbps, down_kbps: n.down_kbps };
    let top = Row::new()
        .width(Length::Fill)
        .push(readout(Direction::Up, "UPLINK", n.up.clone(), skin::AMBER, true))
        .push(fill_x())
        .push(readout(Direction::Down, "DOWNLINK", n.down.clone(), skin::PHOSPHOR, false));
    let bottom = Row::new()
        .width(Length::Fill)
        .push(t(n.tx_total.clone(), skin::SCOPE_FOOT, skin::DIM))
        .push(fill_x())
        .push(t("SESSION", skin::SCOPE_FOOT, skin::DIM))
        .push(fill_x())
        .push(t(n.rx_total.clone(), skin::SCOPE_FOOT, skin::DIM));
    let overlay = Column::new().width(Length::Fill).height(Length::Fill).padding([10.0, skin::SCOPE_INSET]).push(top).push(widget::space().height(Length::Fill)).push(bottom);
    cosmic::iced::widget::stack([
        layered(widgets::Scope { drive }, Length::Fill, Length::Fixed(skin::SCOPE_HEIGHT)),
        overlay.into(),
    ])
    .width(Length::Fill)
    .height(Length::Fixed(skin::SCOPE_HEIGHT))
    .into()
}

fn quality_ink(q: Quality) -> Color {
    match q {
        Quality::Nominal => skin::PHOSPHOR,
        Quality::Degraded => skin::AMBER,
        Quality::Poor => skin::RED,
        Quality::Idle => skin::DIMMER,
    }
}

fn ping_row<'a>(n: &Network) -> Element<'a, Msg> {
    let p = &n.ping;
    let q = quality_ink(p.quality);
    let left = Column::new()
        .width(Length::Fixed(skin::PING_LEFT_W))
        .spacing(2)
        .push(t("PING · TQ", skin::READOUT_LABEL, skin::PHOSPHOR))
        .push(Row::new().spacing(4).align_y(Alignment::End).push(t(p.value.clone(), skin::PING_VALUE, skin::WHITE)).push(t("MS", skin::UNIT, skin::DIM)))
        .push(t(n.ping_note.clone(), skin::PING_STATS, skin::DIM));
    let spark = widgets::Sparkline { dots: p.dots.clone(), avg_y: p.avg_y, color: q, live: n.live };
    let middle = Column::new()
        .width(Length::Fill)
        .spacing(4)
        .push(layered(spark, Length::Fill, Length::Fixed(SPARK_H)))
        .push(t(p.stats.clone(), skin::PING_STATS, skin::DIM))
        .push(t(p.jitter_loss.clone(), skin::PING_STATS, skin::DIM));
    // The quality word alone on the right; JIT · LOSS sits under the
    // MIN/AVG/MAX line instead. The handoff's three columns (64 · 240 · 74)
    // do not fit a 360 px popover with three-digit pings: the sparkline was
    // clipped and both stat lines wrapped. A fixed width, right-aligned,
    // so NOMINAL → DEGRADED → IDLE does not reflow the sparkline.
    let word = Row::new().spacing(6).align_y(Alignment::Center).push(t(p.quality.word(), skin::QUALITY_WORD, q)).push(led(q, false, 6.0));
    let right = widget::container(word).width(Length::Fixed(skin::QUALITY_W)).align_x(Horizontal::Right);
    Row::new().width(Length::Fill).padding(skin::ROW_PAD).spacing(skin::ROW_GAP).align_y(Alignment::Center).push(left).push(middle).push(right).into()
}

fn facts<'a>(n: &Network) -> Element<'a, Msg> {
    Row::new()
        .width(Length::Fill)
        .padding(skin::FACTS_PAD)
        .spacing(6)
        .align_y(Alignment::Center)
        .push(t("ENDPOINT", skin::FACTS_KEY, skin::PHOSPHOR))
        .push(t(n.endpoint.clone(), skin::FACTS, skin::WHITE))
        .push(t("·", skin::FACTS, skin::LINE_2))
        .push(t("PEER", skin::FACTS_KEY, skin::PHOSPHOR))
        .push(t(n.peer.clone(), skin::FACTS, skin::WHITE))
        .push(fill_x())
        .push(t(n.uptime.clone(), skin::FACTS, ink(n.uptime_ink)))
        .into()
}

fn network<'a>(n: &Network) -> Element<'a, Msg> {
    let mut col = Column::new().width(Length::Fill);
    if n.scope {
        col = col.push(scope(n)).push(hairline(0.0));
    }
    card(col.push(ping_row(n)).push(hairline(skin::HAIRLINE_INSET)).push(facts(n)), false)
}

// ---- 04 host --------------------------------------------------------------------------------

fn gauge_row<'a>(g: &Gauge) -> Element<'a, Msg> {
    let lit_color = match g.level {
        Level::Normal => skin::PHOSPHOR,
        Level::Warn => skin::AMBER,
        Level::Crit => skin::RED,
    };
    let cells = (0..skin::GAUGE_CELLS).map(|i| {
        let color = if i >= g.lit {
            skin::LINE
        } else if g.level == Level::Normal && i + 1 == g.lit {
            skin::WHITE
        } else {
            lit_color
        };
        widget::container(widget::space().width(Length::Fill).height(Length::Fixed(skin::GAUGE_CELL_H))).width(Length::Fill).class(skin::cell_class(color)).into()
    });
    let value_ink = if g.level == Level::Normal { skin::WHITE } else { lit_color };
    Row::new()
        .width(Length::Fill)
        .spacing(8)
        .align_y(Alignment::Center)
        .push(widget::container(t(g.label, skin::GAUGE_LABEL, skin::PHOSPHOR)).width(Length::Fixed(skin::GAUGE_LABEL_W)))
        .push(Row::with_children(cells.collect::<Vec<Element<'a, Msg>>>()).width(Length::Fill).spacing(skin::GAUGE_GAP))
        .push(widget::container(t(g.value.clone(), skin::GAUGE_VALUE, value_ink)).width(Length::Fixed(skin::GAUGE_VALUE_W)).align_x(Horizontal::Right))
        .push(widget::container(t(g.detail.clone(), skin::GAUGE_DETAIL, skin::DIM)).width(Length::Fixed(skin::GAUGE_DETAIL_W)).align_x(Horizontal::Right))
        .into()
}

fn host<'a>(c: &Console) -> Element<'a, Msg> {
    card(Column::with_children(c.host.iter().map(gauge_row).collect::<Vec<_>>()).width(Length::Fill).spacing(skin::HOST_ROW_GAP).padding(skin::HOST_PAD), false)
}

// ---- notice, action row, menu, footer ---------------------------------------------------------

fn notice<'a>(text: &str) -> Element<'a, Msg> {
    let stripe = layered(widgets::Hazard, Length::Fill, Length::Fixed(skin::NOTICE_STRIPE_H));
    let body = widget::container(widget::text(text.to_string()).size(skin::NOTICE.size).font(fonts::font(skin::NOTICE.face)).class(cosmic::theme::Text::Color(skin::AMBER)))
        .width(Length::Fill)
        .padding(skin::NOTICE_PAD);
    margin(widget::container(Column::new().width(Length::Fill).push(stripe).push(body)).width(Length::Fill).class(skin::notice_class()))
}

fn action_row<'a>(c: &Console, menu_open: bool) -> Element<'a, Msg> {
    let overflow = widget::button::custom(centered(layered(widgets::Ellipsis, Length::Fixed(12.0), Length::Fixed(2.0)), Horizontal::Center))
        .width(Length::Fixed(skin::OVERFLOW_PX))
        .height(Length::Fixed(skin::OVERFLOW_PX))
        .padding(0)
        .class(skin::overflow_class(menu_open))
        .on_press(Msg::ToggleMenu);
    let mut row = Row::new().width(Length::Fill).spacing(skin::ACTION_GAP);
    row = row.push(primary(c.launch));
    margin(row.push(overflow))
}

/// The primary button: ready, launching (step n / 4) or inert. An
/// engraved rule (`widgets::LaunchPlate`) round a drawn ▶ — B612 Mono's own sits
/// small and low — the label, and the four step cells on the right.
fn primary<'a>(l: LaunchButton) -> Element<'a, Msg> {
    let (label, sub, look, done) = match l {
        LaunchButton::Ready => ("LAUNCH EVE".to_string(), None, PrimaryLook::Ready, 0),
        LaunchButton::Launching { step } => ("LAUNCHING".to_string(), Some(format!("STEP {step} / 4")), PrimaryLook::Busy, step.saturating_sub(1)),
        LaunchButton::Inert(why) => (why.to_string(), None, PrimaryLook::Inert, 0),
    };
    let (ink, ground, strength) = match look {
        PrimaryLook::Ready => (skin::BG, skin::PHOSPHOR, 0.45),
        PrimaryLook::Busy => (skin::PHOSPHOR, skin::BG, 0.35),
        PrimaryLook::Inert => (skin::DIMMER, skin::BG, 0.25),
    };
    let mut content = Row::new().width(Length::Fill).spacing(8).align_y(Alignment::Center);
    if look != PrimaryLook::Inert {
        let px = (skin::PRIMARY.size * 0.75).round();
        content = content.push(layered(widgets::Arrow { direction: Direction::Right, color: ink }, Length::Fixed(px * 0.8), Length::Fixed(px)));
    }
    content = content.push(t(label, skin::PRIMARY, ink)).push(fill_x());
    if let Some(sub) = sub {
        content = content.push(t(sub, skin::PRIMARY_SUB, ink));
    }
    if look != PrimaryLook::Inert {
        content = content.push(layered(widgets::StepCells { done, ink }, Length::Fixed(widgets::StepCells::WIDTH), Length::Fixed(widgets::STEP_CELL.1)));
    }
    let face = cosmic::iced::widget::stack([
        layered(widgets::LaunchPlate { ink, ground, strength }, Length::Fill, Length::Fixed(skin::PRIMARY_HEIGHT)),
        centered(widget::container(content).padding([0.0, skin::PRIMARY_PAD_X]), Horizontal::Left),
    ])
    .width(Length::Fill)
    .height(Length::Fixed(skin::PRIMARY_HEIGHT));
    let button = widget::button::custom(face)
        .width(Length::Fill)
        .height(Length::Fixed(skin::PRIMARY_HEIGHT))
        .padding(0)
        .class(skin::primary_class(look))
        .on_press_maybe((look == PrimaryLook::Ready).then_some(Msg::Launch));
    if look == PrimaryLook::Ready {
        widget::container(button).width(Length::Fill).class(skin::launch_glow_class()).into()
    } else {
        button.into()
    }
}

/// The launch log: one line per step, then the failure reason if any.
fn launch_card<'a>(lines: &[LogLine], failed: Option<&str>) -> Element<'a, Msg> {
    let mut col = Column::new().width(Length::Fill).spacing(3);
    for l in lines {
        let (ink, word) = match l.status {
            Step::Done => (skin::PHOSPHOR, "OK"),
            Step::Running => (skin::AMBER, "…"),
            Step::Pending => (skin::DIMMER, ""),
            Step::Failed => (skin::RED, "FAIL"),
        };
        col = col.push(
            Row::new()
                .width(Length::Fill)
                .push(t(format!("▸ {}", l.text), skin::LOG, ink))
                .push(fill_x())
                .push(t(word, skin::LOG, ink)),
        );
    }
    if let Some(why) = failed {
        col = col.push(t(why, skin::LOG, skin::RED));
    }
    margin(widget::container(col).width(Length::Fill).padding(skin::pad(7.0, 12.0, 8.0, 12.0)).class(skin::log_class()))
}

fn menu_row<'a>(r: &MenuRow) -> Element<'a, Msg> {
    let color = if r.action.is_none() { skin::DIMMER } else if r.danger { skin::RED } else { skin::PHOSPHOR };
    widget::button::custom(centered(widget::container(t(r.label.to_uppercase(), skin::MENU, color)).padding(skin::MENU_ROW_PAD), Horizontal::Left))
        .width(Length::Fill)
        .height(Length::Fixed(skin::MENU_ROW_HEIGHT))
        .padding(0)
        .class(skin::menu_row_class(r.danger))
        .on_press_maybe(r.action.map(Msg::Press))
        .into()
}

fn menu<'a>(running: bool) -> Element<'a, Msg> {
    let rows = yutani::applet::menu::rows(running);
    Column::new()
        .width(Length::Fill)
        .push(hairline(0.0))
        .push(Column::with_children(rows.iter().map(menu_row).collect::<Vec<_>>()).width(Length::Fill).spacing(1).padding(skin::MENU_PAD))
        .into()
}

fn note_line<'a>(note: &Note) -> Element<'a, Msg> {
    let color = if note.progress { skin::DIM } else { skin::RED };
    widget::container(widget::text(note.text.to_uppercase()).size(skin::NOTICE.size).font(fonts::font(skin::NOTICE.face)).class(cosmic::theme::Text::Color(color)))
        .width(Length::Fill)
        .padding(skin::pad(0.0, 14.0, 10.0, 14.0))
        .into()
}

fn footer<'a>(c: &Console) -> Element<'a, Msg> {
    let ready = Row::new()
        .spacing(4)
        .align_y(Alignment::Center)
        .push(t("READY FOR INQUIRY", skin::FOOTER, skin::DIM))
        .push(layered(widgets::Cursor, Length::Fixed(skin::CURSOR_W), Length::Fixed(skin::CURSOR_H)));
    Column::new()
        .width(Length::Fill)
        .push(hairline(0.0))
        .push(Row::new().width(Length::Fill).padding(skin::FOOTER_PAD).align_y(Alignment::Center).push(t(c.footer.clone(), skin::FOOTER, skin::DIM)).push(fill_x()).push(ready))
        .into()
}

// ---- the popover ----------------------------------------------------------------------------------

pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let c = state.console();
    let count = c.count.clone().map(|n| t(n, skin::COUNT, skin::WHITE));
    let mut col = Column::new()
        .width(Length::Fill)
        .push(header(&c))
        .push(section("01", "CONTROL", None))
        .push(control(&c))
        .push(section("02", c.accounts_label, count))
        .push(accounts(&c))
        .push(section("03", "NETWORK", Some(t("EVE TRAFFIC ONLY", skin::SECTION_META, skin::DIM))))
        .push(network(&c.network))
        .push(section("04", "HOST", Some(t(c.host_meta.clone(), skin::SECTION_META, skin::DIM))))
        .push(host(&c));
    if let Some(n) = &c.notice {
        col = col.push(notice(n));
    }
    col = col.push(action_row(&c, state.menu_open));
    if let Some(log) = &c.launch_log {
        col = col.push(launch_card(log, c.launch_failed.as_deref()));
    }
    if let Some(note) = state.visible_note() {
        col = col.push(note_line(note));
    }
    if state.menu_open {
        col = col.push(menu(c.running));
    }
    col = col.push(footer(&c));
    // The handoff's 70 % phosphor line along the top edge. Its scanlines
    // are left out: on the panel's popup surface they garbled the 8–9 px
    // type at 1× (2026-10-07), and the handoff makes them optional.
    let top_line = widget::container(widget::space().width(Length::Fill).height(Length::Fixed(1.0))).class(skin::hairline_class(skin::TOP_LINE));
    widget::container(Column::new().width(Length::Fill).push(top_line).push(col))
        .width(Length::Fill)
        .class(skin::popover_class())
        .into()
}
