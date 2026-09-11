//! Dock mode: one strip per output along `dock_edge`, thumbnails in a row
//! (top/bottom) or column (left/right).
//!
//! Unlike floating mode, a dock strip is not a per-client surface: hover,
//! click and right-click are handled at the widget level (`mouse_area`
//! around each thumbnail) rather than through the surface-level pointer
//! path, so `App::client_for_surface` never resolves a dock surface id.

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::protocol::wl_output::WlOutput;
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use cosmic::Element;

use super::{App, Msg, thumbnail};
use crate::backend::{CaptureImage, Handle};
use crate::model::config::{Config, Edge};

/// Gap between the strip edge and thumbnails, and between thumbnails.
pub const DOCK_PADDING: u32 = 8;

#[derive(Clone, Debug)]
pub struct DockSurface {
    pub output: WlOutput,
    pub id: SurfaceId,
    /// Last thickness sent via `set_size` (or the creation size), so a
    /// resize to the same thickness can be skipped.
    pub last_thickness: u32,
}

#[derive(Clone, Debug)]
pub enum DockMsg {
    Press(Handle),
    RightPress(Handle),
    Enter(Handle),
    Exit(Handle),
}

/// Strip thickness (the dimension perpendicular to the edge): the thumbnail
/// height (incl. border) for the given image aspect — zoomed if any
/// thumbnail on the strip is hovered — plus padding on both sides.
///
/// Callers pass `None` for the image so the strip uses the 16:9 default
/// aspect: one thickness per strip regardless of individual client aspects.
pub fn thickness(config: &Config, any_hovered: bool, image: Option<&CaptureImage>) -> u32 {
    let (_, h) = thumbnail::zoomed_size(config, image, any_hovered);
    h + 2 * DOCK_PADDING
}

pub fn is_horizontal(edge: Edge) -> bool {
    matches!(edge, Edge::Top | Edge::Bottom)
}

/// `set_size` arguments for a strip of `thickness` along `edge`: the axis
/// perpendicular to the edge is fixed, the other stretches to the output.
pub fn size_for(edge: Edge, thickness: u32) -> (Option<u32>, Option<u32>) {
    if is_horizontal(edge) { (None, Some(thickness)) } else { (Some(thickness), None) }
}

pub fn settings(id: SurfaceId, output: WlOutput, edge: Edge, thickness: u32) -> SctkLayerSurfaceSettings {
    let anchor = match edge {
        Edge::Top => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        Edge::Bottom => Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        Edge::Left => Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
        Edge::Right => Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
    };
    SctkLayerSurfaceSettings {
        id,
        layer: Layer::Overlay,
        keyboard_interactivity: KeyboardInteractivity::None,
        anchor,
        output: IcedOutput::Output(output),
        namespace: "yutani-dock".into(),
        size: Some(size_for(edge, thickness)),
        exclusive_zone: 0,
        ..Default::default()
    }
}

pub fn view<'a>(app: &'a App, output: &WlOutput) -> Element<'a, Msg> {
    let order = app.dock_order_for(output);
    let items: Vec<Element<'a, Msg>> = order
        .into_iter()
        .filter_map(|h| app.clients.get(&h).map(|c| (h, c)))
        .map(|(h, c)| {
            let (w, hgt) = thumbnail::zoomed_size(&app.config, c.image.as_ref(), c.hovered);
            let cb = thumbnail::Callbacks {
                press: Msg::Dock(DockMsg::Press(h.clone())),
                right_press: Msg::Dock(DockMsg::RightPress(h.clone())),
                enter: Msg::Dock(DockMsg::Enter(h.clone())),
                exit: Msg::Dock(DockMsg::Exit(h)),
            };
            widget::container(thumbnail::interactive(c, &app.config, cb))
                .width(Length::Fixed(w as f32))
                .height(Length::Fixed(hgt as f32))
                .into()
        })
        .collect();
    // Items sit against the docked edge, so a hovered (zoomed) thumbnail
    // grows away from it.
    let edge = app.config.dock_edge;
    let strip: Element<'a, Msg> = match edge {
        Edge::Top => widget::row::with_children(items).spacing(DOCK_PADDING as f32).align_y(Alignment::Start).into(),
        Edge::Bottom => widget::row::with_children(items).spacing(DOCK_PADDING as f32).align_y(Alignment::End).into(),
        Edge::Left => widget::column::with_children(items).spacing(DOCK_PADDING as f32).align_x(Alignment::Start).into(),
        Edge::Right => widget::column::with_children(items).spacing(DOCK_PADDING as f32).align_x(Alignment::End).into(),
    };
    widget::container(strip).padding(DOCK_PADDING as f32).center(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::Config;

    #[test]
    fn thickness_is_thumb_height_plus_padding_and_grows_when_hovered() {
        let config = Config { thumb_width: 320, border_px: 2, zoom_factor: 1.5, ..Config::default() };
        assert_eq!(thickness(&config, false, None), 184 + 16);
        assert_eq!(thickness(&config, true, None), 274 + 16);
    }
}
