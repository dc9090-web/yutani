//! Launch EVE (Nostromo spec, Phase 3): the four steps from "ask Steam" to
//! "the new client has its hotkey", as a pure state machine. The daemon
//! feeds it observations — ticks with whether the EVE Launcher process is
//! alive, clients appearing, placed and closing, Steam windows changing —
//! and applies the effects it returns. Generic over the window handle so
//! it is tested without Wayland. `LaunchState` is what the status reply
//! carries to the applet.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use serde::{Deserialize, Serialize};

pub const STEAM_TIMEOUT_MS: u64 = 60_000;
pub const LAUNCHER_GONE_MS: u64 = 10_000;
pub const DONE_LINGER_MS: u64 = 2_000;
pub const FAILED_LINGER_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchState {
    pub steps: [Step; 4],
    /// `CTRL+ALT+n`: predicted (clients + 1) until the new client is placed,
    /// then its real slot; `—` past 9.
    pub hotkey: String,
    /// Why a step failed, upper case, for the log's red line.
    pub failed: Option<String>,
}

impl LaunchState {
    /// The step the button counts: the first that is not done, 1-based.
    pub fn current(&self) -> u8 {
        self.steps.iter().position(|s| *s != Step::Done).map_or(4, |i| i as u8 + 1)
    }

    pub fn finished(&self) -> bool {
        self.steps.iter().all(|s| *s == Step::Done)
    }

    pub fn is_failed(&self) -> bool {
        self.steps.contains(&Step::Failed)
    }
}

pub enum Obs<H> {
    Tick { now_ms: u64, launcher_alive: bool },
    ClientAppeared(H),
    ClientPlaced { handle: H, slot: usize },
    ClientGone(H),
    SteamWindow { handle: H, visible: bool },
    SpawnFailed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect<H> {
    Minimise(H),
    Focus(H),
}

pub struct Launch<H> {
    state: LaunchState,
    prefix: String,
    started_ms: u64,
    now_ms: u64,
    known: HashSet<H>,
    /// Steam windows that were visible when the launch started: left alone.
    steam_open: HashSet<H>,
    launcher_seen: bool,
    launcher_gone_since: Option<u64>,
    target: Option<H>,
    ended_ms: Option<u64>,
}

fn hotkey(prefix: &str, slot: usize) -> String {
    if (1..=9).contains(&slot) { format!("{prefix}+{slot}").to_uppercase() } else { "—".to_string() }
}

impl<H: Clone + Eq + Hash> Launch<H> {
    pub fn start(
        now_ms: u64,
        prefix: &str,
        clients: impl IntoIterator<Item = H>,
        steam_windows: impl IntoIterator<Item = (H, bool)>,
    ) -> Self {
        let known: HashSet<H> = clients.into_iter().collect();
        let steam: HashMap<H, bool> = steam_windows.into_iter().collect();
        Launch {
            state: LaunchState {
                steps: [Step::Running, Step::Pending, Step::Pending, Step::Pending],
                hotkey: hotkey(prefix, known.len() + 1),
                failed: None,
            },
            prefix: prefix.to_string(),
            started_ms: now_ms,
            now_ms,
            steam_open: steam.into_iter().filter(|(_, v)| *v).map(|(h, _)| h).collect(),
            known,
            launcher_seen: false,
            launcher_gone_since: None,
            target: None,
            ended_ms: None,
        }
    }

    pub fn state(&self) -> &LaunchState {
        &self.state
    }

    /// The new client this launch is waiting to see placed, once it appeared.
    pub fn target(&self) -> Option<&H> {
        self.target.as_ref()
    }

    fn active(&self) -> bool {
        !self.state.finished() && !self.state.is_failed()
    }

    fn end(&mut self) {
        self.ended_ms.get_or_insert(self.now_ms);
    }

    fn fail(&mut self, step: usize, why: &str) {
        self.state.steps[step] = Step::Failed;
        self.state.failed = Some(why.to_string());
        self.end();
    }

    /// Steam handed EVE on: steps 1 and 2 are done, 3 runs.
    fn steam_done(&mut self) {
        if self.state.steps[0] == Step::Running {
            self.state.steps[0] = Step::Done;
            self.state.steps[1] = Step::Done;
            self.state.steps[2] = Step::Running;
        }
    }

