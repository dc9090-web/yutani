//! Pure drag/click state machine for thumbnails.
//!
//! While a drag is in progress the thumbnail's layer surface is enlarged to
//! cover the whole output and stays put; the thumbnail is drawn at an offset
//! inside it. Because the surface never moves, surface-local pointer
//! coordinates are absolute output coordinates, so a delta from the press
//! point is exact (no compounding error from a surface that moves later than
//! we asked). The canvas itself is only requested once local motion from the
//! press point exceeds `DRAG_THRESHOLD` (`Phase::Idle` → `Arming`, via
//! `on_move_local`), so a plain click never touches the surface at all.
//! Before the enlargement is confirmed (`Phase::Arming`) absolute motion is
//! ignored, since those coordinates are still relative to the old origin.

use cosmic::iced::Point;
use cosmic::iced::mouse::Button;
use cosmic::iced::window::Id as SurfaceId;

pub const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Pressed but motion has not yet crossed `DRAG_THRESHOLD`; surface is
    /// still thumbnail-sized and local coordinates are trustworthy.
    Idle,
    /// Enlarge requested; ignore absolute motion until `on_window_resize`
    /// confirms.
    Arming,
    /// Surface is full-output; cursor coords are absolute.
    Dragging,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DragState {
    pub surface: SurfaceId,
    pub button: Button,
    /// Surface-local position of the cursor at press time (valid while
    /// `Idle`, since the surface hasn't moved or resized yet).
    pub press_local: Point,
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
    /// Local motion crossed the threshold while `Idle`: enter the canvas.
    StartDrag,
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
        press_local: cursor_local,
        press_abs: (cursor_local.x + pos.0 as f32, cursor_local.y + pos.1 as f32),
        start_pos: pos,
        pinned,
        moved: false,
        phase: Phase::Idle,
        canvas: (0, 0),
    })
}

/// Surface-local motion while `Idle`. Once it crosses `DRAG_THRESHOLD`, moves
/// to `Arming` and reports `StartDrag` (once) so the caller can enlarge the
/// surface; a `pinned` thumbnail never leaves `Idle`.
pub fn on_move_local(drag: &mut DragState, cursor_local: Point) -> Outcome {
    if drag.pinned || drag.phase != Phase::Idle {
        return Outcome::None;
    }
    let dx = cursor_local.x - drag.press_local.x;
    let dy = cursor_local.y - drag.press_local.y;
    if (dx * dx + dy * dy).sqrt() < DRAG_THRESHOLD {
        return Outcome::None;
    }
    drag.moved = true;
    drag.phase = Phase::Arming;
    Outcome::StartDrag
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

    #[test]
    fn press_left_or_right_starts_idle_drag_middle_does_not() {
        assert!(matches!(
            on_press(sid(), Button::Left, Point::new(5.0, 5.0), (40, 40), false),
            Some(DragState { phase: Phase::Idle, .. })
        ));
        assert!(on_press(sid(), Button::Right, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Middle, Point::new(5.0, 5.0), (40, 40), false).is_none());
    }

    #[test]
    fn small_local_motion_stays_idle_and_release_is_a_click() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        assert_eq!(on_move_local(&mut d, Point::new(12.0, 11.0)), Outcome::None);
        assert_eq!(d.phase, Phase::Idle);
        assert_eq!(on_release(d), Outcome::Click(Button::Left));
    }

    #[test]
    fn local_motion_beyond_threshold_starts_the_drag_then_arms() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        assert_eq!(on_move_local(&mut d, Point::new(16.0, 10.0)), Outcome::StartDrag);
        assert_eq!(d.phase, Phase::Arming);
        assert!(d.moved);
        // absolute motion is ignored until armed
        assert_eq!(on_move(&mut d, Point::new(70.0, 55.0)), Outcome::None);
        on_armed(&mut d, (2560, 1440));
        // press_abs = (50,50); abs (70,55) → start + (20,5)
        assert_eq!(on_move(&mut d, Point::new(70.0, 55.0)), Outcome::Move((60, 45)));
        assert_eq!(on_release(d), Outcome::DragEnd);
    }

    #[test]
    fn pinned_never_starts_a_drag_but_clicks() {
        let mut d = on_press(sid(), Button::Right, Point::new(10.0, 10.0), (40, 40), true).unwrap();
        assert_eq!(on_move_local(&mut d, Point::new(90.0, 90.0)), Outcome::None);
        assert_eq!(on_release(d), Outcome::Click(Button::Right));
    }
}
