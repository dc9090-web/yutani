//! The throughput graph's data (redesign spec §2): the last [`SAMPLES`]
//! rate samples, one per poll, each direction scaled against its own
//! rolling maximum — so a quiet tunnel still shows its shape and a burst
//! one way does not flatten the other.

use std::collections::VecDeque;

use super::rate::Rates;

/// 34 columns, as designed: ≈ 60 s of history at the popup's 1 Hz poll.
pub const SAMPLES: usize = 34;

#[derive(Clone, Debug, PartialEq)]
pub struct History {
    samples: VecDeque<Rates>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// Starts full of zeros, so the graph has its baseline from the first
    /// draw rather than growing in from the right.
    pub fn new() -> Self {
        History { samples: std::iter::repeat_n(Rates::default(), SAMPLES).collect() }
    }

    pub fn push(&mut self, rates: Rates) {
        if self.samples.len() >= SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(rates);
    }

    /// Back to the flat baseline (the daemon went away).
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// `(up, down)` per column in `0..=1`, oldest first, each direction
    /// against its own window maximum. A window with no traffic is all
    /// zeros — the view draws the 1 px baseline, not this.
    pub fn bars(&self) -> Vec<(f32, f32)> {
        let max_tx = self.samples.iter().map(|r| r.tx).fold(0.0_f64, f64::max);
        let max_rx = self.samples.iter().map(|r| r.rx).fold(0.0_f64, f64::max);
        let scale = |v: f64, max: f64| if max > 0.0 { (v / max).clamp(0.0, 1.0) as f32 } else { 0.0 };
        self.samples.iter().map(|r| (scale(r.tx, max_tx), scale(r.rx, max_rx))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_holds_the_last_34_samples_oldest_first() {
        let mut h = History::new();
        assert_eq!(h.bars().len(), SAMPLES);
        assert!(h.bars().iter().all(|b| *b == (0.0, 0.0)));
        for i in 1..=40u64 {
            h.push(Rates { tx: i as f64, rx: 0.0 });
        }
        let bars = h.bars();
        assert_eq!(bars.len(), SAMPLES);
        // Samples 7..=40 remain; the newest is the maximum.
        assert!((bars[0].0 - 7.0 / 40.0).abs() < 1e-6);
        assert_eq!(bars[SAMPLES - 1], (1.0, 0.0));
    }

    /// Each direction has its own scale: a busy download must not squash
    /// a quiet upload into the baseline.
    #[test]
    fn each_direction_scales_against_its_own_maximum() {
        let mut h = History::new();
        h.push(Rates { tx: 10.0, rx: 1_000_000.0 });
        h.push(Rates { tx: 5.0, rx: 500_000.0 });
        let bars = h.bars();
        assert_eq!(bars[SAMPLES - 2], (1.0, 1.0));
        assert_eq!(bars[SAMPLES - 1], (0.5, 0.5));
        h.clear();
        assert!(h.bars().iter().all(|b| *b == (0.0, 0.0)));
    }
}
