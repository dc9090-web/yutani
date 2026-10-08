//! The popover's drawn pieces (Nostromo handoff): the sand-field scope,
//! the ping sparkline, the rockers, dotted section rules, corner brackets,
//! and the footer's blinking cursor. Anything that moves
//! animates inside its own canvas — `RedrawRequested` steps it and asks for
//! the next frame — so the popover's `view()` is not rebuilt per frame.

use std::time::{Duration, Instant};

use cosmic::Renderer;
use cosmic::iced::mouse;
use cosmic::iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use cosmic::iced::{Color, Point, Rectangle, Size, window};

use yutani::applet::console::RockerState;
use yutani::applet::ping::{Dot, SPARK_H, SPARK_W};
use yutani::applet::rocker::knob_left;
use yutani::applet::sand::{Drive, Sand, Tint};
use yutani::applet::skin::{self, AMBER, BG, CORNER_MARK_PX, DIM, DIMMER, LINE, LINE_2, PHOSPHOR, WHITE};

fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

// ---- scope ---------------------------------------------------------------------

/// The sand field behind the network readouts.
pub struct Scope {
    pub drive: Drive,
}

#[derive(Default)]
pub struct ScopeState {
    sand: Option<Sand>,
    last: Option<Instant>,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Scope {
    type State = ScopeState;

    fn update(&self, state: &mut ScopeState, event: &Event, bounds: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        let dt = state.last.map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        state.last = Some(*now);
        let sand = state.sand.get_or_insert_with(|| Sand::new(bounds.width, bounds.height, 0x5EED));
        sand.resize(bounds.width, bounds.height);
        sand.step(dt, self.drive);
        Some(Action::request_redraw_at(*now + Duration::from_millis(33))) // 30 fps
    }

    fn draw(&self, state: &ScopeState, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        if let Some(sand) = &state.sand {
            for (x, y, size, tint, level) in sand.dots(self.drive) {
                // Only grains inside the scope: canvas geometry is not
                // clipped to its bounds, and in strong wind grains overshoot
                // into the ping row below (2026-10-07).
                if level == 0 || x < 0.0 || y < 0.0 || x + size > bounds.width || y + size > bounds.height {
                    continue;
                }
                let base = match tint {
                    Tint::Downlink => PHOSPHOR,
                    Tint::Uplink => AMBER,
                    Tint::Spark => WHITE,
                };
                frame.fill_rectangle(Point::new(x, y), Size::new(size, size), alpha(base, f32::from(level) / 10.0));
            }
        }
        // 7×7 phosphor corner marks inside each corner.
        let (w, h, m) = (bounds.width, bounds.height, CORNER_MARK_PX);
        for (x, y, dx, dy) in [(0.0, 0.0, 1.0, 1.0), (w, 0.0, -1.0, 1.0), (0.0, h, 1.0, -1.0), (w, h, -1.0, -1.0)] {
            let path = Path::new(|p| {
                p.move_to(Point::new(x + dx * m, y + dy * 0.5));
                p.line_to(Point::new(x + dx * 0.5, y + dy * 0.5));
                p.line_to(Point::new(x + dx * 0.5, y + dy * m));
            });
            frame.stroke(&path, Stroke::default().with_color(PHOSPHOR).with_width(1.0));
        }
        vec![frame.into_geometry()]
    }
}

// ---- ping sparkline -------------------------------------------------------------

pub struct Sparkline {
    pub dots: Vec<Dot>,
    pub avg_y: f32,
    pub color: Color,
    pub live: bool,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Sparkline {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // The dots are laid out on the handoff's 240 px track; the row
        // leaves the sparkline less than that on a 360 px popover, so the
        // track is scaled to the width it actually got rather than clipped
        // (which hid the newest samples at its right end).
        let w = bounds.width;
        let sx = w / SPARK_W;
        let base = SPARK_H - 0.5;
        frame.stroke(&Path::line(Point::new(0.0, base), Point::new(w, base)), Stroke::default().with_color(LINE).with_width(1.0));
        if self.live {
            // The average: white 30 %, dashed 1 4.
            let mut x = 0.0;
            while x < w {
                frame.fill_rectangle(Point::new(x, self.avg_y - 0.5), Size::new(1.0, 1.0), alpha(WHITE, 0.3));
                x += 5.0;
            }
        }
        for d in &self.dots {
            let x = (d.x * sx).min(w - d.size);
            frame.fill_rectangle(Point::new(x, d.y), Size::new(d.size, d.size), alpha(self.color, d.opacity));
        }
        vec![frame.into_geometry()]
    }
}

// ---- rocker ---------------------------------------------------------------------

/// The 38×19 rocker. The knob slides for 120 ms whenever `on` flips.
pub struct Rocker {
    pub state: RockerState,
}

#[derive(Default)]
pub struct RockerAnim {
    shown: Option<bool>,
    flipped: Option<Instant>,
}

impl Rocker {
    fn on(&self) -> bool {
        matches!(self.state, RockerState::On | RockerState::Pending)
    }
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Rocker {
    type State = RockerAnim;

    fn update(&self, s: &mut RockerAnim, event: &Event, _: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        let on = self.on();
        if s.shown.is_some_and(|shown| shown != on) {
            s.flipped = Some(*now);
        }
        s.shown = Some(on);
        let moving = s.flipped.is_some_and(|f| now.saturating_duration_since(f) < Duration::from_millis(yutani::applet::rocker::KNOB_MS));
        moving.then(Action::request_redraw)
    }

    fn draw(&self, s: &RockerAnim, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (fill, edge, knob_ink, opacity) = match self.state {
            RockerState::Off => (None, LINE_2, DIM, 1.0),
            RockerState::On => (Some(PHOSPHOR), PHOSPHOR, BG, 1.0),
            RockerState::Pending => (Some(AMBER), AMBER, BG, 1.0),
            RockerState::Disabled => (None, LINE, DIMMER, 0.45),
        };
        let rect = Path::rounded_rectangle(Point::new(0.5, 0.5), Size::new(skin::TOGGLE_W - 1.0, skin::TOGGLE_H - 1.0), 2.0.into());
        if let Some(f) = fill {
            frame.fill(&rect, alpha(f, opacity));
        }
        frame.stroke(&rect, Stroke::default().with_color(alpha(edge, opacity)).with_width(1.0));
        let since = s.flipped.map(|f| Instant::now().saturating_duration_since(f));
        let left = knob_left(self.on(), since);
        let knob = Path::rounded_rectangle(Point::new(left, 3.0), Size::new(skin::KNOB_PX, skin::KNOB_PX), 1.0.into());
        frame.fill(&knob, alpha(knob_ink, opacity));
        vec![frame.into_geometry()]
    }
}

// ---- dotted rule ------------------------------------------------------------------

/// The section header's dotted rule: 2 px `LINE_2`, 3 px gap, 1 px high.
pub struct DottedRule;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for DottedRule {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let y = (bounds.height / 2.0).floor();
        let mut x = 0.0;
        while x < bounds.width {
            frame.fill_rectangle(Point::new(x, y), Size::new(2.0, 1.0), LINE_2);
            x += 5.0;
        }
        vec![frame.into_geometry()]
    }
}

// ---- corner brackets ----------------------------------------------------------------

/// 8×8 phosphor brackets, top-left and bottom-right, on the two
/// interactive cards.
pub struct Brackets;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Brackets {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h, b) = (bounds.width, bounds.height, skin::BRACKET_PX);
        frame.fill_rectangle(Point::ORIGIN, Size::new(b, 1.0), PHOSPHOR);
        frame.fill_rectangle(Point::ORIGIN, Size::new(1.0, b), PHOSPHOR);
        frame.fill_rectangle(Point::new(w - b, h - 1.0), Size::new(b, 1.0), PHOSPHOR);
        frame.fill_rectangle(Point::new(w - 1.0, h - b), Size::new(1.0, b), PHOSPHOR);
        vec![frame.into_geometry()]
    }
}

