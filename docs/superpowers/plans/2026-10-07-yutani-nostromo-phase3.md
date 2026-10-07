# Nostromo popover — Phase 3 (Launch EVE) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "▶ LAUNCH EVE" button in the popover.
- The daemon starts EVE through Steam.
- It minimises Steam's window if that window popped up.
- It waits for the new client, then reports its hotkey and focuses it.
- The popover shows the four steps live.

**Architecture:**
- A pure, generic state machine (`src/launch_eve.rs`, lib) takes observations (ticks, clients appearing/placed/closing, Steam windows) and returns effects (minimise, focus). It owns the wire type `LaunchState` that the status reply carries.
- The daemon spawns `steam`, feeds the machine from backend events and a 500 ms tick, and applies its effects.
- The applet renders the button and a log card from `Status.launch`.

**Tech Stack:** Rust 2024, libcosmic a401af8 (cctk toplevel info/management), serde.

**Spec:** `docs/superpowers/specs/2026-10-07-yutani-nostromo-design.md` § "Phase 3 — Launch EVE" (revised 2026-10-07).

## Global Constraints

- **No `cargo fmt`, ever.** Stage files by path only, never `git add -A` / `git add .`; `design/` is untracked on purpose.
- **Commit messages** end with a blank line, then exactly `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Every task gates on:** `cargo test --lib`, `cargo test --bin yutani-applet` and `cargo test --bin yutani` all pass, and touched files get no new clippy warnings.
- **New wire fields** are `serde(default)`. IPC text for the new request: `launch`.
- **Launch command:** `steam steam://rungameid/8500`.
- **Launcher process name:** `evelauncher.exe` (case-insensitive match on `comm` or argv[0] basename, as `adopt::is_eve_process` does).
- **Steam's window** = a toplevel whose app_id equals `steam`, ignoring case.
- **Timings:**
  - step 1 fails after 60 s;
  - step 3 fails 10 s after the launcher went away with no new client;
  - a finished state stays 2 s;
  - a failed one stays 5 s;
  - the daemon tick runs at 500 ms while a launch is active.
- **Copy, verbatim:**
  - `STEAM · APPLAUNCH 8500`, `STEAM · WINDOW MINIMISED`, `EVE CLIENT · STARTING`, `HOTKEY ASSIGNED · {hotkey}`;
  - buttons `▶ LAUNCH EVE`, `LAUNCHING…`, `STEP n / 4`, `START YUTANI TO LAUNCH`, `ALL 9 SLOTS IN USE`;
  - line statuses `OK`, `…`, `FAIL`.
- **Failure reasons (upper case):** `STEAM DID NOT START EVE`, `LAUNCHER CLOSED`, `CLIENT CLOSED`, `CANNOT RUN STEAM: <error>`.

---

### Task 1: The launch state machine (`src/launch_eve.rs`)

**Files:**
- Create: `src/launch_eve.rs`
- Modify: `src/lib.rs` (`pub mod launch_eve;`), `src/tunnel/status.rs` (`Status.launch`)
- Modify: every `Status { … }` literal that lists all fields (grep `Status {` in src; add `launch: None`)

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] pub enum Step { Pending, Running, Done, Failed }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)] pub struct LaunchState { pub steps: [Step; 4], pub hotkey: String, pub failed: Option<String> }
impl LaunchState { pub fn current(&self) -> u8 /* 1..=4, the first not Done; 4 when all done */; pub fn finished(&self) -> bool; pub fn is_failed(&self) -> bool }
pub enum Obs<H> { Tick { now_ms: u64, launcher_alive: bool }, ClientAppeared(H), ClientPlaced { handle: H, slot: usize }, ClientGone(H), SteamWindow { handle: H, visible: bool }, SpawnFailed(String) }
#[derive(Debug, PartialEq, Eq)] pub enum Effect<H> { Minimise(H), Focus(H) }
pub struct Launch<H> { .. }
impl<H: Clone + Eq + Hash> Launch<H> {
    pub fn start(now_ms: u64, prefix: &str, clients: impl IntoIterator<Item = H>, steam_windows: impl IntoIterator<Item = (H, bool)>) -> Self;
    pub fn observe(&mut self, obs: Obs<H>) -> Vec<Effect<H>>;
    pub fn state(&self) -> &LaunchState;
    pub fn expired(&self, now_ms: u64) -> bool;
}
// in tunnel/status.rs Status: #[serde(default)] pub launch: Option<crate::launch_eve::LaunchState>,
```

- [ ] **Step 1: Write the failing tests** (in `src/launch_eve.rs`):

```rust
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
```

- [ ] **Step 2: Run them.** `cargo test --lib launch_eve` → FAIL.

- [ ] **Step 3: Implement** (above the tests):

```rust
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
```

  In `src/tunnel/status.rs`, add to `Status` after `steam`:

```rust
    /// A Launch EVE in progress or just ended (`launch_eve`); `None`
    /// otherwise and (`serde(default)`) from a daemon older than Phase 3.
    #[serde(default)]
    pub launch: Option<crate::launch_eve::LaunchState>,
