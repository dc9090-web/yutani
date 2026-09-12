//! Dock mode: a layout policy for the ordinary per-client surfaces. Each
//! thumbnail keeps its own layer surface (so cosmic-comp can round its
//! corners and hover/click go through the usual pointer path); this module
//! only decides where those surfaces sit — in a row (Top/Bottom) or column
//! (Left/Right) centred along `dock_edge`, `DOCK_INSET` px from the edge,
//! `DOCK_GAP` px apart — and pushes a `set_margin` to any that moved.

use cosmic::Task;
use cosmic::cctk::wayland_client::protocol::wl_output::WlOutput;
use cosmic::iced::platform_specific::shell::commands::layer_surface::set_margin;

use super::{App, Msg, Output};
use crate::backend::Handle;
use crate::model::config::Edge;

/// Distance between neighbouring thumbnails.
pub const DOCK_GAP: i32 = 8;
/// Distance from the docked edge of the output to the thumbnails.
pub const DOCK_INSET: i32 = 8;

/// Top-left positions (logical px) of `sizes.len()` thumbnails laid along
/// `edge` of an `output` of the given logical size: centred along the edge
/// (integer division — an odd remainder goes to the far side), `inset` from
/// the edge, `gap` apart. Bottom/Right align each item's far side with the
/// edge, so a taller/wider (hovered) item grows away from it.
pub fn layout(edge: Edge, output: (i32, i32), sizes: &[(u32, u32)], gap: i32, inset: i32) -> Vec<(i32, i32)> {
    let (ow, oh) = output;
    let horizontal = matches!(edge, Edge::Top | Edge::Bottom);
    let along = |(w, h): (u32, u32)| if horizontal { w as i32 } else { h as i32 };
    let n = sizes.len() as i32;
    let total: i32 = sizes.iter().map(|&s| along(s)).sum::<i32>() + gap * (n - 1).max(0);
    let mut cursor = (if horizontal { ow } else { oh } - total) / 2;
    sizes
        .iter()
        .map(|&s| {
            let (w, h) = (s.0 as i32, s.1 as i32);
            let pos = match edge {
                Edge::Top => (cursor, inset),
                Edge::Bottom => (cursor, oh - inset - h),
                Edge::Left => (inset, cursor),
                Edge::Right => (ow - inset - w, cursor),
            };
            cursor += along(s) + gap;
            pos
        })
        .collect()
}

impl App {
    /// Dock mode: the clients on `output` that have a surface, in dock order.
    pub(super) fn dock_order_for(&self, output: &WlOutput) -> Vec<Handle> {
        self.ordered(super::Mode::Dock, |h, c| {
            c.surface.is_some() && self.output_for_thumb(h).as_ref() == Some(output)
        })
    }

    /// Where each of `output`'s docked surfaces goes, in dock order, at its
    /// current (zoomed if hovered) size.
    fn dock_positions_for(&self, output: &Output) -> Vec<(Handle, (i32, i32))> {
        let order = self.dock_order_for(&output.handle);
        let sizes: Vec<(u32, u32)> = order.iter().map(|h| self.surface_size(&self.clients[h])).collect();
        let positions = layout(self.config.dock_edge, output.logical_size, &sizes, DOCK_GAP, DOCK_INSET);
        order.into_iter().zip(positions).collect()
    }

    /// Dock mode: where `handle`'s surface belongs, given that
    /// `client.surface` is already set (so the layout counts it).
    pub(super) fn dock_position_of(&self, handle: &Handle) -> (i32, i32) {
        let output = self
            .output_for_thumb(handle)
            .and_then(|o| self.outputs.iter().find(|k| k.handle == o));
        output
            .and_then(|o| self.dock_positions_for(o).into_iter().find(|(h, _)| h == handle))
            .map(|(_, pos)| pos)
            .unwrap_or((DOCK_INSET, DOCK_INSET))
    }