// ---- cursor ----------------------------------------------------------------------------

/// The footer's 6×9 block cursor, blinking at 1 Hz, stepped.
pub struct Cursor;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Cursor {
    type State = Option<Instant>;

    fn update(&self, start: &mut Option<Instant>, event: &Event, _: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        let s = *start.get_or_insert(*now);
        let ms = now.saturating_duration_since(s).as_millis() as u64;
        Some(Action::request_redraw_at(*now + Duration::from_millis(500 - ms % 500)))
    }

    fn draw(&self, start: &Option<Instant>, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let on = start.is_none_or(|s| (Instant::now().saturating_duration_since(s).as_millis() / 500).is_multiple_of(2));
        // Off is a block in the popover's ground, never an empty frame: on
        // the panel's popup surface the frames where this canvas drew
        // nothing made the sand field flash brighter and dimmer at the
        // blink's rhythm (2026-10-07).
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), if on { PHOSPHOR } else { BG });
        vec![frame.into_geometry()]
    }
}

// ---- hazard stripe ---------------------------------------------------------------------

/// The Steam notice's hazard stripe: 135° amber/black, 6 px each.
pub struct Hazard;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Hazard {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG);
        let h = bounds.height;
        let mut x = -h;
        while x < bounds.width + h {
            let band = Path::new(|p| {
                p.move_to(Point::new(x, 0.0));
                p.line_to(Point::new(x + 6.0, 0.0));
                p.line_to(Point::new(x + 6.0 - h, h));
                p.line_to(Point::new(x - h, h));
                p.close();
            });
            frame.fill(&band, AMBER);
            x += 12.0;
        }
        vec![frame.into_geometry()]
    }
}

