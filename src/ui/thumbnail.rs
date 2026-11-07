//! The thumbnail widget: one per client surface, in both modes.

use cosmic::iced::{Border, Color, ContentFit, Length};
use cosmic::iced::platform_specific::shell::subsurface_widget::Subsurface;
use cosmic::widget;
use cosmic::{Element, theme};

use super::{Client, Msg, status};
use crate::backend::CaptureImage;
use crate::backend::gl::swaps_axes;
use crate::model::config::{Config, Mode, parse_color};

/// Layer-surface size (including border) for a thumbnail: the captured
/// window's aspect as displayed (a raw frame with a 90°/270° transform is
/// shown with its axes swapped).
pub fn size(config: &Config, image: Option<&CaptureImage>) -> (u32, u32) {
    size_at_width(config.thumb_width, config.border_width(), image)
}

/// `size` at an explicit content width: the hover zoom scales the width
/// alone, and this runs per frame and per dock relayout, so it takes the
/// two numbers rather than a `Config` (six `Vec<String>`s to clone).
fn size_at_width(thumb_width: u32, border_px: u32, image: Option<&CaptureImage>) -> (u32, u32) {
    match image {
        Some(img) if img.width > 0 && img.height > 0 => {
            let (w, h) = if swaps_axes(img.transform) { (img.height, img.width) } else { (img.width, img.height) };
            size_for_width(thumb_width, border_px, w, h)
        }
        _ => size_for_width(thumb_width, border_px, 16, 9),
    }
}

/// Layer-surface size when hovered (or not); content scales by
/// `zoom_factor`, the border stays fixed.
pub fn zoomed_size(config: &Config, image: Option<&CaptureImage>, zoomed: bool) -> (u32, u32) {
    if !zoomed {
        return size(config, image);
    }
    let width = (config.thumb_width as f32 * config.zoom_factor).round() as u32;
    size_at_width(width, config.border_width(), image)
}

/// Surface size for a `src_w`×`src_h` source shown `thumb_width` wide
/// inside a `border_px` border.
pub fn size_for_width(thumb_width: u32, border_px: u32, src_w: u32, src_h: u32) -> (u32, u32) {
    let inner_w = thumb_width.max(1);
    let inner_h = ((inner_w as u64 * src_h as u64) / src_w.max(1) as u64) as u32;
    (inner_w + 2 * border_px, inner_h.max(1) + 2 * border_px)
}

/// The whole thumbnail's opacity factor (opacity spec §1.3): the configured
/// percent, except that the thumbnail under the pointer is fully opaque so
/// it can be read, fading back when the pointer leaves.
pub fn opacity(config: &Config, hovered: bool) -> f32 {
    if hovered { 1.0 } else { f32::from(config.thumb_opacity) / 100.0 }
}

/// `color` with its alpha multiplied by `factor`; the channels are untouched.
pub fn with_opacity(color: Color, factor: f32) -> Color {
    Color { a: color.a * factor, ..color }
}

/// The alarm border colour, `#e11d48`.
pub const ALARM: [f32; 4] = [0.882, 0.114, 0.282, 1.0];
/// The alarm caption colour: the same red lifted to read on the dark pill (`#fb7185`).
const ALARM_INK: Color = Color { r: 0.984, g: 0.443, b: 0.522, a: 1.0 };
/// The caption pill: near-opaque so the text never fights the frame behind it.
const PILL: Color = Color { r: 0.02, g: 0.02, b: 0.03, a: 0.82 };

/// The border colour: alarm (shot at or tackled) over everything, else the
/// active override, else the theme accent, else the inactive colour.
pub fn border_rgba(alarm: bool, activated: bool, active_override: Option<[f32; 4]>, inactive: [f32; 4], accent: [f32; 4]) -> [f32; 4] {
    if alarm {
        return ALARM;
    }
    match (activated, active_override) {
        (true, Some(c)) => c,
        (true, None) => accent,
        (false, _) => inactive,
    }
}

