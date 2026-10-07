//! The scope's sand field (Nostromo handoff "The sand field"): 1 000 grains
//! riding a slow dune surface, blown right by a wind that is the tunnel's
//! throughput. The colour mix *is* the up/down ratio, the speed *is* the
//! rate. Pure: the canvas program in the applet binary owns one, steps it
//! on every redraw and paints `dots()`.

pub const GRAINS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Downlink,
    Uplink,
    Spark,
}

/// What drives the field: tunnel up and service running, and the rates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    pub live: bool,
    pub up_kbps: f32,
    pub down_kbps: f32,
}

pub fn wind_target(d: Drive) -> f32 {
    if d.live { 10.0 + (d.up_kbps + d.down_kbps).min(120.0) * 0.85 } else { 2.0 }
}

/// The uplink's share of the traffic; 30 % when there is none.
pub fn up_share(d: Drive) -> f32 {
    let total = d.up_kbps + d.down_kbps;
    if d.live && total >= IDLE_KBPS { d.up_kbps / total } else { 0.3 }
}

/// Below this much traffic the colour mix is the idle 30 %: an idle link's
/// trickle is all-up one second and all-down the next, which flipped the
/// whole field between amber and phosphor every poll (seen 2026-10-07).
pub const IDLE_KBPS: f32 = 1.0;

/// How fast the field's colour mix follows the traffic: about 1 s to
/// close most of a change, so the grains drift between colours.
const SHARE_EASE_PER_S: f32 = 1.5;

#[derive(Clone, Copy, Debug)]
struct Grain {
    x: f32,
    y: f32,
    off: f32,
    ph: f32,
    size: f32,
    b: f32,
    k: f32,
}

pub struct Sand {
    w: f32,
    h: f32,
    t: f32,
    wind: Option<f32>,
    /// The eased uplink share the tints follow (`None` before the first step).
    share: Option<f32>,
    grains: Vec<Grain>,
    rng: u64,
}

impl Sand {
    pub fn new(w: f32, h: f32, seed: u64) -> Self {
        let mut s = Sand { w, h, t: 0.0, wind: None, share: None, grains: Vec::with_capacity(GRAINS), rng: seed.max(1) };
        s.seed_grains();
        s
    }

