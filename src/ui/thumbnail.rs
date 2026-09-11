//! The thumbnail widget shared by floating (and, later, dock) modes.

use cosmic::iced::{Border, ContentFit, Length};
use cosmic::iced::platform_specific::shell::subsurface_widget::Subsurface;
use cosmic::widget;
use cosmic::{Element, theme};

use super::{Client, Msg};
use crate::backend::CaptureImage;
use crate::model::config::{Config, parse_color};

/// Layer-surface size (including border) for a thumbnail.
pub fn size(config: &Config, image: Option<&CaptureImage>) -> (u32, u32) {
    match image {
        Some(img) if img.width > 0 && img.height > 0 => size_for(config, img.width, img.height),
        _ => size_for(config, 16, 9),
    }
}

pub fn size_for(config: &Config, src_w: u32, src_h: u32) -> (u32, u32) {
    let inner_w = config.thumb_width.max(1);
    let inner_h = ((inner_w as u64 * src_h as u64) / src_w.max(1) as u64) as u32;
    (inner_w + 2 * config.border_px, inner_h.max(1) + 2 * config.border_px)
}

/// Smallest x along a top row (`origin`, step `pitch`) not already used.
pub fn next_free_x(taken: &[i32], origin: i32, pitch: i32) -> i32 {
    (0..).map(|slot| origin + slot * pitch).find(|x| !taken.contains(x)).unwrap()
}

pub fn view<'a>(client: &'a Client, config: &Config, on_press: Msg) -> Element<'a, Msg> {
    let border_hex = if client.info.activated { &config.active_border } else { &config.inactive_border };
    let [r, g, b, a] = parse_color(border_hex).unwrap_or([1.0, 0.5, 0.0, 1.0]);
    let border_color = cosmic::iced::Color { r, g, b, a };
    let border_px = config.border_px as f32;

    let image: Element<'a, Msg> = match &client.image {
        Some(img) => Subsurface::new(img.buffer.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(ContentFit::Contain)
            .alpha(config.opacity)
            .transform(img.transform)
            .into(),
        None => widget::container(widget::text("waiting for frame…").size(12))
            .center(Length::Fill)
            .into(),
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

    let stack = cosmic::iced::widget::stack(layers).width(Length::Fill).height(Length::Fill);

    let framed = widget::container(stack)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(move |_| widget::container::Style {
            border: Border { color: border_color, width: border_px, radius: 4.0.into() },
            ..Default::default()
        }));

    widget::mouse_area(framed).on_press(on_press).into()
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
    fn next_free_x_fills_the_first_gap() {
        assert_eq!(next_free_x(&[], 40, 336), 40);
        assert_eq!(next_free_x(&[40], 40, 336), 376);
        assert_eq!(next_free_x(&[40, 376], 40, 336), 712);
        assert_eq!(next_free_x(&[376], 40, 336), 40);
        assert_eq!(next_free_x(&[40, 712], 40, 336), 376);
    }
}