```

  Add `launch: None` to every full `Status { … }` literal (find them with `grep -rn "Status {" src | grep -v "TunnelStatus\|OutputStatus\|ClientStatus"`). Add `pub mod launch_eve;` to `src/lib.rs`.

- [ ] **Step 4: Run the tests.** `cargo test --lib`, `cargo test --bin yutani` and `cargo test --bin yutani-applet` → PASS.
- [ ] **Step 5: Commit.** Message: `feat: the Launch EVE state machine and its wire form`, with the trailer.

---

### Task 2: Backend Steam windows, launcher check, IPC `launch`

**Files:**
- Modify: `src/backend/mod.rs` (`Event::SteamWindow`, `Event::SteamWindowGone`), `src/backend/toplevels.rs` (emit them)
- Modify: `src/adopt.rs` (`pub fn process_running(patterns: &[String], uid: u32) -> bool`)
- Modify: `src/ipc.rs` (`Request::Launch`, `launch`)

**Interfaces — Produces:**
- `Event::SteamWindow(Handle, bool /* visible = !minimized */)`
- `Event::SteamWindowGone(Handle)`
- `adopt::process_running(&["evelauncher.exe".into()], uid) -> bool`
- `Request::Launch`

- [ ] **Step 1: Write the failing tests.**
  - `ipc.rs`, following the existing parse/to_line tests: `assert_eq!(Request::parse("launch"), Ok(Request::Launch)); assert_eq!(Request::Launch.to_line(), "launch\n");`. Match the real function names used there.
  - `adopt.rs`: factor the per-process name check out of `scan` into `fn process_name_matches(comm: &str, argv0: &str, patterns: &[String]) -> bool`, and test it: `("evelauncher.exe", "", ["evelauncher.exe"]) → true`; `("wineserver", r"C:\EVE\Launcher\evelauncher.exe", …) → true`; `("bash", "/usr/bin/bash", …) → false`.
  - `backend`: add a pure helper `pub fn is_steam_window(app_id: &str) -> bool` (`app_id.eq_ignore_ascii_case("steam")`) with a test (`"steam"`, `"Steam"` → true; `"steam_app_8500"`, `""` → false).
- [ ] **Step 2: Run them** → FAIL.
- [ ] **Step 3: Implement.**
  - **IPC:** `Request::Launch` with parse `("launch", None) => Ok(Request::Launch)` and `to_line` `"launch\n"`.
  - **`process_running(patterns, uid) -> bool`:** walks `/proc` like `scan`, but stops at the first process owned by `uid` whose name matches. No cgroup filter: the launcher counts wherever it runs. Reuse `process_name_matches`, and read argv[0] only when `comm` does not match, as `scan` does.
  - **Backend:**
    - `new_toplevel` / `update_toplevel`: when `client_info` is `None` and `is_steam_window(&info.app_id)`, send `Event::SteamWindow(handle.clone(), !info.state.contains(&State::Minimized))`.
    - `toplevel_closed`: if the closed toplevel was a Steam window, send `Event::SteamWindowGone`. Track Steam handles in a `HashSet<Handle>` on `AppData` (field `steam_windows`), because `info()` is gone by then.
    - Doc comments say why: Launch EVE minimises a Steam window that pops up.
  - **Daemon handlers:** in `src/ui/mod.rs`, for now keep a `steam_windows: HashMap<Handle, bool>` on `App`, updated by the two events. Answer `Request::Launch` with `Reply::Now(Err("launch: not yet".into()))`; Task 3 replaces it. This keeps the match exhaustive and the build green.
- [ ] **Step 4: Run the tests** → PASS.
- [ ] **Step 5: Commit.** Message: `feat(daemon): see Steam's window and the EVE Launcher; IPC launch`, with the trailer.