    /// Re-seed the grains when the scope's size really changed.
    pub fn resize(&mut self, w: f32, h: f32) {
        if (w - self.w).abs() > 0.5 || (h - self.h).abs() > 0.5 {
            self.w = w;
            self.h = h;
            self.seed_grains();
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn grain(&mut self, x: f32) -> Grain {
        let off = (self.rand() - 0.62) * 28.0;
        Grain {
            x,
            y: self.h * 0.64 + off,
            off,
            ph: self.rand() * std::f32::consts::TAU,
            size: if self.rand() < 0.12 { 1.5 } else { 1.0 },
            b: 0.3 + self.rand() * 0.7,
            k: self.rand(),
        }
    }

    fn seed_grains(&mut self) {
        self.grains.clear();
        for _ in 0..GRAINS {
            let x = self.rand() * self.w;
            let g = self.grain(x);
            self.grains.push(g);
        }
    }

    fn surface(&self, x: f32) -> f32 {
        let t = self.t;
        self.h * 0.64 + 11.0 * (x * 0.011 + t * 0.25).sin() + 5.0 * (x * 0.029 - t * 0.17 + 1.3).sin() + 3.0 * (x * 0.07 + t * 0.6).sin()
    }

    /// Advance `dt` seconds (capped at 50 ms).
    pub fn step(&mut self, dt: f32, d: Drive) {
        let dt = dt.clamp(0.0, 0.05);
        self.t += dt;
        let target = wind_target(d);
        let wind = match self.wind {
            None => target,
            Some(w) => w + (target - w) * (dt * 2.5).min(1.0),
        };
        self.wind = Some(wind);
        let target = up_share(d);
        self.share = Some(match self.share {
            None => target,
            Some(s) => s + (target - s) * (dt * SHARE_EASE_PER_S).min(1.0),
        });
        let t = self.t;
        for i in 0..self.grains.len() {
            let g = self.grains[i];
            let ys = self.surface(g.x);
            let gust = 0.5 + 0.5 * (g.x * 0.02 + t * 1.7 + g.ph).sin();
            let vx = wind * (0.5 + gust) * (0.6 + g.k * 0.8) * if g.off < 0.0 { 1.6 } else { 0.7 };
            let vy = (ys + g.off - g.y) * 2.5 + wind * 0.15 * (g.x * 0.05 + t * 2.3 + g.ph).sin();
            let mut g = Grain { x: g.x + vx * dt, y: g.y + vy * dt, ..g };
            if g.x > self.w + 2.0 {
                g = self.grain(-2.0);
            }
            self.grains[i] = g;
        }
    }

    /// Every grain as `(x, y, size, tint, alpha level 0..=10)`.
    pub fn dots(&self, d: Drive) -> impl Iterator<Item = (f32, f32, f32, Tint, u8)> + '_ {
        let share = self.share.unwrap_or_else(|| up_share(d));
        let dim = if d.live { 1.0 } else { 0.45 };
        let t = self.t;
        self.grains.iter().map(move |g| {
            let a = g.b * (0.5 + 0.5 * (t * 2.2 + g.ph * 3.0).sin()) * dim;
            let tint = if g.k > 0.975 {
                Tint::Spark
            } else if g.k < share {
                Tint::Uplink
            } else {
                Tint::Downlink
            };
            (g.x, g.y, g.size, tint, (a * 10.0).round().clamp(0.0, 10.0) as u8)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: Drive = Drive { live: true, up_kbps: 30.0, down_kbps: 70.0 };
    const DOWN: Drive = Drive { live: false, up_kbps: 0.0, down_kbps: 0.0 };

    #[test]
    fn wind_follows_throughput_and_dies_with_the_tunnel() {
        assert_eq!(wind_target(DOWN), 2.0);
        assert_eq!(wind_target(Drive { live: true, up_kbps: 0.0, down_kbps: 0.0 }), 10.0);
        assert!((wind_target(LIVE) - (10.0 + 100.0 * 0.85)).abs() < 1e-4);
        assert!((wind_target(Drive { live: true, up_kbps: 900.0, down_kbps: 900.0 }) - (10.0 + 120.0 * 0.85)).abs() < 1e-4, "capped at 120");
        assert_eq!(up_share(LIVE), 0.3);
        assert_eq!(up_share(DOWN), 0.3, "idle: 30 % amber");
        assert_eq!(up_share(Drive { live: true, up_kbps: 0.0, down_kbps: 0.0 }), 0.3);
        assert_eq!(up_share(Drive { live: true, up_kbps: 0.4, down_kbps: 0.0 }), 0.3, "a trickle is idle");
    }

    /// One poll all-up, the next all-down: the field must not flip colour
    /// wholesale — the mix eases toward the new share.
    #[test]
    fn the_colour_mix_eases_instead_of_flipping() {
        let up = Drive { live: true, up_kbps: 50.0, down_kbps: 0.0 };
        let down = Drive { live: true, up_kbps: 0.0, down_kbps: 50.0 };
        let mut s = Sand::new(338.0, 104.0, 4);
        for _ in 0..60 {
            s.step(0.033, up);
        }
        let amber = |s: &Sand, d| s.dots(d).filter(|(.., t, _)| *t == Tint::Uplink).count();
        let before = amber(&s, up);
        assert!(before > 900, "{before}");
        s.step(0.033, down);
        let after = amber(&s, down);
        assert!(after > before * 8 / 10, "one frame after the flip most grains are still amber: {after} of {before}");
        for _ in 0..90 {
            s.step(0.033, down);
        }
        assert!(amber(&s, down) < 100, "three seconds later the mix has followed");
    }

    #[test]
    fn a_seed_reproduces_the_field() {
        let (mut a, mut b) = (Sand::new(338.0, 104.0, 7), Sand::new(338.0, 104.0, 7));
        for _ in 0..10 {
            a.step(0.016, LIVE);
            b.step(0.016, LIVE);
        }
        assert_eq!(a.dots(LIVE).collect::<Vec<_>>(), b.dots(LIVE).collect::<Vec<_>>());
        assert_eq!(a.dots(LIVE).count(), GRAINS);
    }

    #[test]
    fn grains_blow_right_and_respawn_at_the_left_edge() {
        let mut s = Sand::new(338.0, 104.0, 1);
        for _ in 0..2_000 {
            s.step(0.05, Drive { live: true, up_kbps: 500.0, down_kbps: 500.0 });
        }
        assert!(s.dots(LIVE).all(|(x, _, _, _, _)| (-2.0..=340.0).contains(&x)), "nothing escapes past W + 2");
    }

    #[test]
    fn a_long_frame_is_capped_at_50_ms() {
        let (mut a, mut b) = (Sand::new(338.0, 104.0, 3), Sand::new(338.0, 104.0, 3));
        a.step(5.0, LIVE);
        b.step(0.05, LIVE);
        assert_eq!(a.dots(LIVE).collect::<Vec<_>>(), b.dots(LIVE).collect::<Vec<_>>());
    }

    #[test]
    fn alpha_levels_are_quantised_and_dim_when_down() {
        let mut s = Sand::new(338.0, 104.0, 5);
        s.step(0.016, LIVE);
        assert!(s.dots(LIVE).all(|(.., q)| q <= 10));
        let live_sum: u32 = s.dots(LIVE).map(|(.., q)| u32::from(q)).sum();
        let down_sum: u32 = s.dots(DOWN).map(|(.., q)| u32::from(q)).sum();
        assert!(down_sum < live_sum, "down grains are at 45 %");
    }

    #[test]
    fn tints_split_by_uplink_share_with_rare_sparks() {
        let s = Sand::new(338.0, 104.0, 11);
        let all_up = Drive { live: true, up_kbps: 10.0, down_kbps: 0.0 };
        let all_down = Drive { live: true, up_kbps: 0.0, down_kbps: 10.0 };
        assert!(s.dots(all_down).all(|(.., t, _)| t != Tint::Uplink));
        assert!(s.dots(all_up).all(|(.., t, _)| t != Tint::Downlink));
        let sparks = s.dots(LIVE).filter(|(.., t, _)| *t == Tint::Spark).count();
        assert!(sparks > 5 && sparks < 60, "≈2.5 %: {sparks}");
    }

    #[test]
    fn resize_reseeds_only_on_a_real_change() {
        let mut s = Sand::new(338.0, 104.0, 2);
        s.step(0.016, LIVE);
        let before: Vec<_> = s.dots(LIVE).collect();
        s.resize(338.0, 104.0);
        assert_eq!(before, s.dots(LIVE).collect::<Vec<_>>());
        s.resize(300.0, 104.0);
        assert!(s.dots(LIVE).all(|(x, ..)| x <= 302.0));
    }
}
