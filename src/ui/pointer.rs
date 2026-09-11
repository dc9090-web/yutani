//! Pure drag/click state machine for thumbnails.
//!
//! While a drag is in progress the thumbnail's layer surface is enlarged to
//! cover the whole output and stays put; the thumbnail is drawn at an offset
//! inside it. Because the surface never moves, surface-local pointer
//! coordinates are absolute output coordinates, so a delta from the press
//! point is exact (no compounding error from a surface that moves later than
//! we asked). Before the enlargement is confirmed (`Phase::Arming`) motion is
//! ignored, since those coordinates are still relative to the old origin.

use cosmic::iced::Point;
use cosmic::iced::mouse::Button;
use cosmic::iced::window::Id as SurfaceId;

pub const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Enlarge requested; ignore motion until `on_window_resize` confirms.
    Arming,
    /// Surface is full-output; cursor coords are absolute.
    Dragging,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DragState {
    pub surface: SurfaceId,
    pub button: Button,
    /// Absolute (output) position of the cursor at press time.
    pub press_abs: (f32, f32),
    /// Thumbnail position at press time.
    pub start_pos: (i32, i32),
    pub pinned: bool,
    pub moved: bool,
    pub phase: Phase,
    /// Size of the enlarged surface once known (clamp bounds).
    pub canvas: (i32, i32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    /// Move the thumbnail to this top-left (before snapping/clamping).
    Move((i32, i32)),
    /// Button released without dragging.
    Click(Button),
    /// Button released after dragging.
    DragEnd,
}

/// `cursor_local` is surface-local at a time when the surface is still
/// thumbnail-sized at `pos`, so `local + pos` is the absolute press point.
pub fn on_press(
    surface: SurfaceId,
    button: Button,
    cursor_local: Point,
    pos: (i32, i32),
    pinned: bool,
) -> Option<DragState> {
    if !matches!(button, Button::Left | Button::Right) {
        return None;
    }
    Some(DragState {
        surface,
        button,
        press_abs: (cursor_local.x + pos.0 as f32, cursor_local.y + pos.1 as f32),
        start_pos: pos,
        pinned,
        moved: false,
        phase: Phase::Arming,
        canvas: (0, 0),
    })
}

/// The surface has been resized to the full-output canvas of this size.
pub fn on_armed(drag: &mut DragState, canvas: (i32, i32)) {
    drag.phase = Phase::Dragging;
    drag.canvas = canvas;
}

/// `cursor_abs` is surface-local, which equals absolute once `Dragging`.
pub fn on_move(drag: &mut DragState, cursor_abs: Point) -> Outcome {
    if drag.pinned || drag.phase != Phase::Dragging {
        return Outcome::None;
    }
    let dx = cursor_abs.x - drag.press_abs.0;
    let dy = cursor_abs.y - drag.press_abs.1;
    if !drag.moved && (dx * dx + dy * dy).sqrt() < DRAG_THRESHOLD {
        return Outcome::None;
    }
    drag.moved = true;
    Outcome::Move((drag.start_pos.0 + dx.round() as i32, drag.start_pos.1 + dy.round() as i32))
}

pub fn on_release(drag: DragState) -> Outcome {
    if drag.moved { Outcome::DragEnd } else { Outcome::Click(drag.button) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> SurfaceId {
        SurfaceId::unique()
    }

    const CANVAS: (i32, i32) = (2560, 1440);

    #[test]
    fn press_left_or_right_starts_drag_middle_does_not() {
        let d = on_press(sid(), Button::Left, Point::new(5.0, 5.0), (40, 40), false).unwrap();
        assert_eq!(d.phase, Phase::Arming);
        assert_eq!(d.press_abs, (45.0, 45.0));
        assert!(on_press(sid(), Button::Right, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Middle, Point::new(5.0, 5.0), (40, 40), false).is_none());
    }

    #[test]
    fn motion_while_arming_is_ignored() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        // Still surface-local to the old origin; must not be trusted.
        assert_eq!(on_move(&mut d, Point::new(300.0, 300.0)), Outcome::None);
        assert!(!d.moved);
        on_armed(&mut d, CANVAS);
        assert_eq!(d.phase, Phase::Dragging);
        assert_eq!(d.canvas, CANVAS);
        assert_eq!(on_move(&mut d, Point::new(70.0, 55.0)), Outcome::Move((60, 45)));
    }

    #[test]
    fn small_motion_is_not_a_drag_and_release_is_a_click() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        on_armed(&mut d, CANVAS);
        assert_eq!(on_move(&mut d, Point::new(52.0, 51.0)), Outcome::None);
        assert!(!d.moved);
        assert_eq!(on_release(d), Outcome::Click(Button::Left));
    }

    #[test]
    fn motion_beyond_threshold_moves_by_absolute_delta() {
        // press local (10,10) at pos (40,40) → press_abs (50,50)
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        on_armed(&mut d, CANVAS);
        assert_eq!(on_move(&mut d, Point::new(70.0, 55.0)), Outcome::Move((60, 45)));
        assert_eq!(on_move(&mut d, Point::new(73.0, 55.0)), Outcome::Move((63, 45)));
        assert!(d.moved);
        assert_eq!(on_release(d), Outcome::DragEnd);
    }

    #[test]
    fn pinned_surface_never_moves_but_still_clicks() {
        let mut d = on_press(sid(), Button::Right, Point::new(10.0, 10.0), (40, 40), true).unwrap();
        assert_eq!(on_move(&mut d, Point::new(90.0, 90.0)), Outcome::None);
        // Even if something armed it, pinned wins.
        on_armed(&mut d, CANVAS);
        assert_eq!(on_move(&mut d, Point::new(190.0, 190.0)), Outcome::None);
        assert_eq!(on_release(d), Outcome::Click(Button::Right));
    }
}
