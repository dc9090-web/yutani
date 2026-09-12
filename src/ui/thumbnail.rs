//! The thumbnail widget: one per client surface, in both modes.

use cosmic::iced::{Border, ContentFit, Length};
use cosmic::iced::platform_specific::shell::subsurface_widget::Subsurface;
use cosmic::widget;
use cosmic::{Element, theme};

use super::{Client, Msg};
use crate::backend::CaptureImage;
use crate::backend::gl::swaps_axes;
use crate::model::config::{Config, Mode, parse_color};

/// Layer-surface size (including border) for a thumbnail: the captured
/// window's aspect as displayed (a raw frame with a 90°/270° transform is
/// shown with its axes swapped).
pub fn size(config: &Config, image: Option<&CaptureImage>) -> (u32, u32) {
    match image {
        Some(img) if img.width > 0 && img.height > 0 => {
            let (w, h) = if swaps_axes(img.transform) { (img.height, img.width) } else { (img.width, img.height) };
            size_for(config, w, h)
        }
        _ => size_for(config, 16, 9),
    }
}

/// Layer-surface size when hovered (or not); content scales by
/// `zoom_factor`, the border stays fixed.
pub fn zoomed_size(config: &Config, image: Option<&CaptureImage>, zoomed: bool) -> (u32, u32) {
    if !zoomed {
        return size(config, image);
    }
    let scaled = Config {
        thumb_width: (config.thumb_width as f32 * config.zoom_factor).round() as u32,
        ..config.clone()
    };
    size(&scaled, image)
}

pub fn size_for(config: &Config, src_w: u32, src_h: u32) -> (u32, u32) {
    let inner_w = config.thumb_width.max(1);
    let inner_h = ((inner_w as u64 * src_h as u64) / src_w.max(1) as u64) as u32;
    (inner_w + 2 * config.border_px, inner_h.max(1) + 2 * config.border_px)
}

pub fn view<'a>(client: &'a Client, config: &Config) -> Element<'a, Msg> {
    // Active border: the configured colour, else the theme accent (what the
    // compositor outlines the focused window with). Resolved inside the style
    // closure because that is where the theme is available.
    let active_override = client.info.activated.then(|| config.active_border.as_deref().and_then(parse_color)).flatten();
    let inactive = parse_color(&config.inactive_border).unwrap_or([0.25, 0.25, 0.25, 1.0]);
    let activated = client.info.activated;
    let border_px = config.border_px as f32;

    let image: Element<'a, Msg> = if client.unavailable {
        widget::container(widget::text("capture unavailable").size(12))
            .center(Length::Fill)
            .class(theme::Container::custom(|_| widget::container::Style {
                background: Some(cosmic::iced::Background::Color(cosmic::iced::Color { r: 0.25, g: 0.25, b: 0.25, a: 0.9 })),
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
                .into(),
            None => widget::container(widget::text("waiting for frame…").size(12))
                .center(Length::Fill)
                .into(),
        }
    };

    let mut layers: Vec<Element<'a, Msg>> = vec![image];
    if config.show_names {
        let label = widget::container(widget::text(client.info.login.label()).size(13))
            .padding([2, 6])
            .class(theme::Container::custom(|_| widget::container::Style {
                background: Some(cosmic::iced::Background::Color(cosmic::iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.6 })),
                border: Border { radius: 4.0.into(), ..Default::default() },
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
            widget::container(widget::text("📌").size(14))
                .align_top(Length::Fill)
                .align_right(Length::Fill)
                .padding(4)
                .into(),
        );
    }

    let stack = cosmic::iced::widget::stack(layers).width(Length::Fill).height(Length::Fill);

    let radius = config.corner_radius as f32;
    let framed = widget::container(stack)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(move |theme| {
            let [r, g, b, a] = match (activated, active_override) {
                (true, Some(c)) => c,
                (true, None) => {
                    let accent = theme.cosmic().accent_color();
                    [accent.red, accent.green, accent.blue, accent.alpha]
                }
                (false, _) => inactive,
            };
            widget::container::Style {
                border: Border { color: cosmic::iced::Color { r, g, b, a }, width: border_px, radius: radius.into() },
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
        let config = Config { thumb_width: 320, border_px: 2, ..Config::default() };
        assert_eq!(size_for(&config, 2560, 1440), (324, 184));
        assert_eq!(size_for(&config, 1000, 1000), (324, 324));
    }

    #[test]
    fn zoomed_size_scales_content_not_border() {
        let config = Config { thumb_width: 320, border_px: 2, zoom_factor: 1.5, ..Config::default() };
        assert_eq!(zoomed_size(&config, None, false), (324, 184));
        assert_eq!(zoomed_size(&config, None, true), (484, 274));
    }
}
