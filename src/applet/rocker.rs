//! The 38×19 square rocker (Nostromo handoff "Rocker"): a 13 px knob at
//! `left 2` (off) or `left 21` (on), sliding between them in 120 ms.

use std::time::Duration;

pub const KNOB_MS: u64 = 120;
const OFF: f32 = 2.0;
const ON: f32 = 21.0;

/// The knob's left edge, `since_flip` after the rocker last changed state
/// (`None`: at rest).
pub fn knob_left(on: bool, since_flip: Option<Duration>) -> f32 {
	let (from, to) = if on { (OFF, ON) } else { (ON, OFF) };
	let Some(d) = since_flip else { return to };
	// Whole milliseconds: exact at the test points, smooth enough at 60 fps.
	let t = (d.as_millis() as f32 / KNOB_MS as f32).min(1.0);
	from + (to - from) * t
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_knob_rests_at_2_off_and_21_on() {
		assert_eq!(knob_left(false, None), 2.0);
		assert_eq!(knob_left(true, None), 21.0);
	}

	#[test]
	fn the_knob_travels_in_120_ms() {
		assert_eq!(knob_left(true, Some(Duration::ZERO)), 2.0);
		assert_eq!(knob_left(true, Some(Duration::from_millis(60))), 11.5);
		assert_eq!(knob_left(true, Some(Duration::from_millis(120))), 21.0);
		assert_eq!(knob_left(true, Some(Duration::from_secs(9))), 21.0);
		assert_eq!(knob_left(false, Some(Duration::from_millis(60))), 11.5);
	}
}