    /// Dock mode: move every docked surface to where the layout puts it now
    /// (client set, hover sizes, aspects, edge or output size may have
    /// changed). A surface that didn't move gets nothing. A drag canvas keeps
    /// its margin; `leave_canvas` applies the updated position at drag end.
    /// Outside dock mode this is a no-op, so callers need not check.
    pub(super) fn relayout_dock(&mut self) -> Task<cosmic::Action<Msg>> {
        if self.config.mode != super::Mode::Dock {
            return Task::none();
        }
        let mut tasks = Vec::new();
        for output in self.outputs.clone() {
            // An output whose logical size is not known yet would centre
            // everything at negative coordinates; wait for its InfoUpdate.
            if output.logical_size.0 <= 0 || output.logical_size.1 <= 0 {
                continue;
            }
            for (h, pos) in self.dock_positions_for(&output) {
                let c = self.clients.get_mut(&h).unwrap();
                if c.position == pos {
                    continue;
                }
                c.position = pos;
                let id = c.surface.unwrap();
                if self.in_canvas(id) {
                    continue;
                }
                tracing::info!(?id, x = pos.0, y = pos.1, "dock: relayout");
                tasks.push(set_margin(id, pos.1, 0, 0, pos.0));
            }
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: (i32, i32) = (2560, 1440);
    const ONE: [(u32, u32); 1] = [(484, 274)];
    const THREE: [(u32, u32); 3] = [(324, 184), (484, 274), (324, 324)];

    #[test]
    fn empty_input_gives_empty_layout() {
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            assert!(layout(edge, OUT, &[], DOCK_GAP, DOCK_INSET).is_empty());
        }
    }

    #[test]
    fn one_item_is_centred_along_the_edge_and_inset_from_it() {
        assert_eq!(layout(Edge::Top, OUT, &ONE, 8, 8), vec![((2560 - 484) / 2, 8)]);
        assert_eq!(layout(Edge::Bottom, OUT, &ONE, 8, 8), vec![((2560 - 484) / 2, 1440 - 8 - 274)]);
        assert_eq!(layout(Edge::Left, OUT, &ONE, 8, 8), vec![(8, (1440 - 274) / 2)]);
        assert_eq!(layout(Edge::Right, OUT, &ONE, 8, 8), vec![(2560 - 8 - 484, (1440 - 274) / 2)]);
    }

    #[test]
    fn three_items_on_top_and_bottom_are_a_centred_row_with_gaps() {
        // total = 324 + 8 + 484 + 8 + 324 = 1148; x0 = (2560 - 1148) / 2 = 706
        let xs = [706, 706 + 324 + 8, 706 + 324 + 8 + 484 + 8];
        assert_eq!(layout(Edge::Top, OUT, &THREE, 8, 8), vec![(xs[0], 8), (xs[1], 8), (xs[2], 8)]);
        // Bottom: each item's own bottom sits `inset` above the edge.
        assert_eq!(
            layout(Edge::Bottom, OUT, &THREE, 8, 8),
            vec![(xs[0], 1440 - 8 - 184), (xs[1], 1440 - 8 - 274), (xs[2], 1440 - 8 - 324)]
        );
    }

    #[test]
    fn three_items_on_left_and_right_are_a_centred_column_with_gaps() {
        // total = 184 + 8 + 274 + 8 + 324 = 798; y0 = (1440 - 798) / 2 = 321
        let ys = [321, 321 + 184 + 8, 321 + 184 + 8 + 274 + 8];
        assert_eq!(layout(Edge::Left, OUT, &THREE, 8, 8), vec![(8, ys[0]), (8, ys[1]), (8, ys[2])]);
        // Right: each item's own right side sits `inset` from the edge.
        assert_eq!(
            layout(Edge::Right, OUT, &THREE, 8, 8),
            vec![(2560 - 8 - 324, ys[0]), (2560 - 8 - 484, ys[1]), (2560 - 8 - 324, ys[2])]
        );
    }

    #[test]
    fn odd_totals_centre_by_integer_division() {
        // 2560 - 483 = 2077; 2077 / 2 = 1038 (remainder goes to the right).
        assert_eq!(layout(Edge::Top, OUT, &[(483, 100)], 8, 8), vec![(1038, 8)]);
        // 1440 - 101 = 1339; 1339 / 2 = 669.
        assert_eq!(layout(Edge::Left, OUT, &[(483, 101)], 8, 8), vec![(8, 669)]);
    }

    #[test]
    fn gap_and_inset_are_honoured() {
        let two = [(100, 50), (100, 50)];
        // total = 100 + 20 + 100 = 220; x0 = (2560 - 220) / 2 = 1170
        assert_eq!(layout(Edge::Top, OUT, &two, 20, 30), vec![(1170, 30), (1290, 30)]);
        assert_eq!(layout(Edge::Bottom, OUT, &two, 20, 30), vec![(1170, 1440 - 30 - 50), (1290, 1440 - 30 - 50)]);
    }
}