// ---- launch plate -----------------------------------------------------------------------

/// The primary button's engraved rule, 3 px inside the edge, as on the
/// Nostromo's console plates. Solid, not translucent: `ink` mixed into
/// `ground` by `strength`. On the panel's popup surface this canvas lands
/// *over* the label (the preview window draws it under) and its alpha came
/// out far darker, so CRT scanlines struck through the text (2026-10-08) —
/// nothing here may cross the label, and nothing relies on blending.
pub struct LaunchPlate {
    pub ink: Color,
    /// What the rule sits on: the slab's fill, or the popover ground.
    pub ground: Color,
    pub strength: f32,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for LaunchPlate {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width.floor(), bounds.height.floor());
        let mix = |a: f32, b: f32| b + (a - b) * self.strength;
        let rule = Color::from_rgb(mix(self.ink.r, self.ground.r), mix(self.ink.g, self.ground.g), mix(self.ink.b, self.ground.b));
        let i = 3.0;
        frame.fill_rectangle(Point::new(i, i), Size::new(w - 2.0 * i, 1.0), rule);
        frame.fill_rectangle(Point::new(i, h - i - 1.0), Size::new(w - 2.0 * i, 1.0), rule);
        frame.fill_rectangle(Point::new(i, i + 1.0), Size::new(1.0, h - 2.0 * i - 2.0), rule);
        frame.fill_rectangle(Point::new(w - i - 1.0, i + 1.0), Size::new(1.0, h - 2.0 * i - 2.0), rule);
        vec![frame.into_geometry()]
    }
}

/// The launch's four steps as cells, right of the label: outlined when
/// pending, filled when done.
pub struct StepCells {
    pub done: u8,
    pub ink: Color,
}

/// One cell's size and the gap between cells.
pub const STEP_CELL: (f32, f32, f32) = (5.0, 10.0, 3.0);

impl StepCells {
    pub const WIDTH: f32 = 4.0 * STEP_CELL.0 + 3.0 * STEP_CELL.2;
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for StepCells {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (cw, ch, gap) = STEP_CELL;
        let y = ((bounds.height - ch) / 2.0).floor();
        for n in 0..4u8 {
            let x = f32::from(n) * (cw + gap);
            if n < self.done {
                frame.fill_rectangle(Point::new(x, y), Size::new(cw, ch), self.ink);
            } else {
                let edge = self.ink;
                frame.fill_rectangle(Point::new(x, y), Size::new(cw, 1.0), edge);
                frame.fill_rectangle(Point::new(x, y + ch - 1.0), Size::new(cw, 1.0), edge);
                frame.fill_rectangle(Point::new(x, y + 1.0), Size::new(1.0, ch - 2.0), edge);
                frame.fill_rectangle(Point::new(x + cw - 1.0, y + 1.0), Size::new(1.0, ch - 2.0), edge);
            }
        }
        vec![frame.into_geometry()]
    }
}

// ---- overflow glyph ---------------------------------------------------------------------

/// The overflow button's ⋯ (not in B612 Mono): three 2×2 phosphor squares,
/// 3 px apart, centred in the canvas.
pub struct Ellipsis;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Ellipsis {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let total = 3.0 * 2.0 + 2.0 * 3.0;
        let x0 = ((bounds.width - total) / 2.0).floor();
        let y = ((bounds.height - 2.0) / 2.0).floor();
        for n in 0..3 {
            frame.fill_rectangle(Point::new(x0 + n as f32 * 5.0, y), Size::new(2.0, 2.0), PHOSPHOR);
        }
        vec![frame.into_geometry()]
    }
}

// ---- arrow -----------------------------------------------------------------------------

/// Which way an [`Arrow`] points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// A filled triangle standing in for ▲ ▼ ◄, which B612 Mono lacks (▲ ◄)
/// or draws at a different weight than the drawn ones (▼): the readout
/// labels and the focused tag draw all three here so they match.
pub struct Arrow {
    pub direction: Direction,
    pub color: Color,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Arrow {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let [a, b, c] = match self.direction {
            Direction::Up => [Point::new(w / 2.0, 0.0), Point::new(w, h), Point::new(0.0, h)],
            Direction::Down => [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w / 2.0, h)],
            Direction::Left => [Point::new(0.0, h / 2.0), Point::new(w, 0.0), Point::new(w, h)],
            Direction::Right => [Point::new(0.0, 0.0), Point::new(w, h / 2.0), Point::new(0.0, h)],
        };
        let path = Path::new(|p| {
            p.move_to(a);
            p.line_to(b);
            p.line_to(c);
            p.close();
        });
        frame.fill(&path, self.color);
        vec![frame.into_geometry()]
    }
}
