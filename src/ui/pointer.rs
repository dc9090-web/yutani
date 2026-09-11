//! Pure drag/click state machine for thumbnails. Cursor positions are
//! surface-local (iced `Point`); we convert to absolute output coordinates by
//! adding the surface's current position so a moving surface does not
//! confuse the delta.

use cosmic::iced::Point;
use cosmic::iced::mouse::Button;
use cosmic::iced::window::Id as SurfaceId;

pub const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct DragState {
    pub surface: SurfaceId,
    pub button: Button,
    pub press_abs: (f32, f32),
    pub start_pos: (i32, i32),
    pub pinned: bool,
    pub moved: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    /// Move the surface to this top-left (before snapping).
    Move((i32, i32)),
    /// Button released without dragging.
    Click(Button),
    /// Button released after dragging.
    DragEnd,
}

pub fn on_press(surface: SurfaceId, button: Button, cursor: Point, pos: (i32, i32), pinned: bool) -> Option<DragState> {
    if !matches!(button, Button::Left | Button::Right) {
        return None;
    }
    Some(DragState {
        surface,
        button,
        press_abs: (cursor.x + pos.0 as f32, cursor.y + pos.1 as f32),
        start_pos: pos,
        pinned,
        moved: false,
    })
}

pub fn on_move(drag: &mut DragState, cursor: Point, current_pos: (i32, i32)) -> Outcome {
    if drag.pinned {
        return Outcome::None;
    }
    let abs = (cursor.x + current_pos.0 as f32, cursor.y + current_pos.1 as f32);
    let dx = abs.0 - drag.press_abs.0;
    let dy = abs.1 - drag.press_abs.1;
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

    #[test]
    fn press_left_or_right_starts_drag_middle_does_not() {
        assert!(on_press(sid(), Button::Left, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Right, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Middle, Point::new(5.0, 5.0), (40, 40), false).is_none());
    }

    #[test]
    fn small_motion_is_not_a_drag_and_release_is_a_click() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        assert_eq!(on_move(&mut d, Point::new(12.0, 11.0), (40, 40)), Outcome::None);
        assert!(!d.moved);
        assert_eq!(on_release(d), Outcome::Click(Button::Left));
    }

    #[test]
    fn motion_beyond_threshold_moves_by_absolute_delta() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        // cursor moved +20,+5 within the (not yet moved) surface
        assert_eq!(on_move(&mut d, Point::new(30.0, 15.0), (40, 40)), Outcome::Move((60, 45)));
        // surface now at (60,45); cursor back at local (10,10) means no further motion
        assert_eq!(on_move(&mut d, Point::new(10.0, 10.0), (60, 45)), Outcome::Move((60, 45)));
        // then +3 more in x
        assert_eq!(on_move(&mut d, Point::new(13.0, 10.0), (60, 45)), Outcome::Move((63, 45)));
        assert!(d.moved);
        assert_eq!(on_release(d), Outcome::DragEnd);
    }

    #[test]
    fn pinned_surface_never_moves_but_still_clicks() {
        let mut d = on_press(sid(), Button::Right, Point::new(10.0, 10.0), (40, 40), true).unwrap();
        assert_eq!(on_move(&mut d, Point::new(90.0, 90.0), (40, 40)), Outcome::None);
        assert_eq!(on_release(d), Outcome::Click(Button::Right));
    }
}
