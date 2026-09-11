//! Dock mode: one strip per output along `dock_edge`, thumbnails in a row
//! (top/bottom) or column (left/right).
//!
//! Unlike floating mode, a dock strip is not a per-client surface: hover,
//! click and right-click are handled at the widget level (`mouse_area`
//! around each thumbnail) rather than through the surface-level pointer
//! path, so `App::client_for_surface` never resolves a dock surface id.
//!
//! The strip spans its output along the edge but only the thumbnails accept
//! input: its wl_surface input region is set to exactly the thumbnail rects
//! (`item_rects`), so the empty band passes clicks through to whatever is
//! underneath (EVE's own HUD, typically).

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::protocol::wl_output::WlOutput;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, set_anchor, set_input_zone, set_size,
};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{Alignment, Length, Rectangle};
use cosmic::widget;
use cosmic::{Element, Task};

use super::{App, Client, Msg, Output, rules, thumbnail};
use crate::backend::{Cmd, Handle};
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
    /// Last input region sent via `set_input_zone` (or the creation zone),
    /// so a redundant region update can be skipped.
    pub last_zone: Vec<Rectangle>,
}

#[derive(Clone, Debug)]
pub enum DockMsg {
    Press(Handle),
    RightPress(Handle),
    Enter(Handle),
    Exit(Handle),
}

/// Strip thickness (the dimension perpendicular to the edge) for the
/// clients on it: the largest thumbnail extent along that axis as `view`
/// will lay it out — each client at its own aspect, zoomed only if *it* is
/// hovered — plus padding on both sides. For a top/bottom strip that is the
/// tallest thumbnail's height; for a left/right strip the widest one's
/// width. A client without a frame yet is laid out at the 16:9 default, as
/// is an empty strip.
pub fn thickness_for_strip<'a>(config: &Config, edge: Edge, clients: impl Iterator<Item = &'a Client>) -> u32 {
    let horizontal = is_horizontal(edge);
    let perp = |(w, h): (u32, u32)| if horizontal { h } else { w };
    let max = clients
        .map(|c| perp(thumbnail::zoomed_size(config, c.image.as_ref(), c.hovered)))
        .max()
        .unwrap_or_else(|| perp(thumbnail::zoomed_size(config, None, false)));
    max + 2 * DOCK_PADDING
}