pub fn view<'a>(client: &'a Client, config: &Config) -> Element<'a, Msg> {
    // Active border: the configured colour, else the theme accent (what the
    // compositor outlines the focused window with). Resolved inside the style
    // closure because that is where the theme is available.
    let active_override = client.info.activated.then(|| config.active_border.as_deref().and_then(parse_color)).flatten();
    let inactive = parse_color(&config.inactive_border).unwrap_or([0.25, 0.25, 0.25, 1.0]);
    let activated = client.info.activated;
    let border_px = config.border_width() as f32;
    // One factor for everything drawn here — the image (through the
    // compositor's alpha modifier) and every iced layer over it — so the
    // thumbnail fades as one object rather than leaving an opaque frame
    // around a ghost. Text over the dark label and placeholder fills is
    // white whatever the theme, so it takes the factor as an explicit colour.
    let alpha = opacity(config, client.hovered);
    let text_ink = theme::Text::Color(with_opacity(Color::WHITE, alpha));
    let now = std::time::Instant::now();
    let alarm = client.status.alarm(now);
    let status_line = config.show_status.then(|| status::caption(&client.status, now)).flatten();

    let image: Element<'a, Msg> = if client.unavailable {
        widget::container(widget::text("capture unavailable").size(12).class(text_ink))
            .center(Length::Fill)
            .class(theme::Container::custom(move |_| widget::container::Style {
                background: Some(cosmic::iced::Background::Color(with_opacity(Color { r: 0.25, g: 0.25, b: 0.25, a: 0.9 }, alpha))),
                ..Default::default()
            }))
            .into()
    } else {
        match &client.image {
            Some(img) => Subsurface::new(img.buffer.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                // Below the parent (z < 0) so the iced border, name label and
                // pin glyph paint over the image. The backend ships frames
                // already upright and corner-masked; when it can't (no GL)
                // the raw frame carries its own transform.
                .z(-1)
                .transform(img.transform)
                // libcosmic drives `wp_alpha_modifier_v1` from this: the
                // compositor multiplies the alpha at composition time, so a
                // change costs no re-render.
                .alpha(alpha)
                .into(),
            None => widget::container(widget::text("waiting for frame…").size(12).class(text_ink))
                .center(Length::Fill)
                .into(),
        }
    };

    let mut layers: Vec<Element<'a, Msg>> = vec![image];
    if config.show_names || status_line.is_some() {
        let mut lines: Vec<Element<'a, Msg>> = Vec::new();
        if config.show_names {
            lines.push(widget::text(client.info.login.label()).size(15).font(cosmic::font::bold()).class(text_ink).into());
        }
        if let Some(line) = status_line {
            // The alarm colour again in the caption, so the signal survives
            // a switched-off border.
            let ink = theme::Text::Color(with_opacity(if alarm { ALARM_INK } else { Color { a: 0.92, ..Color::WHITE } }, alpha));
            lines.push(widget::text(line).size(12).font(cosmic::font::semibold()).class(ink).into());
        }
        let lines = cosmic::iced::widget::column(lines).spacing(1);
        let label = widget::container(lines)
            .padding([4, 9])
            .class(theme::Container::custom(move |_| widget::container::Style {
                background: Some(cosmic::iced::Background::Color(with_opacity(PILL, alpha))),
                border: Border { radius: 6.0.into(), ..Default::default() },
                ..Default::default()
            }));
        layers.push(
            widget::container(label)
                .align_bottom(Length::Fill)
                .align_left(Length::Fill)
                .padding(4)
                .into(),
        );
    }
    // Pins only mean something in floating mode; dock positions come from
    // the layout, so a stale `pinned` (from a saved layout) shows nothing.
    if client.pinned && config.mode == Mode::Floating {
        layers.push(
            widget::container(widget::text("📌").size(14).class(text_ink))
                .align_top(Length::Fill)
                .align_right(Length::Fill)
                .padding(4)
                .into(),
        );
    }

    let stack = cosmic::iced::widget::stack(layers).width(Length::Fill).height(Length::Fill);

    let radius = config.radius() as f32;
    let framed = widget::container(stack)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(move |theme| {
            let accent = theme.cosmic().accent_color();
            let [r, g, b, a] = border_rgba(alarm, activated, active_override, inactive, [accent.red, accent.green, accent.blue, accent.alpha]);
            widget::container::Style {
                border: Border { color: with_opacity(Color { r, g, b, a }, alpha), width: border_px, radius: radius.into() },
                ..Default::default()
            }
        }));

    framed.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_uses_default_aspect_before_first_frame() {
        let config = Config { thumb_width: 320, border_px: 2, ..Config::default() };
        assert_eq!(size(&config, None), (324, 184));
    }

    #[test]
    fn size_follows_captured_aspect() {
        assert_eq!(size_for_width(320, 2, 2560, 1440), (324, 184));
        assert_eq!(size_for_width(320, 2, 1000, 1000), (324, 324));
    }

    /// Opacity spec §1.5: the configured percent as a factor, except under
    /// the pointer, where a thumbnail is always readable.
    #[test]
    fn a_hovered_thumbnail_is_opaque_and_the_rest_take_the_configured_percent() {
        let config = Config { thumb_opacity: 80, ..Config::default() };
        assert!((opacity(&config, false) - 0.8).abs() < 1e-6);
        assert_eq!(opacity(&config, true), 1.0);
        assert_eq!(opacity(&Config::default(), false), 1.0);
        let c = with_opacity(Color { r: 0.1, g: 0.2, b: 0.3, a: 0.5 }, 0.5);
        assert_eq!((c.r, c.g, c.b), (0.1, 0.2, 0.3));
        assert!((c.a - 0.25).abs() < 1e-6);
    }

    #[test]
    fn alarm_outranks_active_and_inactive() {
        let grey = [0.1, 0.1, 0.1, 1.0];
        let accent = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(border_rgba(true, true, None, grey, accent), ALARM);
        assert_eq!(border_rgba(true, false, None, grey, accent), ALARM);
        assert_eq!(border_rgba(false, false, None, grey, accent), grey);
        assert_eq!(border_rgba(false, true, None, grey, accent), accent);
        assert_eq!(border_rgba(false, true, Some([0.0, 1.0, 0.0, 1.0]), grey, accent), [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn a_border_switched_off_takes_no_room() {
        let config = Config { thumb_width: 320, border_px: 2, show_border: Some(false), ..Config::default() };
        assert_eq!(size(&config, None), (320, 180));
    }

    #[test]
    fn zoomed_size_scales_content_not_border() {
        let config = Config { thumb_width: 320, border_px: 2, zoom_factor: 1.5, ..Config::default() };
        assert_eq!(zoomed_size(&config, None, false), (324, 184));
        assert_eq!(zoomed_size(&config, None, true), (484, 274));
    }
}
