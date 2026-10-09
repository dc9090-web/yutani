//! The popover's drawn pieces (Nostromo handoff): the rockers, dotted
//! section rules, corner brackets, hazard stripes and the footer's
//! blinking cursor. Anything that moves
//! animates inside its own canvas — `RedrawRequested` steps it and asks for
//! the next frame — so the popover's `view()` is not rebuilt per frame.

use std::time::{Duration, Instant};

use cosmic::Renderer;
use cosmic::iced::mouse;
use cosmic::iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use cosmic::iced::{Color, Point, Rectangle, Size, window};

use yutani::applet::console::RockerState;
use yutani::applet::rocker::knob_left;
use yutani::applet::skin::{self, AMBER, BG, DIM, DIMMER, LINE, LINE_2, PHOSPHOR};

fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
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

/// The Launch EVE button's hazard caps: the [`Hazard`] stripe in
/// phosphor, each band with a softer halo either side, breathing slowly
/// between dim and full. Every colour is mixed solid over `BG` — on the
/// panel's popup surface canvas alpha renders far darker than intended.
pub struct GlowHazard;

/// One breath, dim → bright → dim.
const PULSE: Duration = Duration::from_millis(3200);
/// Redraw step while it breathes: smooth enough at this pace.
const PULSE_FRAME: Duration = Duration::from_millis(50);

fn mix(over: Color, ink: Color, t: f32) -> Color {
    Color::from_rgb(over.r + (ink.r - over.r) * t, over.g + (ink.g - over.g) * t, over.b + (ink.b - over.b) * t)
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for GlowHazard {
    type State = Option<Instant>;

    fn update(&self, start: &mut Option<Instant>, event: &Event, _: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        start.get_or_insert(*now);
        Some(Action::request_redraw_at(*now + PULSE_FRAME))
    }

    fn draw(&self, start: &Option<Instant>, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG);
        let t = start.map_or(0.0, |s| Instant::now().saturating_duration_since(s).as_secs_f32() / PULSE.as_secs_f32());
        // 0 → 1 → 0 once per breath, eased at both ends.
        let breath = 0.5 - 0.5 * (t * std::f32::consts::TAU).cos();
        let level = 0.55 + 0.45 * breath;
        let core = mix(BG, PHOSPHOR, level);
        let halo = mix(BG, PHOSPHOR, 0.22 * level);
        let h = bounds.height;
        let band = |x: f32, w: f32| {
            Path::new(|p| {
                p.move_to(Point::new(x, 0.0));
                p.line_to(Point::new(x + w, 0.0));
                p.line_to(Point::new(x + w - h, h));
                p.line_to(Point::new(x - h, h));
                p.close();
            })
        };
        let mut x = -h;
        while x < bounds.width + h {
            frame.fill(&band(x - 2.0, 10.0), halo);
            frame.fill(&band(x, 6.0), core);
            x += 12.0;
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

/// A filled left-pointing triangle standing in for ◄, which B612 Mono
/// lacks: the focused tag's arrow.
pub struct Arrow {
    pub color: Color,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Arrow {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let [a, b, c] = [Point::new(0.0, h / 2.0), Point::new(w, 0.0), Point::new(w, h)];
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