    pub fn observe(&mut self, obs: Obs<H>) -> Vec<Effect<H>> {
        if !self.active() {
            if let Obs::Tick { now_ms, .. } = obs {
                self.now_ms = now_ms;
            }
            return Vec::new();
        }
        match obs {
            Obs::Tick { now_ms, launcher_alive } => {
                self.now_ms = now_ms;
                if launcher_alive {
                    self.launcher_seen = true;
                    self.launcher_gone_since = None;
                    self.steam_done();
                } else if self.launcher_seen && self.state.steps[2] == Step::Running {
                    let since = *self.launcher_gone_since.get_or_insert(now_ms);
                    if now_ms.saturating_sub(since) >= LAUNCHER_GONE_MS {
                        self.fail(2, "LAUNCHER CLOSED");
                    }
                }
                if self.state.steps[0] == Step::Running && now_ms.saturating_sub(self.started_ms) >= STEAM_TIMEOUT_MS {
                    self.fail(0, "STEAM DID NOT START EVE");
                }
                Vec::new()
            }
            Obs::SpawnFailed(why) => {
                self.fail(0, &format!("CANNOT RUN STEAM: {}", why.to_uppercase()));
                Vec::new()
            }
            Obs::ClientAppeared(h) => {
                if self.target.is_none() && !self.known.contains(&h) {
                    self.steam_done();
                    self.state.steps[2] = Step::Done;
                    self.state.steps[3] = Step::Running;
                    self.target = Some(h);
                }
                Vec::new()
            }
            Obs::ClientPlaced { handle, slot } => {
                if self.target.as_ref() == Some(&handle) && self.state.steps[3] == Step::Running {
                    self.state.steps[3] = Step::Done;
                    self.state.hotkey = hotkey(&self.prefix, slot);
                    self.end();
                    return vec![Effect::Focus(handle)];
                }
                Vec::new()
            }
            Obs::ClientGone(h) => {
                if self.target.as_ref() == Some(&h) {
                    self.fail(3, "CLIENT CLOSED");
                }
                Vec::new()
            }
            Obs::SteamWindow { handle, visible } => {
                if visible && !self.steam_open.contains(&handle) { vec![Effect::Minimise(handle)] } else { Vec::new() }
            }
        }
    }

