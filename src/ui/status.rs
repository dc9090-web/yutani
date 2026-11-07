//! Per-client state derived from the EVE log (spec §Status): where the
//! character is and what is being done to it right now. Pure; the clock is
//! passed in.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use yutani::eve_logs::EventKind;

pub const DAMAGE_WINDOW: Duration = Duration::from_secs(5);
pub const TACKLE_HOLD: Duration = Duration::from_secs(10);
pub const JAM_HOLD: Duration = Duration::from_secs(5);
pub const NEUT_HOLD: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tackle {
    Scram,
    Point,
}

#[derive(Debug, Default)]
pub struct Status {
    pub system: Option<String>,
    pub docked: bool,
    hits: VecDeque<(Instant, u32)>,
    tackled_until: Option<(Instant, Tackle)>,
    jammed_until: Option<Instant>,
    neuted_until: Option<Instant>,
}

impl Status {
    pub fn apply(&mut self, kind: &EventKind, now: Instant) {
        match kind {
            EventKind::System(s) => self.system = Some(s.clone()),
            EventKind::Docked => self.docked = true,
            EventKind::Undocked(s) => {
                self.docked = false;
                self.system = Some(s.clone());
            }
            EventKind::Damage { amount, .. } => self.hits.push_back((now, *amount)),
            EventKind::Scrambled(_) => self.tackled_until = Some((now + TACKLE_HOLD, Tackle::Scram)),
            EventKind::Disrupted(_) => {
                // A scram outranks a point while both hold.
                if !matches!(self.tackled_until, Some((until, Tackle::Scram)) if until > now) {
                    self.tackled_until = Some((now + TACKLE_HOLD, Tackle::Point));
                }
            }
            EventKind::Jammed(_) => self.jammed_until = Some(now + JAM_HOLD),
            EventKind::Neuted { .. } => self.neuted_until = Some(now + NEUT_HOLD),
        }
        self.tick(now);
    }

    /// Drop what has expired.
    pub fn tick(&mut self, now: Instant) {
        while self.hits.front().is_some_and(|(at, _)| now.duration_since(*at) > DAMAGE_WINDOW) {
            self.hits.pop_front();
        }
        if self.tackled_until.is_some_and(|(u, _)| u <= now) {
            self.tackled_until = None;
        }
        if self.jammed_until.is_some_and(|u| u <= now) {
            self.jammed_until = None;
        }
        if self.neuted_until.is_some_and(|u| u <= now) {
            self.neuted_until = None;
        }
    }

    /// Something timed is still live (the 1 Hz tick is needed).
    pub fn active(&self, now: Instant) -> bool {
        self.under_fire(now) || self.tackled(now) || self.jammed_until.is_some_and(|u| u > now) || self.neuted_until.is_some_and(|u| u > now)
    }

    pub fn under_fire(&self, now: Instant) -> bool {
        self.hits.iter().any(|(at, _)| now.duration_since(*at) <= DAMAGE_WINDOW)
    }

    pub fn tackled(&self, now: Instant) -> bool {
        self.tackled_until.is_some_and(|(u, _)| u > now)
    }

    /// Incoming damage per second over the window, `None` when no hit landed.
    pub fn dps(&self, now: Instant) -> Option<u32> {
        let sum: u32 = self.hits.iter().filter(|(at, _)| now.duration_since(*at) <= DAMAGE_WINDOW).map(|(_, n)| n).sum();
        (sum > 0).then(|| sum / DAMAGE_WINDOW.as_secs() as u32)
    }

    pub fn tags(&self, now: Instant) -> Vec<&'static str> {
        let mut v = Vec::new();
        match self.tackled_until {
            Some((u, Tackle::Scram)) if u > now => v.push("SCRAM"),
            Some((u, Tackle::Point)) if u > now => v.push("POINT"),
            _ => {}
        }
        if self.jammed_until.is_some_and(|u| u > now) {
            v.push("JAM");
        }
        if self.neuted_until.is_some_and(|u| u > now) {
            v.push("NEUT");
        }
        v
    }

    /// Red border: shot at or held.
    pub fn alarm(&self, now: Instant) -> bool {
        self.under_fire(now) || self.tackled(now)
    }
}

/// The second caption line, `None` when there is nothing to say.
pub fn caption(status: &Status, now: Instant) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    match (&status.system, status.docked) {
        (Some(s), true) => parts.push(format!("Docked · {s}")),
        (Some(s), false) => parts.push(s.clone()),
        (None, true) => parts.push("Docked".into()),
        (None, false) => {}
    }
    if let Some(dps) = status.dps(now) {
        parts.push(format!("{dps} dps"));
    }
    parts.extend(status.tags(now).into_iter().map(str::to_string));
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dmg(n: u32) -> EventKind {
        EventKind::Damage { amount: n, from: "x".into(), weapon: None }
    }

    #[test]
    fn dps_is_the_window_sum_over_its_length() {
        let t0 = Instant::now();
        let mut s = Status::default();
        assert_eq!(s.dps(t0), None);
        s.apply(&dmg(100), t0);
        s.apply(&dmg(150), t0 + Duration::from_secs(1));
        assert_eq!(s.dps(t0 + Duration::from_secs(1)), Some(50));
        assert!(s.under_fire(t0 + Duration::from_secs(1)));
        s.tick(t0 + Duration::from_secs(7));
        assert_eq!(s.dps(t0 + Duration::from_secs(7)), None);
        assert!(!s.under_fire(t0 + Duration::from_secs(7)));
        assert!(!s.active(t0 + Duration::from_secs(7)));
    }

    #[test]
    fn holds_expire_and_tags_keep_their_order() {
        let t0 = Instant::now();
        let mut s = Status::default();
        s.apply(&EventKind::Neuted { gj: 1, from: "n".into() }, t0);
        s.apply(&EventKind::Jammed("j".into()), t0);
        s.apply(&EventKind::Scrambled("s".into()), t0);
        assert_eq!(s.tags(t0), vec!["SCRAM", "JAM", "NEUT"]);
        assert!(s.alarm(t0));
        let t6 = t0 + Duration::from_secs(6);
        assert_eq!(s.tags(t6), vec!["SCRAM"]);
        let t11 = t0 + Duration::from_secs(11);
        assert_eq!(s.tags(t11), Vec::<&str>::new());
        assert!(!s.alarm(t11));
        s.apply(&EventKind::Disrupted("p".into()), t11);
        assert_eq!(s.tags(t11), vec!["POINT"]);
    }

    #[test]
    fn system_and_dock_state() {
        let t0 = Instant::now();
        let mut s = Status::default();
        s.apply(&EventKind::System("Jita".into()), t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("Jita"));
        s.apply(&EventKind::Docked, t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("Docked · Jita"));
        s.apply(&EventKind::Undocked("Perimeter".into()), t0);
        assert!(!s.docked);
        assert_eq!(caption(&s, t0).as_deref(), Some("Perimeter"));
        s.apply(&EventKind::System("Urlen".into()), t0);
        assert_eq!(s.system.as_deref(), Some("Urlen"));
    }

    #[test]
    fn caption_composes_system_dps_and_tags() {
        let t0 = Instant::now();
        let mut s = Status::default();
        assert_eq!(caption(&s, t0), None);
        s.apply(&dmg(1200), t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("240 dps"));
        s.apply(&EventKind::System("Korsiki".into()), t0);
        s.apply(&EventKind::Scrambled("s".into()), t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("Korsiki · 240 dps · SCRAM"));
    }
}