---

### Task 3: Daemon runs the launch

**Files:**
- Modify: `src/ui/mod.rs`
- Create: `src/launch_steam.rs` (bin module; add `mod launch_steam;` in `src/main.rs`)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: `App.launch: Option<Launch<Handle>>`, `Msg::LaunchTick`, `Msg::LaunchSpawnFailed(String)`; the status reply includes `launch`.

- [ ] **Step 1: Write the failing test** for the spawner (`src/launch_steam.rs`):

```rust
//! Start EVE through Steam for Launch EVE: `steam steam://rungameid/8500`,
//! detached (null stdio, own process group) and reaped on a thread, so a
//! Steam that exits is never a zombie under the daemon and a signal to the
//! daemon's group never reaches it.

use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

pub const URL: &str = "steam://rungameid/8500";

pub fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.arg(URL).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    c
}

/// Spawn `program URL` and reap it in the background; `Err` if it cannot
/// be started at all.
pub fn spawn(program: &str) -> std::io::Result<()> {
    let mut child = command(program).spawn()?;
    std::thread::Builder::new().name("reap-steam".into()).spawn(move || {
        let _ = child.wait();
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_steam_with_the_eve_url() {
        let c = command("steam");
        assert_eq!(c.get_program(), "steam");
        assert_eq!(c.get_args().collect::<Vec<_>>(), [URL]);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        assert!(spawn("/no/such/steam-yutani-test").is_err());
        assert!(spawn("/bin/true").is_ok());
    }
}
```

- [ ] **Step 2: Run it** → FAIL. Then implement it as shown → PASS.

- [ ] **Step 3: Wire `src/ui/mod.rs`.**
  1. **`App` fields:**
     - `launch: Option<yutani::launch_eve::Launch<Handle>>`;
     - `launch_epoch: Instant`, the zero for `now_ms`, set at startup.
  2. **`Request::Launch`:**
     - If a launch is active (Some and not expired): `Reply::Now(Ok(None))`.
     - Else, if `self.clients.len() >= 9`: `Err("all 9 slots in use")`.
     - Otherwise:
       - Set `self.launch = Some(Launch::start(now_ms, &self.config.shortcuts.prefix_label(), self.clients.keys().cloned(), self.steam_windows.iter().map(|(h, v)| (h.clone(), *v))))`.
       - Then `crate::launch_steam::spawn("steam")`. On `Err(e)`, feed `Obs::SpawnFailed(e.to_string())`.
       - Reply `Ok(None)` in both cases. The applet shows the failure through status.
  3. **Effects:** a helper `fn apply_launch(&mut self, effects: Vec<Effect<Handle>>) -> Task<…>`:
     - `Minimise(h)` sends `self.send(Cmd::Minimize(h))`;
     - `Focus(h)` calls `let _ = self.activate(h);`.
  4. **Observations:**
     - In `Event::ClientAdded | ClientUpdated`: if the handle was **not** in `self.clients` before the `entry().or_insert_with`, feed `Obs::ClientAppeared(handle.clone())`. Compute `let is_new = !self.clients.contains_key(&handle);` before the insert.
     - In `Event::ClientRemoved`: `Obs::ClientGone(handle)`.
     - In `Event::SteamWindow(h, visible)`: update `steam_windows` and feed `Obs::SteamWindow { handle: h, visible }`.
     - In `SteamWindowGone`: remove it from `steam_windows`.
  5. **`Msg::LaunchTick`:**
     - `now_ms` comes from `launch_epoch`.
     - `launcher_alive` comes from `crate::adopt::process_running(&["evelauncher.exe".to_string()], uid)`. The `/proc` walk is a few ms, at 2 Hz, only while launching. If that ever shows in a profile, move it to `spawn_blocking`.
     - Feed `Obs::Tick`.
     - If the launch has a target whose client is `placed`, feed `Obs::ClientPlaced { handle, slot }` with `slot = self.focus_order().iter().position(|x| x == &target)? + 1`. Expose the target with `pub fn target(&self) -> Option<&H>` on `Launch`, added in this task with a one-line test in `launch_eve.rs`.
     - Apply the effects.
     - If `expired(now_ms)`, set `self.launch = None`.
  6. **Subscription:** while `self.launch.is_some()`, push `cosmic::iced::time::every(Duration::from_millis(500)).map(|_| Msg::LaunchTick)`.
  7. **Status:** the `Request::Status` handler adds `launch: self.launch.as_ref().map(|l| l.state().clone())` to the `Status` it builds.
- [ ] **Step 4: Run the tests.** `cargo test --bin yutani`, `cargo test --lib` and `cargo build --release` → PASS / ok.
- [ ] **Step 5: Commit.** Message: `feat(daemon): Launch EVE through Steam, tracked to the new client's hotkey`, with the trailer.

---

### Task 4: The applet's button and launch log

**Files:**
- Modify: `src/applet/mod.rs` (`Action::Launch` → `Request::Launch`)
- Modify: `src/applet/console.rs` (`LaunchButton` from status; `launch_log`; height)
- Modify: `src/bin/yutani-applet/console_view.rs` (render the log card)
- Modify: `src/bin/yutani-applet/app.rs` (`Msg::Launch` → `Press(Action::Launch)`)

**Interfaces — Produces:**
- `Console.launch_log: Option<Vec<LogLine>>`, with `pub struct LogLine { pub text: String, pub status: Step }`
- `Console.launch_failed: Option<String>`

- [ ] **Step 1: Write the failing tests** (`console.rs`):

```rust
    fn launching(steps: [Step; 4], failed: Option<&str>) -> Status {
        let mut s = status(true, Some(4), 2);
        s.launch = Some(LaunchState { steps, hotkey: "CTRL+ALT+3".into(), failed: failed.map(Into::into) });
        s
    }

    #[test]
    fn the_button_follows_the_service_the_slots_and_the_launch() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let i = inputs(&h, &p);
        assert_eq!(console(None, &i).launch, LaunchButton::Inert("START YUTANI TO LAUNCH"));
        assert_eq!(console(Some(&status(true, Some(4), 2)), &i).launch, LaunchButton::Ready);
        assert_eq!(console(Some(&status(true, Some(4), 9)), &i).launch, LaunchButton::Inert("ALL 9 SLOTS IN USE"));
        use Step::*;
        let c = console(Some(&launching([Done, Done, Running, Pending], None)), &i);
        assert_eq!(c.launch, LaunchButton::Launching { step: 3 });
        // Finished or failed: the button is ready again while the log lingers.
        assert_eq!(console(Some(&launching([Done; 4], None)), &i).launch, LaunchButton::Ready);
        assert_eq!(console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &i).launch, LaunchButton::Ready);
    }

    #[test]
    fn the_log_names_each_step_and_the_hotkey() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        use Step::*;
        let c = console(Some(&launching([Done, Done, Running, Pending], None)), &inputs(&h, &p));
        let log = c.launch_log.expect("log while launching");
        let texts: Vec<&str> = log.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["STEAM · APPLAUNCH 8500", "STEAM · WINDOW MINIMISED", "EVE CLIENT · STARTING", "HOTKEY ASSIGNED · CTRL+ALT+3"]);
        assert_eq!(log[2].status, Running);
        assert!(console(Some(&status(true, Some(4), 2)), &inputs(&h, &p)).launch_log.is_none());
        let f = console(Some(&launching([Done, Done, Failed, Pending], Some("LAUNCHER CLOSED"))), &inputs(&h, &p));
        assert_eq!(f.launch_failed.as_deref(), Some("LAUNCHER CLOSED"));
    }

    #[test]
    fn the_log_card_counts_in_the_height() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        use Step::*;
        let plain = console(Some(&status(true, Some(4), 2)), &inputs(&h, &p));
        let busy = console(Some(&launching([Done, Done, Running, Pending], None)), &inputs(&h, &p));
        assert_eq!(console_height(&busy, false) - console_height(&plain, false), LOG_CARD_PX);
    }
```

  Replace the Phase 1 assertion `assert_eq!(c.launch, LaunchButton::Hidden, "Phase 1")` in `running_and_connected_fills_every_section` with `assert_eq!(c.launch, LaunchButton::Ready)`. The `Hidden` variant goes away.
- [ ] **Step 2: Run them** → FAIL.
- [ ] **Step 3: Implement.**
  - **`Action::Launch`:** add it with `request()` → `Some(Request::Launch)`. Update `actions_map_onto_the_ipc_protocol`. It has no `waiting_note`.
  - **`console.rs`:**
    - Remove `LaunchButton::Hidden`.
    - Compute the button as the tests require: stopped, then 9 clients, then active launch (`!finished && !is_failed` → `Launching { step: current() }`), then Ready.
    - `launch_log`: `Some` whenever `status.launch` is `Some`. The four texts come from the copy constants, with the hotkey filled in from `LaunchState.hotkey`.
    - `launch_failed`: `failed.clone()`.
    - `pub const LOG_CARD_PX: i32 = 7 + 4 * 13 + 3 * 3 + 8 + 2 + 10;` This is the card's padding, 4 lines, gaps, border and margin. Add 13 more when `launch_failed` is `Some`, and add it in `console_height`.
  - **`console_view.rs`:**
    - `LaunchButton::Hidden` disappears from the match.
    - Add a `launch_log` card after the action row, using `skin::log_class()`, `skin::LOG`, padding `skin::pad(7.0, 12.0, 8.0, 12.0)` and spacing 3.
    - Each line is a `Row`: `t("▸ " + text, LOG, ink)` · `fill_x()` · `t(status, LOG, ink)`.
      - Done: phosphor, `OK`.
      - Running: amber, `…`.
      - Pending: dimmer, empty.
      - Failed: red, `FAIL`.
    - Then, if `launch_failed` is set, one red line with the reason.
    - `▸` must be in B612 Mono. Check it with `fonts::advance_em`'s raw coverage, as the Task-1 tests did; if it is missing, draw it as an `Arrow` Right — add `Direction::Right` to widgets.
  - **`app.rs`:** `Msg::Launch => self.update(Msg::Press(Action::Launch))`. The existing press path sends the request, and an `err` reply becomes the red note.
- [ ] **Step 4: Run the tests.** Run all three test suites and `cargo build --release`.
- [ ] **Step 5: Commit.** Message: `feat(applet): Launch EVE button and the four-step log`, with the trailer.

---

### Task 5: Install and test with Daniel (controller)

- [ ] Build and install the package from the branch, as before: set the PKGBUILD's `#branch=` temporarily, use an absolute path with `pkexec`, then restore the PKGBUILD.
- [ ] Restart the daemon and the panel. The tunnel worker is unchanged, so it does not need a restart.
- [ ] Daniel presses ▶ LAUNCH EVE with Steam open, then with Steam minimised. In both cases:
  - he clicks Play in the launcher;
  - the log advances to `HOTKEY ASSIGNED · CTRL+ALT+n` and the new client is focused;
  - Steam is left alone if it was open, and minimised if it popped up.
- [ ] Failure path: press, then close the launcher. About 10 s later the log shows `LAUNCHER CLOSED` in red, and it clears 5 s after that.
- [ ] Take a screenshot of the log card while it is mid-launch.
