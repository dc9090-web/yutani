//! Dock mode: one strip per output along `dock_edge`, thumbnails in a row
//! (top/bottom) or column (left/right).
//!
//! Unlike floating mode, a dock strip is not a per-client surface: hover,
//! click and right-click are handled at the widget level (`mouse_area`
//! around each thumbnail) rather than through the surface-level pointer
//! path, so `App::client_for_surface` never resolves a dock surface id.

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::protocol::wl_output::WlOutput;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, set_anchor, set_size,
};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use cosmic::{Element, Task};

use super::{App, Client, Msg, Output, rules, thumbnail};
use crate::backend::{CaptureImage, Cmd, Handle};
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

/// Layer-shell anchor for a strip along `edge`: the edge itself plus both
/// its perpendicular sides (so the strip spans the output along that axis),
/// never the opposite edge.
pub fn anchor_for(edge: Edge) -> Anchor {
    match edge {
        Edge::Top => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        Edge::Bottom => Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        Edge::Left => Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
        Edge::Right => Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
    }
}

pub fn settings(id: SurfaceId, output: WlOutput, edge: Edge, thickness: u32) -> SctkLayerSurfaceSettings {
    let anchor = anchor_for(edge);
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

/// Dock mode lifecycle: strip creation/teardown/resize and the per-thumbnail
/// hover state that drives them. `Msg::Dock` dispatch itself stays in
/// `mod.rs::update`; this is what it calls into.
impl App {
    /// Dock mode: the clients shown on `output`, in strip order.
    pub(super) fn dock_order_for(&self, output: &WlOutput) -> Vec<Handle> {
        let shown: Vec<(&Handle, &Client)> = self
            .clients
            .iter()
            .filter(|(_, c)| self.should_show(c) && self.output_for(&c.info).as_ref() == Some(output))
            .collect();
        rules::dock_order(shown.iter().map(|(h, c)| (*h, c.info.login.label())))
    }

    pub(super) fn dock_thickness_for(&self, output: &WlOutput) -> u32 {
        let any_hovered = self.dock_order_for(output).iter().any(|h| self.clients[h].hovered);
        // `None` image: one 16:9-based thickness per strip, whatever the
        // individual clients' aspects (see `dock::thickness`).
        thickness(&self.config, any_hovered, None)
    }

    /// Dock mode: one strip per output that has a shown client; none for
    /// outputs that don't.
    pub(super) fn reconcile_docks(&mut self) -> Task<cosmic::Action<Msg>> {
        let mut tasks = Vec::new();
        let outputs: Vec<Output> = self.outputs.clone();
        for o in &outputs {
            let needed = !self.dock_order_for(&o.handle).is_empty();
            let has = self.docks.iter().any(|d| d.output == o.handle);
            if needed && !has {
                let id = SurfaceId::unique();
                let thickness = self.dock_thickness_for(&o.handle);
                let edge = self.config.dock_edge;
                tracing::info!(?id, output = %o.name, ?edge, thickness, "dock: create strip");
                self.docks.push(DockSurface { output: o.handle.clone(), id, last_thickness: thickness });
                tasks.push(get_layer_surface(settings(id, o.handle.clone(), edge, thickness)));
            } else if !needed && has {
                tasks.push(self.destroy_dock_on(&o.handle));
            }
        }
        // Strips on outputs that are gone.
        let stale: Vec<WlOutput> = self
            .docks
            .iter()
            .filter(|d| !outputs.iter().any(|o| o.handle == d.output))
            .map(|d| d.output.clone())
            .collect();
        for output in stale {
            tasks.push(self.destroy_dock_on(&output));
        }
        Task::batch(tasks)
    }

    pub(super) fn destroy_dock_on(&mut self, output: &WlOutput) -> Task<cosmic::Action<Msg>> {
        let Some(pos) = self.docks.iter().position(|d| d.output == *output) else { return Task::none() };
        let d = self.docks.remove(pos);
        let name = self.outputs.iter().find(|o| o.handle == d.output).map(|o| o.name.as_str()).unwrap_or("?");
        tracing::info!(id = ?d.id, output = %name, "dock: destroy strip");
        destroy_layer_surface(d.id)
    }

    pub(super) fn destroy_docks(&mut self) -> Task<cosmic::Action<Msg>> {
        let outputs: Vec<WlOutput> = self.docks.iter().map(|d| d.output.clone()).collect();
        Task::batch(outputs.iter().map(|o| self.destroy_dock_on(o)))
    }

    /// Resize the strip on `output` to its current thickness, if changed.
    pub(super) fn resize_dock_if_needed(&mut self, output: &WlOutput) -> Task<cosmic::Action<Msg>> {
        let thick = self.dock_thickness_for(output);
        let edge = self.config.dock_edge;
        let Some(d) = self.docks.iter_mut().find(|d| d.output == *output) else { return Task::none() };
        if d.last_thickness == thick {
            return Task::none();
        }
        d.last_thickness = thick;
        let (w, h) = size_for(edge, thick);
        set_size(d.id, w, h)
    }

    /// Pure `dock_edge` change with the mode unchanged: re-anchor every
    /// existing strip to `edge` in place instead of destroy+create, since
    /// the client set on each strip doesn't change. `set_size` is sent
    /// unconditionally — the axis flips (row↔column) even when the
    /// thickness itself is unchanged.
    pub(super) fn reanchor_docks(&mut self, edge: Edge) -> Task<cosmic::Action<Msg>> {
        let mut tasks = Vec::new();
        for d in &self.docks {
            let name = self.outputs.iter().find(|o| o.handle == d.output).map(|o| o.name.as_str()).unwrap_or("?");
            tracing::info!(id = ?d.id, output = %name, ?edge, "dock: re-anchor strip");
            tasks.push(set_anchor(d.id, anchor_for(edge)));
            let (w, h) = size_for(edge, d.last_thickness);
            tasks.push(set_size(d.id, w, h));
        }
        Task::batch(tasks)
    }

    pub(super) fn on_dock(&mut self, msg: DockMsg) -> Task<cosmic::Action<Msg>> {
        match msg {
            DockMsg::Press(h) => {
                self.send(Cmd::Activate(h));
                Task::none()
            }
            DockMsg::RightPress(h) => {
                self.send(Cmd::Minimize(h));
                Task::none()
            }
            DockMsg::Enter(h) => self.set_dock_hover(&h, true),
            DockMsg::Exit(h) => self.set_dock_hover(&h, false),
        }
    }

    /// Dock hover: the thumbnail grows in place on the next redraw; the
    /// strip is thickened to make room (and shrunk back on exit).
    fn set_dock_hover(&mut self, h: &Handle, hovered: bool) -> Task<cosmic::Action<Msg>> {
        let Some(c) = self.clients.get_mut(h) else { return Task::none() };
        c.hovered = hovered;
        match self.output_for(&self.clients[h].info) {
            Some(output) => self.resize_dock_if_needed(&output),
            None => Task::none(),
        }
    }
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

    #[test]
    fn anchor_for_has_edge_and_both_perpendiculars_not_opposite() {
        let cases = [
            (Edge::Top, Anchor::TOP, Anchor::BOTTOM, [Anchor::LEFT, Anchor::RIGHT]),
            (Edge::Bottom, Anchor::BOTTOM, Anchor::TOP, [Anchor::LEFT, Anchor::RIGHT]),
            (Edge::Left, Anchor::LEFT, Anchor::RIGHT, [Anchor::TOP, Anchor::BOTTOM]),
            (Edge::Right, Anchor::RIGHT, Anchor::LEFT, [Anchor::TOP, Anchor::BOTTOM]),
        ];
        for (edge, edge_bit, opposite, perpendiculars) in cases {
            let anchor = anchor_for(edge);
            assert!(anchor.contains(edge_bit), "{edge:?} missing its own edge bit");
            for p in perpendiculars {
                assert!(anchor.contains(p), "{edge:?} missing perpendicular {p:?}");
            }
            assert!(!anchor.contains(opposite), "{edge:?} should not contain opposite {opposite:?}");
        }
    }

    #[test]
    fn size_for_is_thickness_on_the_perpendicular_axis() {
        assert_eq!(size_for(Edge::Top, 40), (None, Some(40)));
        assert_eq!(size_for(Edge::Bottom, 40), (None, Some(40)));
        assert_eq!(size_for(Edge::Left, 40), (Some(40), None));
        assert_eq!(size_for(Edge::Right, 40), (Some(40), None));
    }
}