/// Where each of `clients`' thumbnails sits in the strip, in strip-local
/// logical pixels, exactly as `view` lays them out: `DOCK_PADDING` from the
/// strip's start corner, `DOCK_PADDING` between items, each at its own
/// (zoomed if hovered) size. Along the perpendicular axis items sit against
/// the docked edge — top/left for Top/Left, far side for Bottom/Right — so
/// a hovered thumbnail grows away from the edge.
///
/// Independent of the output size: the strip content is laid out from its
/// start corner, not centred, so these rects can be sent as the surface's
/// input region before (and regardless of) any configure.
pub fn item_rects(config: &Config, edge: Edge, clients: &[&Client]) -> Vec<Rectangle> {
    let pad = DOCK_PADDING as f32;
    let horizontal = is_horizontal(edge);
    let sizes: Vec<(f32, f32)> = clients
        .iter()
        .map(|c| {
            let (w, h) = thumbnail::zoomed_size(config, c.image.as_ref(), c.hovered);
            (w as f32, h as f32)
        })
        .collect();
    // Content extent along the perpendicular axis: the largest item.
    let max_perp = sizes.iter().map(|&(w, h)| if horizontal { h } else { w }).fold(0.0_f32, f32::max);
    let mut along = pad;
    sizes
        .into_iter()
        .map(|(w, h)| {
            let rect = if horizontal {
                let y = if edge == Edge::Bottom { pad + max_perp - h } else { pad };
                let r = Rectangle { x: along, y, width: w, height: h };
                along += w + pad;
                r
            } else {
                let x = if edge == Edge::Right { pad + max_perp - w } else { pad };
                let r = Rectangle { x, y: along, width: w, height: h };
                along += h + pad;
                r
            };
            rect
        })
        .collect()
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

pub fn settings(
    id: SurfaceId,
    output: WlOutput,
    edge: Edge,
    thickness: u32,
    input_zone: Vec<Rectangle>,
) -> SctkLayerSurfaceSettings {
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
        // Only the thumbnails take input; the rest of the band is
        // click-through (see `item_rects`).
        input_zone: Some(input_zone),
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
    // grows away from it. Along the edge the strip starts at the output's
    // start corner (no centring): `item_rects` must be able to predict these
    // positions without knowing the output size.
    let edge = app.config.dock_edge;
    let strip: Element<'a, Msg> = match edge {
        Edge::Top => widget::row::with_children(items).spacing(DOCK_PADDING as f32).align_y(Alignment::Start).into(),
        Edge::Bottom => widget::row::with_children(items).spacing(DOCK_PADDING as f32).align_y(Alignment::End).into(),
        Edge::Left => widget::column::with_children(items).spacing(DOCK_PADDING as f32).align_x(Alignment::Start).into(),
        Edge::Right => widget::column::with_children(items).spacing(DOCK_PADDING as f32).align_x(Alignment::End).into(),
    };
    // Shrink-sized root: iced places it at the surface origin, so the
    // content is `DOCK_PADDING` from the top-left whatever the surface size.
    widget::container(strip).padding(DOCK_PADDING as f32).into()
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

    /// Dock mode: the clients shown on `output`, in strip order.
    fn dock_clients_for(&self, output: &WlOutput) -> Vec<&Client> {
        self.dock_order_for(output).iter().map(|h| &self.clients[h]).collect()
    }

    pub(super) fn dock_thickness_for(&self, output: &WlOutput) -> u32 {
        thickness_for_strip(&self.config, self.config.dock_edge, self.dock_clients_for(output).into_iter())
    }

    /// Input region for the strip on `output`: its thumbnails' rects.
    pub(super) fn dock_zone_for(&self, output: &WlOutput) -> Vec<Rectangle> {
        item_rects(&self.config, self.config.dock_edge, &self.dock_clients_for(output))
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
                let zone = self.dock_zone_for(&o.handle);
                let edge = self.config.dock_edge;
                tracing::info!(?id, output = %o.name, ?edge, thickness, input_rects = zone.len(), "dock: create strip");
                self.docks.push(DockSurface {
                    output: o.handle.clone(),
                    id,
                    last_thickness: thickness,
                    last_zone: zone.clone(),
                });
                tasks.push(get_layer_surface(settings(id, o.handle.clone(), edge, thickness, zone)));
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
        // Surviving strips: their client set, hover, or aspects may have
        // changed, which moves thickness and/or the input region. Both are
        // deduped against what was last sent.
        let live: Vec<WlOutput> = self.docks.iter().map(|d| d.output.clone()).collect();
        for output in live {
            tasks.push(self.refresh_dock(&output));
        }
        Task::batch(tasks)
    }

    /// Bring the strip on `output` (if any) up to date with its clients:
    /// thickness and input region, each only if it actually changed.
    pub(super) fn refresh_dock(&mut self, output: &WlOutput) -> Task<cosmic::Action<Msg>> {
        Task::batch([self.resize_dock_if_needed(output), self.set_dock_zone_if_needed(output)])
    }

    /// Send the strip's current thumbnail rects as its input region, if
    /// they differ from what was last sent.
    pub(super) fn set_dock_zone_if_needed(&mut self, output: &WlOutput) -> Task<cosmic::Action<Msg>> {
        let zone = self.dock_zone_for(output);
        let Some(d) = self.docks.iter_mut().find(|d| d.output == *output) else { return Task::none() };
        if d.last_zone == zone {
            return Task::none();
        }
        tracing::info!(id = ?d.id, input_rects = zone.len(), "dock: set input zone");
        d.last_zone = zone.clone();
        set_input_zone(d.id, Some(zone))
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
    /// strip is thickened to make room (and shrunk back on exit) and its
    /// input region follows the new rects.
    fn set_dock_hover(&mut self, h: &Handle, hovered: bool) -> Task<cosmic::Action<Msg>> {
        let Some(c) = self.clients.get_mut(h) else { return Task::none() };
        c.hovered = hovered;
        match self.output_for(&self.clients[h].info) {
            Some(output) => self.refresh_dock(&output),
            None => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ClientInfo;
    use crate::model::client::Login;
    use crate::model::config::Config;
    use cosmic::iced::Point;

    /// A client with no frame yet (16:9 default aspect), hovered or not.
    fn client(hovered: bool) -> Client {
        Client {
            info: ClientInfo { login: Login::LoggingIn, activated: false, minimized: false, outputs: Vec::new() },
            image: None,
            surface: None,
            position: (0, 0),
            pinned: false,
            last_cursor: Point::ORIGIN,
            hovered,
            unavailable: false,
            placed: false,
            last_size: None,
            docked: true,
            paused: false,
        }
    }

    fn config() -> Config {
        Config { thumb_width: 320, border_px: 2, zoom_factor: 1.5, ..Config::default() }
    }

    #[test]
    fn thickness_is_tallest_thumb_plus_padding_and_only_the_hovered_one_is_zoomed() {
        let config = config();
        // Empty strip: 16:9 default, unzoomed.
        assert_eq!(thickness_for_strip(&config, Edge::Bottom, std::iter::empty()), 184 + 16);
        let plain = [client(false), client(false)];
        assert_eq!(thickness_for_strip(&config, Edge::Bottom, plain.iter()), 184 + 16);
        // One hovered: its zoomed height wins.
        let mixed = [client(false), client(true)];
        assert_eq!(thickness_for_strip(&config, Edge::Top, mixed.iter()), 274 + 16);
        // Column strips: thickness is along x, i.e. the widest item.
        assert_eq!(thickness_for_strip(&config, Edge::Left, plain.iter()), 324 + 16);
        assert_eq!(thickness_for_strip(&config, Edge::Right, mixed.iter()), 484 + 16);
    }

    #[test]
    fn item_rects_run_from_the_start_corner_with_padding_and_spacing() {
        let config = config();
        let a = client(false);
        let b = client(false);
        let rects = item_rects(&config, Edge::Bottom, &[&a, &b]);
        let (w, h) = thumbnail::zoomed_size(&config, None, false);
        assert_eq!((w, h), (324, 184));
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rectangle { x: 8.0, y: 8.0, width: 324.0, height: 184.0 });
        assert_eq!(rects[1], Rectangle { x: 8.0 + 324.0 + 8.0, y: 8.0, width: 324.0, height: 184.0 });
    }

    #[test]
    fn item_rects_use_the_zoomed_size_for_the_hovered_item_and_hug_the_docked_edge() {
        let config = config();
        let a = client(false);
        let b = client(true);
        let (zw, zh) = thumbnail::zoomed_size(&config, None, true);
        assert_eq!((zw, zh), (484, 274));
        // Bottom: items sit against the bottom of the content, so the short
        // one is pushed down by the zoomed one's extra height.
        let rects = item_rects(&config, Edge::Bottom, &[&a, &b]);
        assert_eq!(rects[0], Rectangle { x: 8.0, y: 8.0 + (274.0 - 184.0), width: 324.0, height: 184.0 });
        assert_eq!(rects[1], Rectangle { x: 8.0 + 324.0 + 8.0, y: 8.0, width: 484.0, height: 274.0 });
        // Top: both at the top.
        let rects = item_rects(&config, Edge::Top, &[&a, &b]);
        assert_eq!(rects[0].y, 8.0);
        assert_eq!(rects[1], Rectangle { x: 8.0 + 324.0 + 8.0, y: 8.0, width: 484.0, height: 274.0 });
        // Right: a column, items against the right of the content.
        let rects = item_rects(&config, Edge::Right, &[&a, &b]);
        assert_eq!(rects[0], Rectangle { x: 8.0 + (484.0 - 324.0), y: 8.0, width: 324.0, height: 184.0 });
        assert_eq!(rects[1], Rectangle { x: 8.0, y: 8.0 + 184.0 + 8.0, width: 484.0, height: 274.0 });
        // Left: a column at x = padding.
        let rects = item_rects(&config, Edge::Left, &[&a, &b]);
        assert_eq!(rects[0], Rectangle { x: 8.0, y: 8.0, width: 324.0, height: 184.0 });
        assert_eq!(rects[1].x, 8.0);
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