    /// The finished state has been shown for 2 s, or a failure for 5 s.
    pub fn expired(&self, now_ms: u64) -> bool {
        let Some(ended) = self.ended_ms else { return false };
        let linger = if self.state.is_failed() { FAILED_LINGER_MS } else { DONE_LINGER_MS };
        now_ms.saturating_sub(ended) >= linger
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use Step::*;

    fn tick(now_ms: u64, launcher_alive: bool) -> Obs<u32> {
        Obs::Tick { now_ms, launcher_alive }
    }

    /// Clients 1 and 2 exist; Steam window 50 is open (visible), 51 minimised.
    fn started() -> Launch<u32> {
        Launch::start(0, "Ctrl+Alt", [1, 2], [(50, true), (51, false)])
    }

    #[test]
    fn a_launch_starts_on_step_one_with_the_predicted_hotkey() {
        let l = started();
        assert_eq!(l.state().steps, [Running, Pending, Pending, Pending]);
        assert_eq!(l.state().hotkey, "CTRL+ALT+3");
        assert_eq!(l.state().current(), 1);
    }

    #[test]
    fn the_happy_path_runs_all_four_steps_and_focuses_the_new_client() {
        let mut l = started();
        assert!(l.observe(tick(500, false)).is_empty());
        l.observe(tick(1_000, true));
        assert_eq!(l.state().steps, [Done, Done, Running, Pending], "launcher seen: steam did its part");
        l.observe(Obs::ClientAppeared(2)); // an existing client: not ours
        assert_eq!(l.state().steps[2], Running);
        l.observe(Obs::ClientAppeared(7));
        assert_eq!(l.state().steps, [Done, Done, Done, Running]);
        let fx = l.observe(Obs::ClientPlaced { handle: 7, slot: 2 });
        assert_eq!(fx, vec![Effect::Focus(7)]);
        assert_eq!(l.state().steps, [Done, Done, Done, Done]);
        assert_eq!(l.state().hotkey, "CTRL+ALT+2", "the real slot replaces the prediction");
        assert!(l.state().finished());
        assert!(!l.expired(1_500));
    }

    #[test]
    fn a_finished_launch_lingers_two_seconds_and_a_failed_one_five() {
        // The linger runs from the last time the machine saw: 10 000 here.
        let mut l = started();
        l.observe(Obs::ClientAppeared(7));
        l.observe(tick(10_000, false));
        l.observe(Obs::ClientPlaced { handle: 7, slot: 3 });
        assert!(!l.expired(11_999));
        assert!(l.expired(12_000));
        // Failed at the start (0): shown for 5 s.
        let mut f = started();
        f.observe(Obs::SpawnFailed("No such file or directory".into()));
        assert!(f.state().is_failed());
        assert!(!f.expired(4_999));
        assert!(f.expired(5_000));
    }

    #[test]
    fn the_target_is_the_new_client_once_it_appears() {
        let mut l = started();
        assert_eq!(l.target(), None);
        l.observe(Obs::ClientAppeared(2));
        assert_eq!(l.target(), None, "an existing client is not the target");
        l.observe(Obs::ClientAppeared(7));
        assert_eq!(l.target(), Some(&7));
    }

    #[test]
    fn a_client_appearing_first_also_completes_step_one() {
        let mut l = started();
        l.observe(Obs::ClientAppeared(9));
        assert_eq!(l.state().steps, [Done, Done, Done, Running]);
    }

    #[test]
    fn steam_that_never_starts_eve_fails_step_one_after_a_minute() {
        let mut l = started();
        l.observe(tick(59_999, false));
        assert_eq!(l.state().steps[0], Running);
        l.observe(tick(60_000, false));
        assert_eq!(l.state().steps[0], Failed);
        assert_eq!(l.state().failed.as_deref(), Some("STEAM DID NOT START EVE"));
    }

    #[test]
    fn a_closed_launcher_fails_step_three_after_ten_seconds() {
        let mut l = started();
        l.observe(tick(1_000, true));
        l.observe(tick(2_000, false));
        l.observe(tick(11_999, false));
        assert_eq!(l.state().steps[2], Running, "under 10 s: a launcher restarting itself is fine");
        l.observe(tick(12_000, false));
        assert_eq!(l.state().steps[2], Failed);
        assert_eq!(l.state().failed.as_deref(), Some("LAUNCHER CLOSED"));
    }

    #[test]
    fn the_launcher_coming_back_resets_the_ten_seconds() {
        let mut l = started();
        l.observe(tick(1_000, true));
        l.observe(tick(2_000, false));
        l.observe(tick(9_000, true));
        l.observe(tick(15_000, false));
        assert_eq!(l.state().steps[2], Running);
    }

    #[test]
    fn no_timeout_while_the_launcher_lives() {
        let mut l = started();
        l.observe(tick(1_000, true));
        l.observe(tick(3_600_000, true));
        assert_eq!(l.state().steps[2], Running);
    }

    #[test]
    fn the_new_client_closing_before_placement_fails_step_four() {
        let mut l = started();
        l.observe(Obs::ClientAppeared(7));
        l.observe(Obs::ClientGone(7));
        assert_eq!(l.state().steps[3], Failed);
        assert_eq!(l.state().failed.as_deref(), Some("CLIENT CLOSED"));
    }

    #[test]
    fn a_slot_past_nine_has_no_hotkey() {
        let mut l = Launch::start(0, "Ctrl+Alt", 1..=8u32, std::iter::empty());
        assert_eq!(l.state().hotkey, "CTRL+ALT+9");
        l.observe(Obs::ClientAppeared(99));
        l.observe(Obs::ClientPlaced { handle: 99, slot: 10 });
        assert_eq!(l.state().hotkey, "—");
    }

    /// Daniel's choice: only a Steam window that popped up is minimised.
    #[test]
    fn only_a_steam_window_that_popped_up_is_minimised() {
        let mut l = started();
        assert!(l.observe(Obs::SteamWindow { handle: 50, visible: true }).is_empty(), "already open at the start");
        assert_eq!(l.observe(Obs::SteamWindow { handle: 51, visible: true }), vec![Effect::Minimise(51)], "was minimised, popped up");
        assert_eq!(l.observe(Obs::SteamWindow { handle: 52, visible: true }), vec![Effect::Minimise(52)], "new window");
        assert!(l.observe(Obs::SteamWindow { handle: 52, visible: false }).is_empty());
        // Once the launch is over, Steam is the user's again.
        l.observe(Obs::ClientAppeared(7));
        l.observe(Obs::ClientPlaced { handle: 7, slot: 3 });
        assert!(l.observe(Obs::SteamWindow { handle: 51, visible: true }).is_empty());
    }

    #[test]
    fn the_wire_form_round_trips_and_is_optional_in_status() {
        let l = started();
        let json = serde_json::to_string(l.state()).unwrap();
        assert_eq!(serde_json::from_str::<LaunchState>(&json).unwrap(), *l.state());
        let mut v = serde_json::to_value(crate::tunnel::status::Status {
            clients: vec![],
            hidden: false,
            tunnel: Default::default(),
            shortcuts: None,
            outputs: vec![],
            steam: vec![],
            launch: None,
        })
        .unwrap();
        v.as_object_mut().unwrap().remove("launch");
        let s: crate::tunnel::status::Status = serde_json::from_value(v).unwrap();
        assert_eq!(s.launch, None);
    }
}
