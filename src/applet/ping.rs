//! The ping row (Nostromo handoff "Ping row"): the last 34 probes to
//! Tranquility, their stats, the quality word and the sparkline's dots.
//! Samples are fed by the daemon's probe, keyed by its sequence number
//! (Phase 2); quality is judged relative to the session's own median, not
//! absolute thresholds.

use std::collections::VecDeque;

pub const WINDOW: usize = 34;
pub const SPARK_W: f32 = 240.0;
pub const SPARK_H: f32 = 22.0;
const DASH: &str = "—";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Nominal,
    Degraded,
    Poor,
    Idle,
}

impl Quality {
    pub fn word(self) -> &'static str {
        match self {
            Quality::Nominal => "NOMINAL",
            Quality::Degraded => "DEGRADED",
            Quality::Poor => "POOR",
            Quality::Idle => "IDLE",
        }
    }

    /// The quality word for the newest success `latest`, judged against the
    /// window's own median `baseline`, with the window's jitter and loss
    /// (Nostromo spec, Phase 2). The `+ n` floors keep a few-ms ping from
    /// flapping on noise.
    pub fn judge(latest: f32, baseline: f32, jitter: f32, loss_pct: f32) -> Self {
        if loss_pct > 5.0 || latest > 1.6 * baseline + 20.0 {
            Quality::Poor
        } else if loss_pct > 0.0 || latest > 1.25 * baseline + 10.0 || jitter > 0.15 * baseline + 5.0 {
            Quality::Degraded
        } else {
            Quality::Nominal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PingSummary {
    pub value: String,
    pub quality: Quality,
    pub stats: String,
    pub jitter_loss: String,
    pub dots: Vec<Dot>,
    /// The dashed average line's y inside the 240×22 box.
    pub avg_y: f32,
}

/// `None` samples are losses (the probe timed out or was refused).
#[derive(Clone, Debug, Default)]
pub struct PingWindow {
    samples: VecDeque<Option<f32>>,
    last_seq: Option<u64>,
}

fn baseline(size: f32) -> f32 {
    SPARK_H - 2.0 - size / 2.0
}

impl PingWindow {
    /// Take `sample` if `seq` is a probe this window has not seen.
    pub fn push_seq(&mut self, seq: u64, sample: Option<f32>) {
        match self.last_seq {
            Some(last) if seq == last => return,
            // The probe restarted (tunnel worker restart): a new session.
            Some(last) if seq < last => self.clear(),
            _ => {}
        }
        self.last_seq = Some(seq);
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn summary(&self, live: bool) -> PingSummary {
        let ok: Vec<f32> = self.samples.iter().flatten().copied().collect();
        let n = self.samples.len();
        let positions = |i: usize, size: f32| {
            // Right-aligned: the newest sample sits at slot 33.
            let slot = WINDOW - n + i;
            (slot as f32 * SPARK_W / (WINDOW - 1) as f32).min(SPARK_W - size)
        };
        if !live || n == 0 {
            let dots = (0..n)
                .map(|i| {
                    let size = if i == n - 1 { 3.0 } else { 2.0 };
                    let slot = WINDOW - n + i;
                    Dot { x: positions(i, size), y: baseline(size), size, opacity: 0.3 + 0.7 * slot as f32 / (WINDOW - 1) as f32 }
                })
                .collect();
            return PingSummary {
                value: DASH.into(),
                quality: Quality::Idle,
                stats: format!("MIN {DASH} · AVG {DASH} · MAX {DASH}"),
                jitter_loss: format!("JIT {DASH} · LOSS {DASH}"),
                dots,
                avg_y: SPARK_H - 2.0,
            };
        }
        let (min, max) = ok.iter().fold((f32::MAX, 0.0_f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        let avg = if ok.is_empty() { 0.0 } else { ok.iter().sum::<f32>() / ok.len() as f32 };
        let jitter = if ok.len() > 1 { ok.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (ok.len() - 1) as f32 } else { 0.0 };
        let loss = (n - ok.len()) as f32 / n as f32 * 100.0;
        let scale = max.max(1.0) * 1.15;
        let y_of = |v: f32| SPARK_H - 2.0 - v / scale * (SPARK_H - 5.0);
        let dots = self
            .samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let size = if i == n - 1 { 3.0 } else { 2.0 };
                let slot = WINDOW - n + i;
                let y = s.map_or(baseline(size), |v| y_of(v) - size / 2.0);
                Dot { x: positions(i, size), y, size, opacity: 0.3 + 0.7 * slot as f32 / (WINDOW - 1) as f32 }
            })
            .collect();
        let latest = ok.last().copied();
        let median = {
            let mut sorted = ok.clone();
            sorted.sort_by(f32::total_cmp);
            match sorted.len() {
                0 => 0.0,
                l if l % 2 == 1 => sorted[l / 2],
                l => (sorted[l / 2 - 1] + sorted[l / 2]) / 2.0,
            }
        };
        let int = |v: f32| format!("{}", v.round() as i64);
        if ok.is_empty() {
            return PingSummary {
                value: DASH.into(),
                quality: Quality::Poor,
                stats: format!("MIN {DASH} · AVG {DASH} · MAX {DASH}"),
                jitter_loss: format!("JIT {DASH} · LOSS {loss:.1}%"),
                dots,
                avg_y: SPARK_H - 2.0,
            };
        }
        PingSummary {
            value: latest.map_or_else(|| DASH.into(), int),
            quality: latest.map_or(Quality::Poor, |l| Quality::judge(l, median, jitter, loss)),
            stats: format!("MIN {} · AVG {} · MAX {}", int(min), int(avg), int(max)),
            jitter_loss: format!("JIT {jitter:.1} · LOSS {loss:.1}%"),
            dots,
            avg_y: y_of(avg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(samples: &[Option<f32>]) -> PingWindow {
        let mut w = PingWindow::default();
        for (i, s) in samples.iter().enumerate() {
            w.push_seq(i as u64 + 1, *s);
        }
        w
    }

    #[test]
    fn a_down_tunnel_is_idle_with_dashes_and_dots_on_the_baseline() {
        let s = window(&[Some(5.0); 10]).summary(false);
        assert_eq!(s.quality, Quality::Idle);
        assert_eq!((s.value.as_str(), s.stats.as_str(), s.jitter_loss.as_str()), ("—", "MIN — · AVG — · MAX —", "JIT — · LOSS —"));
        assert!(s.dots.iter().all(|d| d.y == SPARK_H - 2.0 - d.size / 2.0));
        assert_eq!(PingWindow::default().summary(true).quality, Quality::Idle, "no samples yet");
    }

    #[test]
    fn stats_over_the_window() {
        let s = window(&[Some(3.0), Some(5.0), None, Some(12.0), Some(4.0)]).summary(true);
        assert_eq!(s.value, "4");
        assert_eq!(s.stats, "MIN 3 · AVG 6 · MAX 12");
        // jitter = mean |Δ| over consecutive successes: (2 + 7 + 8) / 3
        assert_eq!(s.jitter_loss, "JIT 5.7 · LOSS 20.0%");
        assert_eq!(s.quality, Quality::Poor, "20 % loss is over the 5 % line");
    }

    /// Quality is judged against the session's own median, so 360 ms from
    /// the far side of the world is NOMINAL when that is normal (Nostromo spec, Phase 2).
    #[test]
    fn quality_is_relative_to_the_sessions_normal() {
        let q = Quality::judge;
        assert_eq!(q(360.0, 360.0, 3.0, 0.0), Quality::Nominal);
        assert_eq!(q(5.0, 4.0, 1.0, 0.0), Quality::Nominal, "the +10 ms floor keeps a tiny ping calm");
        assert_eq!(q(460.0, 360.0, 3.0, 0.0), Quality::Nominal, "1.25 × 360 + 10 = 460: equal is not above");
        assert_eq!(q(461.0, 360.0, 3.0, 0.0), Quality::Degraded);
        assert_eq!(q(360.0, 360.0, 60.0, 0.0), Quality::Degraded, "jitter above 0.15 × 360 + 5 = 59");
        assert_eq!(q(360.0, 360.0, 3.0, 2.9), Quality::Degraded, "any loss");
        assert_eq!(q(597.0, 360.0, 3.0, 0.0), Quality::Poor, "> 1.6 × 360 + 20 = 596");
        assert_eq!(q(360.0, 360.0, 3.0, 5.9), Quality::Poor, "loss above 5 %");
    }

    #[test]
    fn a_steady_long_ping_reads_nominal_and_a_lost_window_reads_poor() {
        let steady = window(&[Some(360.0), Some(361.0), Some(359.5), Some(360.4)]).summary(true);
        assert_eq!((steady.value.as_str(), steady.quality), ("360", Quality::Nominal));
        assert_eq!(window(&[None, None]).summary(true).quality, Quality::Poor, "all lost");
        assert_eq!(Quality::Nominal.word(), "NOMINAL");
    }

    #[test]
    fn the_window_keeps_34_and_ignores_a_repeated_sequence() {
        let mut w = window(&[Some(1.0); 40]);
        assert_eq!(w.summary(true).dots.len(), WINDOW);
        w.push_seq(40, Some(500.0));
        assert_eq!(w.summary(true).value, "1", "seq 40 was already taken");
        w.push_seq(41, Some(500.0));
        assert_eq!(w.summary(true).value, "500");
    }

    #[test]
    fn a_lower_sequence_is_a_new_probe_session() {
        let mut w = window(&[Some(1.0); 41]);
        w.push_seq(1, Some(7.0));
        let s = w.summary(true);
        assert_eq!((s.value.as_str(), s.dots.len()), ("7", 1), "old samples dropped");
        w.push_seq(1, Some(9.0));
        assert_eq!(w.summary(true).value, "7", "equal is still ignored");
    }

    /// Newest at the right edge, 3 px and opaque; oldest 2 px at .3.
    #[test]
    fn dots_run_oldest_to_newest_left_to_right() {
        let s = window(&[Some(10.0); WINDOW]).summary(true);
        let (first, last) = (s.dots[0], s.dots[WINDOW - 1]);
        assert_eq!((first.x, first.size, first.opacity), (0.0, 2.0, 0.3));
        assert_eq!((last.x, last.size), (SPARK_W - 3.0, 3.0));
        assert!((last.opacity - 1.0).abs() < 1e-6);
        let short = window(&[Some(10.0); 2]).summary(true);
        assert_eq!(short.dots.last().unwrap().x, SPARK_W - 3.0, "a short window still ends at now");
    }
}
