//! Rates from two counter samples. The daemon reports cumulative,
//! monotonic byte counters; the applet turns consecutive polls into
//! bytes/second. A counter that went *down* means the interface was
//! recreated, so the rate is 0 for that step, never negative.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    /// Download, bytes per second.
    pub rx: f64,
    /// Upload, bytes per second.
    pub tx: f64,
}

/// Holds the previous sample. `t_ms` is any monotonic millisecond clock
/// (the applet passes `start.elapsed().as_millis()`), which keeps this
/// testable without a real clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sampler {
    last: Option<(u64, u64, u64)>,
}

impl Sampler {
    pub fn push(&mut self, rx: u64, tx: u64, t_ms: u64) -> Rates {
        let rates = match self.last {
            Some((prev_rx, prev_tx, prev_t)) if t_ms > prev_t => {
                let secs = (t_ms - prev_t) as f64 / 1_000.0;
                Rates {
                    rx: rx.saturating_sub(prev_rx) as f64 / secs,
                    tx: tx.saturating_sub(prev_tx) as f64 / secs,
                }
            }
            _ => Rates::default(),
        };
        self.last = Some((rx, tx, t_ms));
        rates
    }

    /// Forget the previous sample, so the next `push` reports 0 (used when
    /// the daemon goes away or the tunnel drops).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sample_has_no_rate() {
        let mut s = Sampler::default();
        assert_eq!(s.push(1_000, 2_000, 0), Rates { rx: 0.0, tx: 0.0 });
    }

    #[test]
    fn a_rate_is_the_delta_over_the_elapsed_seconds() {
        let mut s = Sampler::default();
        s.push(1_000, 2_000, 1_000);
        assert_eq!(s.push(1_500, 4_000, 2_000), Rates { rx: 500.0, tx: 2_000.0 });
        // Half a second later, half as many bytes → the same rate.
        assert_eq!(s.push(1_750, 5_000, 2_500), Rates { rx: 500.0, tx: 2_000.0 });
    }

    #[test]
    fn a_counter_reset_yields_zero_not_a_negative_rate() {
        let mut s = Sampler::default();
        s.push(10_000, 10_000, 1_000);
        // The interface was recreated: both counters went backwards.
        assert_eq!(s.push(40, 10_500, 2_000), Rates { rx: 0.0, tx: 500.0 });
        // …and the new baseline is the small value, not the old one.
        assert_eq!(s.push(1_040, 11_500, 3_000), Rates { rx: 1_000.0, tx: 1_000.0 });
    }

    #[test]
    fn two_samples_at_the_same_instant_yield_zero_and_reset_forgets_everything() {
        let mut s = Sampler::default();
        s.push(0, 0, 5_000);
        assert_eq!(s.push(9_999, 9_999, 5_000), Rates { rx: 0.0, tx: 0.0 });
        let mut s = Sampler::default();
        s.push(1_000, 1_000, 1_000);
        s.reset();
        assert_eq!(s.push(9_000, 9_000, 2_000), Rates { rx: 0.0, tx: 0.0 });
    }
}
