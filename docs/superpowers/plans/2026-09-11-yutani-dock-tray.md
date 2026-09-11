# Yutani Dock Mode, Tray & Polish — Implementation Plan (plan 3 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the second layout mode (a per-output dock strip), a tray icon (Show/Hide/Quit), pause capture for hidden clients, and land the polish items deferred by the plan-2 review (click-without-blink, tested pure rules, fewer redundant resizes).

**Architecture:** No new protocols. Dock mode is one layer surface per output anchored to a configurable edge, rendering the same thumbnail widget in a row/column with widget-level hit-testing (`mouse_area`); floating mode keeps the per-surface pointer path. The tray is a `ksni` StatusNotifierItem on its own thread feeding an iced subscription. Capture pause/resume is a backend `Cmd` pair honoured by `ScreencopySession::submit`.

**Tech Stack:** as plan 2, plus `ksni = { version = "0.3", default-features = false, features = ["blocking"] }` (shares `zbus` 5 with libcosmic).

**Spec:** `docs/superpowers/specs/2026-09-11-yutani-design.md` §5 (pause), §6 (dock, tray). **Deviations decided here:** dock hover zoom grows the hovered thumbnail in place (the strip thickens); no drag/pin in dock mode (spec agrees); tray icon uses a stock icon name until packaging (plan 5) installs our SVG.

**Prior state:** plans 1–2 merged on `master` (38 tests). Read `src/ui/mod.rs`, `src/ui/pointer.rs`, `src/ui/thumbnail.rs`, `src/backend/{mod,capture,toplevels}.rs`, `src/model/{config,layout}.rs` before starting a task that touches them.

## Global Constraints

- All Wayland types via `cosmic::cctk::*`; libcosmic pinned at rev `a401af8b1c54a8abd393b8c5b7c8809402f83850`. The Wayland event subscription forwards only handled variants (`Output`, `Layer`, `Mouse`), never `RequestResize`/`Frame`.
- Floating layer surfaces: Overlay, top-left anchor, exclusive zone 0, keyboard None, namespace `yutani`. Dock surfaces: Overlay, anchored to `dock_edge` **and** both perpendicular edges (stretch), exclusive zone 0, keyboard None, namespace `yutani-dock`.
- Interaction on a thumbnail (both modes): left click → `Cmd::Activate`; right click → `Cmd::Minimize`; hover → zoom by `zoom_factor`. Floating only: drag (> 4 px) moves; middle click pins. Yutani sends no input to EVE.
- A drag must never move the surface under the pointer (plan-2 canvas design). New in this plan: the canvas is entered only after motion exceeds `DRAG_THRESHOLD` in surface-local coordinates, so a plain click never resizes the surface.
- Hidden clients (visibility rules, tray Hide, dock mode's non-shown) have capture **paused**, not destroyed (spec §5); resume on show.
- Config validated on load; invalid → warn + field default. Commit prefix `feat:`/`fix:`/`test:`/`chore:`. Work on a branch off `master`.

## File Structure

| Path | Responsibility |
| --- | --- |
| `src/ui/rules.rs` (new) | pure: `should_show(...)`, `choose_position(...)`, `dock_order(...)` + tests |
| `src/ui/pointer.rs` (modify) | `Phase::Idle` before `Arming`; `Outcome::StartDrag` |
| `src/ui/mod.rs` (modify) | uses `rules`; arm-after-threshold; dock surfaces; tray/pause wiring; `hidden` flag |
| `src/ui/thumbnail.rs` (modify) | `view` gains optional interaction callbacks for dock mode (`mouse_area`) |
| `src/ui/dock.rs` (new) | dock surface settings, layout of the strip, `DockMsg` |
| `src/ui/tray.rs` (new) | `ksni` tray + subscription → `TrayEvent` |
| `src/backend/mod.rs`, `src/backend/capture.rs` (modify) | `Cmd::PauseCapture/ResumeCapture`; stop session on allocation failure |
| `src/model/config.rs` (modify) | `mode: Mode`, `dock_edge: Edge` |
| `src/model/layout.rs` (modify) | doc comment on grid-then-edge; multi-neighbour test |
| `Cargo.toml` (modify) | `ksni` |

---

### Task 1: Pure rules module and layout test/doc polish

**Files:**
- Create: `src/ui/rules.rs`
- Modify: `src/ui/mod.rs` (use the pure fns), `src/model/layout.rs`, `src/model/config.rs` (add `Mode`, `Edge`, fields)

**Interfaces:**
- Produces: `config::Mode { Floating, Dock }`, `config::Edge { Top, Bottom, Left, Right }`, `Config { mode: Mode (default Floating), dock_edge: Edge (default Bottom), … }`; `rules::should_show(visibility: Visibility, hide_active: bool, hidden: bool, any_activated: bool, this_activated: bool) -> bool`; `rules::choose_position(placed: Option<((i32,i32), bool)>, saved: Option<&ThumbPos>, next: (i32,i32)) -> ((i32,i32), bool)`; `rules::dock_order<'a>(labels: impl Iterator<Item=(&'a Handle, &'a str)>) -> Vec<Handle>` (sorted by label, case-insensitive, stable).

- [ ] **Step 1: Write the failing tests**

Create `src/ui/rules.rs`:

```rust
//! Pure decision rules for the UI, kept free of iced/Wayland types so they
//! can be unit-tested.

use crate::backend::Handle;
use crate::model::config::Visibility;
use crate::model::layout::ThumbPos;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_shows_unless_hidden_or_hide_active() {
        assert!(should_show(Visibility::Always, false, false, false, false));
        assert!(should_show(Visibility::Always, false, false, true, true));
        assert!(!should_show(Visibility::Always, true, false, true, true)); // hide_active + this is active
        assert!(should_show(Visibility::Always, true, false, true, false));
        assert!(!should_show(Visibility::Always, false, true, true, false)); // tray hidden
    }

    #[test]
    fn eve_focused_only_needs_some_activated_client() {
        assert!(!should_show(Visibility::EveFocusedOnly, false, false, false, false));
        assert!(should_show(Visibility::EveFocusedOnly, false, false, true, false));
        assert!(!should_show(Visibility::EveFocusedOnly, true, false, true, true));
    }

    #[test]
    fn placement_prefers_placed_then_saved_then_next() {
        let saved = ThumbPos { output: "DP-1".into(), x: 5, y: 6, pinned: true };
        assert_eq!(choose_position(Some(((1, 2), false)), Some(&saved), (9, 9)), ((1, 2), false));
        assert_eq!(choose_position(None, Some(&saved), (9, 9)), ((5, 6), true));
        assert_eq!(choose_position(None, None, (9, 9)), ((9, 9), false));
    }
}
```

Append to `src/model/layout.rs` tests:

```rust
    #[test]
    fn edge_snap_picks_the_nearest_of_several_neighbours() {
        let a = r(200, 40);  // right edge 300
        let b = r(400, 40);  // left edge 400
        // our left edge (305) is 5 from a's right, our right edge (405) is 5 from b's left → tie → first wins (a)
        assert_eq!(snap(r(305, 40), &[a, b], None, Some(12)), (300, 40));
        // clearly nearer to b
        assert_eq!(snap(r(296, 300), &[a, b], None, Some(12)), (300, 300));
        assert_eq!(snap(r(309, 300), &[b, a], None, Some(12)), (300, 300));
    }
```

(`r(309,300)`: left edge 309 vs a.right 300 → d=9; right edge 409 vs b.left 400 → d=9; aligned-left with b (400) → 91. Tie again → first candidate in iteration order over `others` = b: candidates for b are 500 (d=191), 300 (b.x − w = 300, d=9), 400 (d=91) → best 300; then a: 300 (d=9, not < 9) → stays 300. Result 300 either way. Fine.)

Append to `src/model/config.rs` tests:

```rust
    #[test]
    fn plan3_defaults_and_validation() {
        let c = Config::default();
        assert_eq!(c.mode, Mode::Floating);
        assert_eq!(c.dock_edge, Edge::Bottom);
        let text = "(mode: Dock, dock_edge: Left)";
        let c: Config = ron::from_str(text).unwrap();
        assert_eq!(c.mode, Mode::Dock);
        assert_eq!(c.dock_edge, Edge::Left);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd ~/Yutani && cargo test rules 2>&1 | grep -E '^error' | head -3; cargo test plan3_defaults 2>&1 | grep -E '^error' | head -2`
Expected: `should_show`/`choose_position` not found; `Mode`/`Edge` not found. (Add `pub mod rules;` to `src/ui/mod.rs` first.)

- [ ] **Step 3: Implement**

`src/model/config.rs` — add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Floating,
    Dock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}
```

fields `pub mode: Mode,` and `pub dock_edge: Edge,` (after `snap_edges`), defaults `mode: Mode::Floating, dock_edge: Edge::Bottom`. No validation needed (enums).

`src/ui/rules.rs` — insert above the tests:

```rust
/// Whether a client's thumbnail should be on screen right now.
pub fn should_show(
    visibility: Visibility,
    hide_active: bool,
    hidden: bool,
    any_activated: bool,
    this_activated: bool,
) -> bool {
    if hidden {
        return false;
    }
    let visible = match visibility {
        Visibility::Always => true,
        Visibility::EveFocusedOnly => any_activated,
    };
    visible && !(hide_active && this_activated)
}

/// Where a (re)created floating thumbnail goes: where it already was, else
/// the character's saved spot, else the next free slot.
pub fn choose_position(
    placed: Option<((i32, i32), bool)>,
    saved: Option<&ThumbPos>,
    next: (i32, i32),
) -> ((i32, i32), bool) {
    if let Some(p) = placed {
        return p;
    }
    if let Some(s) = saved {
        return ((s.x, s.y), s.pinned);
    }
    (next, false)
}

/// Dock order: by label, case-insensitive, stable for equal labels.
pub fn dock_order<'a>(labels: impl Iterator<Item = (&'a Handle, &'a str)>) -> Vec<Handle> {
    let mut v: Vec<(&Handle, String)> = labels.map(|(h, l)| (h, l.to_lowercase())).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v.into_iter().map(|(h, _)| h.clone()).collect()
}
```

`src/model/layout.rs` — add to the `snap` doc comment: "Edge distances are measured from the grid-snapped coordinate (when grid snapping is on), so a thumbnail that lands on a grid line near a neighbour still snaps flush; the effective edge threshold is therefore up to `grid/2 + edge_threshold`."

`src/ui/mod.rs` — replace the bodies of `should_show` and the placement `if/else` in `create_surface` with calls to `rules::should_show(self.config.visibility, self.config.hide_active, self.hidden, self.any_client_activated(), client.info.activated)` and `rules::choose_position(client.placed.then_some((client.position, client.pinned)), saved.as_ref(), self.next_position())`. Add `pub hidden: bool` to `App` (init `false`; used by the tray in Task 3).

- [ ] **Step 4: Run tests**

Run: `cd ~/Yutani && cargo test 2>&1 | grep 'test result'` → 43 passed (38 + 3 rules + 1 layout + 1 config).

- [ ] **Step 5: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: pure UI rules module with tests; Mode/Edge config; layout snap docs and multi-neighbour test"
```

---

### Task 2: Interaction polish — arm canvas after threshold; resize only when changed; stop session on allocation failure

**Files:**
- Modify: `src/ui/pointer.rs`, `src/ui/mod.rs`, `src/backend/capture.rs`

**Interfaces:**
- `pointer::Phase { Idle, Arming, Dragging }`; `Outcome::StartDrag` (returned once when local motion first exceeds the threshold while `Idle`); `on_press` now stores `press_local: Point` and starts in `Idle`; `on_move_local(drag, cursor_local: Point) -> Outcome` for `Idle`; `on_move` (absolute) unchanged for `Dragging`.
- `Client.last_size: Option<(u32, u32)>` = last size sent via `set_size`/creation; `App::resize_if_needed(&Handle) -> Task<Msg>`.

- [ ] **Step 1: Tests first (`src/ui/pointer.rs`)**

Replace the pointer tests with:

```rust
    #[test]
    fn press_left_or_right_starts_idle_drag_middle_does_not() {
        assert!(matches!(on_press(sid(), Button::Left, Point::new(5.0, 5.0), (40, 40), false), Some(DragState { phase: Phase::Idle, .. })));
        assert!(on_press(sid(), Button::Right, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Middle, Point::new(5.0, 5.0), (40, 40), false).is_none());
    }

    #[test]
    fn small_local_motion_stays_idle_and_release_is_a_click() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        assert_eq!(on_move_local(&mut d, Point::new(12.0, 11.0)), Outcome::None);
        assert_eq!(d.phase, Phase::Idle);
        assert_eq!(on_release(d), Outcome::Click(Button::Left));
    }

    #[test]
    fn local_motion_beyond_threshold_starts_the_drag_then_arms() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        assert_eq!(on_move_local(&mut d, Point::new(16.0, 10.0)), Outcome::StartDrag);
        assert_eq!(d.phase, Phase::Arming);
        assert!(d.moved);
        // absolute motion is ignored until armed
        assert_eq!(on_move(&mut d, Point::new(70.0, 55.0)), Outcome::None);
        on_armed(&mut d, (2560, 1440));
        // press_abs = (50,50); abs (70,55) → start + (20,5)
        assert_eq!(on_move(&mut d, Point::new(70.0, 55.0)), Outcome::Move((60, 45)));
        assert_eq!(on_release(d), Outcome::DragEnd);
    }

    #[test]
    fn pinned_never_starts_a_drag_but_clicks() {
        let mut d = on_press(sid(), Button::Right, Point::new(10.0, 10.0), (40, 40), true).unwrap();
        assert_eq!(on_move_local(&mut d, Point::new(90.0, 90.0)), Outcome::None);
        assert_eq!(on_release(d), Outcome::Click(Button::Right));
    }
```

- [ ] **Step 2: Implement in `pointer.rs`**

- `Phase` gains `Idle` (initial). `DragState` gains `press_local: Point`.
- `on_press`: `phase: Phase::Idle`, `press_local: cursor_local`, `press_abs` as before.
- New `pub fn on_move_local(drag: &mut DragState, cursor_local: Point) -> Outcome`: if `drag.pinned || drag.phase != Phase::Idle` → `None`; if distance from `press_local` ≥ `DRAG_THRESHOLD` → `drag.moved = true; drag.phase = Phase::Arming; Outcome::StartDrag` else `None`.
- `on_move` (absolute): returns `None` unless `phase == Dragging` (threshold check no longer needed there since `moved` is already true; keep computing `Move(start_pos + (abs − press_abs))`).
- `on_armed` unchanged (Arming → Dragging).

- [ ] **Step 3: Wire in `src/ui/mod.rs`**

- `ButtonPressed(Left|Right)`: create the `DragState`; **do not** enter the canvas; return `Task::none()`.
- `CursorMoved`: if drag for this surface: if `phase == Idle` → `match on_move_local(drag, position)`: `StartDrag` → return `self.enter_canvas(handle, id)` (the existing batch: anchor all, margin 0, size None/None; also `hovered` untouched); else `None`. If `phase == Dragging` → existing absolute path. If `Arming` → ignore.
- `ButtonReleased`: if the drag never left `Idle` (`!drag.moved`), just handle the click — no restore batch (surface never changed). Otherwise as now.
- `view_window`: canvas branch only when `phase != Idle`.
- `in_canvas(id)`: true only when the drag for `id` has `phase != Idle`.
- **Resize dedupe:** add `pub last_size: Option<(u32, u32)>` to `Client`; set it in `create_surface` (initial size) and wherever `set_size` is sent for a client surface (hover enter/leave, Frame, leave_canvas, apply_config). Add:

```rust
    fn resize_if_needed(&mut self, handle: &Handle) -> Task<Msg> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Some(id) = client.surface else { return Task::none() };
        if self.in_canvas(id) {
            return Task::none();
        }
        let size = self.surface_size(client);
        if client.last_size == Some(size) {
            return Task::none();
        }
        self.clients.get_mut(handle).unwrap().last_size = Some(size);
        set_size(id, Some(size.0), Some(size.1))
    }
```

  and use it in `apply_config`'s loop, the `Frame` arm, and hover enter/leave (replacing the direct `set_size` calls; `leave_canvas` sets `last_size` to the size it applies).

- [ ] **Step 4: Backend: stop the session on allocation failure (`src/backend/capture.rs`)**

In `init_done` and `failed(BufferConstraints)`, where `allocate` returns `None`, call `capture.stop()` (after dropping any guard) before sending `CaptureUnavailable`, so the next `start_capture` (on the client's next state change) creates a fresh session instead of no-op-ing on a session with `buffers = None`.

- [ ] **Step 5: Build, test, smoke**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head; cargo test 2>&1 | grep 'test result'` → 43 passed (4 pointer tests replace 5).
Log-only smoke as in plan 2 (`create_surface`, no panics).

- [ ] **Step 6: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: enter drag canvas only after threshold motion; dedupe surface resizes; stop capture session on allocation failure"
```

---

### Task 3: Capture pause/resume for hidden clients, and the tray icon

**Files:**
- Modify: `src/backend/mod.rs`, `src/backend/capture.rs`, `src/ui/mod.rs`, `Cargo.toml`
- Create: `src/ui/tray.rs`

**Interfaces:**
- `Cmd::PauseCapture(Handle)`, `Cmd::ResumeCapture(Handle)`; `Capture.paused: AtomicBool`; `ScreencopySession::submit` returns early when paused; resume submits if `!in_flight`.
- `tray::TrayEvent { ToggleVisibility, Quit }`; `tray::subscription() -> Subscription<TrayEvent>`; `Msg::Tray(TrayEvent)`; `App.hidden: bool` (from Task 1) toggled by the tray; `App.tray: Option<ksni::blocking::Handle<YutaniTray>>` is **not** stored in `App` (the handle lives in the subscription stream); the tray's Show/Hide label is updated by sending the current `hidden` state through a `watch`-style `Arc<AtomicBool>` shared with the tray struct.

- [ ] **Step 1: Backend pause/resume**

`src/backend/capture.rs`: add `pub paused: std::sync::atomic::AtomicBool` to `Capture` (init false). In `submit`: `if capture.paused.load(Ordering::Relaxed) { return; }` (submit already takes `capture: &Arc<Capture>`). Add:

```rust
impl AppData {
    pub fn set_paused(&mut self, handle: &Handle, paused: bool, conn: &Connection) {
        let Some(capture) = self.captures.get(handle).cloned() else { return };
        capture.paused.store(paused, Ordering::Relaxed);
        if !paused {
            let mut guard = capture.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                state.submit(&capture, conn, &self.qh);
            }
        }
    }
}
```

`src/backend/mod.rs`: `Cmd::PauseCapture(Handle)`, `Cmd::ResumeCapture(Handle)`; in `handle_cmd` call `self.set_paused(&h, true/false, &conn)` — `handle_cmd` needs the `Connection`: thread the `conn` clone captured in `start` into the calloop callback (`insert_source(cmd_channel, move |event, _, app_data| … app_data.handle_cmd(cmd, &conn))`).

- [ ] **Step 2: UI pause/resume wiring (`src/ui/mod.rs`)**

In `reconcile_surfaces`, when a surface is destroyed because `!show` → `self.send(Cmd::PauseCapture(h.clone()))`; when created because `show && !has` → `self.send(Cmd::ResumeCapture(h.clone()))`. (A client that is shown from the start is never paused; `ResumeCapture` on a non-paused capture is a harmless no-op submit guarded by `in_flight`.)

- [ ] **Step 3: Tray (`Cargo.toml` + `src/ui/tray.rs`)**

`Cargo.toml`: `ksni = { version = "0.3", default-features = false, features = ["blocking"] }`.

```rust
//! StatusNotifierItem tray icon (COSMIC's status-area applet shows it).

use cosmic::iced::futures::{StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use ksni::blocking::TrayMethods;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug)]
pub enum TrayEvent {
    ToggleVisibility,
    Quit,
}

pub struct YutaniTray {
    tx: mpsc::UnboundedSender<TrayEvent>,
    hidden: Arc<AtomicBool>,
}

impl ksni::Tray for YutaniTray {
    fn id(&self) -> String {
        "yutani".into()
    }
    fn title(&self) -> String {
        "Yutani".into()
    }
    fn icon_name(&self) -> String {
        // Stock icon until packaging installs our own (plan 5).
        "video-display-symbolic".into()
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.unbounded_send(TrayEvent::ToggleVisibility);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let label = if self.hidden.load(Ordering::Relaxed) { "Show thumbnails" } else { "Hide thumbnails" };
        vec![
            StandardItem {
                label: label.into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.unbounded_send(TrayEvent::ToggleVisibility);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.unbounded_send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Shared flag the UI flips so the menu label follows the real state.
pub type HiddenFlag = Arc<AtomicBool>;

pub fn subscription(hidden: HiddenFlag) -> Subscription<TrayEvent> {
    #[derive(Clone)]
    struct Key(HiddenFlag);
    impl std::hash::Hash for Key {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            "yutani-tray".hash(state);
        }
    }
    Subscription::run_with(Key(hidden), |Key(hidden)| {
        let (tx, rx) = mpsc::unbounded::<TrayEvent>();
        let tray = YutaniTray { tx, hidden: hidden.clone() };
        let handle = match tray.spawn() {
            Ok(h) => Some(h),
            Err(e) => {
                tracing::warn!("tray unavailable: {e}");
                None
            }
        };
        // Keep the ksni handle alive for as long as the stream lives.
        iced::futures::stream::unfold((rx, handle), |(mut rx, handle)| async move {
            let ev = rx.next().await?;
            Some((ev, (rx, handle)))
        })
    })
}
```

(If `Subscription::run_with` insists on a `fn` pointer rather than a closure — it did for the backend — use the same `fn` shape as `backend::subscription`.)

- [ ] **Step 4: Wire the tray into `src/ui/mod.rs`**

- `App` gains `hidden_flag: tray::HiddenFlag` (init `Arc::new(AtomicBool::new(false))`); `subscription()` pushes `tray::subscription(self.hidden_flag.clone()).map(Msg::Tray)`.
- `Msg::Tray(TrayEvent::ToggleVisibility)` → `self.hidden = !self.hidden; self.hidden_flag.store(self.hidden, Ordering::Relaxed); self.reconcile_surfaces()`.
- `Msg::Tray(TrayEvent::Quit)` → `cosmic::iced::exit()`.
- After the tray menu label changes we want ksni to re-read `menu()`: ksni only re-reads when `Handle::update` is called. Since the handle lives in the stream, instead make the label independent of state: use **two** items, "Show thumbnails" and "Hide thumbnails", both always present (simplest, no update plumbing); remove `hidden` from `YutaniTray` and the `HiddenFlag` type/param if you go this way — **do this**; it is the YAGNI option.

- [ ] **Step 5: Build, test, smoke**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head; cargo test 2>&1 | grep 'test result'` → 43 passed.
Smoke with EVE running: `RUST_LOG=yutani=info timeout 8 ./target/debug/yutani 2>&1 | grep -E 'tray|create_surface|panic|error' | cut -c1-120` — no `tray unavailable` warning; then `busctl --user list | grep -i StatusNotifierItem` should list a `StatusNotifierItem-<pid>-1` name while it runs. `pkill -f target/debug/yutani`.

- [ ] **Step 6: Commit**

```bash
cd ~/Yutani && git add Cargo.toml Cargo.lock src && git commit -m "feat: pause capture for hidden clients; tray icon with Show/Hide/Quit"
```

---

### Task 4: Dock mode

**Files:**
- Create: `src/ui/dock.rs`
- Modify: `src/ui/mod.rs`, `src/ui/thumbnail.rs`

**Interfaces:**
- `dock::DockSurface { output: WlOutput, id: SurfaceId }`; `dock::settings(id, output, edge, thickness) -> SctkLayerSurfaceSettings`; `dock::thickness(config, any_hovered: bool, image_aspect: Option<(u32,u32)>) -> u32` (thumb height incl. border + 2×`DOCK_PADDING` (8), zoomed when hovered); `dock::view<'a>(app: &'a App, output: &WlOutput) -> Element<'a, Msg>`.
- `thumbnail::view` gains a second variant `thumbnail::interactive<'a>(client, config, on: thumbnail::Callbacks) -> Element<'a, Msg>` where `Callbacks { press: Msg, right_press: Msg, enter: Msg, exit: Msg }` wraps the widget in `mouse_area` (dock mode only; floating keeps the raw widget + surface-level pointer path).
- `Msg::Dock(DockMsg)` with `DockMsg { Press(Handle), RightPress(Handle), Enter(Handle), Exit(Handle) }`.
- `App.docks: Vec<DockSurface>`; `App::reconcile_docks() -> Task<Msg>`.

- [ ] **Step 1: Tests for the pure bits (`src/ui/dock.rs`)**

```rust
    #[test]
    fn thickness_is_thumb_height_plus_padding_and_grows_when_hovered() {
        let config = Config { thumb_width: 320, border_px: 2, zoom_factor: 1.5, ..Config::default() };
        assert_eq!(thickness(&config, false, None), 184 + 16);
        assert_eq!(thickness(&config, true, None), 274 + 16);
    }
```

- [ ] **Step 2: Implement `src/ui/dock.rs`**

```rust
//! Dock mode: one strip per output along `dock_edge`, thumbnails in a row
//! (top/bottom) or column (left/right).

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::protocol::wl_output::WlOutput;
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use cosmic::Element;

use super::{App, Msg, thumbnail};
use crate::backend::Handle;
use crate::model::config::{Config, Edge};

pub const DOCK_PADDING: u32 = 8;

#[derive(Clone, Debug)]
pub struct DockSurface {
    pub output: WlOutput,
    pub id: SurfaceId,
}

#[derive(Clone, Debug)]
pub enum DockMsg {
    Press(Handle),
    RightPress(Handle),
    Enter(Handle),
    Exit(Handle),
}

/// Strip thickness (the dimension perpendicular to the edge).
pub fn thickness(config: &Config, any_hovered: bool, image: Option<&crate::backend::CaptureImage>) -> u32 {
    let (_, h) = thumbnail::zoomed_size(config, image, any_hovered);
    h + 2 * DOCK_PADDING
}

pub fn is_horizontal(edge: Edge) -> bool {
    matches!(edge, Edge::Top | Edge::Bottom)
}

pub fn settings(id: SurfaceId, output: WlOutput, edge: Edge, thickness: u32) -> SctkLayerSurfaceSettings {
    let (anchor, size) = match edge {
        Edge::Top => (Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, (None, Some(thickness))),
        Edge::Bottom => (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (None, Some(thickness))),
        Edge::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (Some(thickness), None)),
        Edge::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (Some(thickness), None)),
    };
    SctkLayerSurfaceSettings {
        id,
        layer: Layer::Overlay,
        keyboard_interactivity: KeyboardInteractivity::None,
        anchor,
        output: IcedOutput::Output(output),
        namespace: "yutani-dock".into(),
        size: Some(size),
        exclusive_zone: 0,
        ..Default::default()
    }
}

pub fn view<'a>(app: &'a App, output: &WlOutput) -> Element<'a, Msg> {
    let order = app.dock_order_for(output);
    let items: Vec<Element<'a, Msg>> = order
        .iter()
        .filter_map(|h| app.clients.get(h).map(|c| (h.clone(), c)))
        .map(|(h, c)| {
            let (w, hgt) = thumbnail::zoomed_size(&app.config, c.image.as_ref(), c.hovered);
            let cb = thumbnail::Callbacks {
                press: Msg::Dock(DockMsg::Press(h.clone())),
                right_press: Msg::Dock(DockMsg::RightPress(h.clone())),
                enter: Msg::Dock(DockMsg::Enter(h.clone())),
                exit: Msg::Dock(DockMsg::Exit(h)),
            };
            widget::container(thumbnail::interactive(c, &app.config, cb))
                .width(Length::Fixed(w as f32))
                .height(Length::Fixed(hgt as f32))
                .into()
        })
        .collect();
    let strip: Element<'a, Msg> = if is_horizontal(app.config.dock_edge) {
        widget::row::with_children(items).spacing(DOCK_PADDING as f32).align_y(Alignment::End).into()
    } else {
        widget::column::with_children(items).spacing(DOCK_PADDING as f32).align_x(Alignment::Start).into()
    };
    widget::container(strip)
        .padding(DOCK_PADDING as f32)
        .center(Length::Fill)
        .into()
}
```

(`row::with_children`/`column::with_children` — verify names in `cosmic::widget`; fall back to `cosmic::iced::widget::{Row, Column}::with_children`.)

- [ ] **Step 3: `thumbnail::interactive` (`src/ui/thumbnail.rs`)**

```rust
pub struct Callbacks {
    pub press: Msg,
    pub right_press: Msg,
    pub enter: Msg,
    pub exit: Msg,
}

pub fn interactive<'a>(client: &'a Client, config: &Config, cb: Callbacks) -> Element<'a, Msg> {
    widget::mouse_area(view(client, config))
        .on_press(cb.press)
        .on_right_press(cb.right_press)
        .on_enter(cb.enter)
        .on_exit(cb.exit)
        .into()
}
```

- [ ] **Step 4: Wire dock mode in `src/ui/mod.rs`**

- `pub mod dock;`, `App.docks: Vec<dock::DockSurface>` (init empty), `Msg::Dock(dock::DockMsg)`.
- `fn dock_order_for(&self, output: &WlOutput) -> Vec<Handle>`: clients whose `should_show` is true and whose `output_for(info)` == `output`, ordered by `rules::dock_order` over `(handle, login.label())`.
- `reconcile_surfaces` becomes mode-aware:
  - `Mode::Floating`: as now, plus destroy all `docks` (and `Cmd::ResumeCapture` for shown clients).
  - `Mode::Dock`: destroy every client's floating surface (`forget_surface` + `destroy_layer_surface`; pause nothing yet), then `reconcile_docks()`: for each known output, if `dock_order_for(output)` is non-empty ensure a `DockSurface` exists (`get_layer_surface(dock::settings(id, output, edge, thickness))`), else destroy it. Pause capture for clients not shown, resume for shown (same rule as floating).
- `view_window(id)`: if `self.docks` has `id` → `dock::view(self, &d.output)`; else the existing floating branches.
- `Msg::Dock(Press(h))` → `send(Cmd::Activate(h))`; `RightPress` → `Minimize`; `Enter(h)` → `hovered = true` then resize the dock surface thickness (`set_size` with the axis from `dock::settings`: `(None, Some(t))` or `(Some(t), None)`); `Exit(h)` → `hovered = false`, same resize. Because the strip re-lays out from `view`, the hovered thumbnail grows in place.
- `Frame` arm: in dock mode there is no per-client surface; nothing to resize (the strip thickness derives from `zoomed_size` of the first hovered/any image — keep it simple: thickness uses `None` image i.e. 16:9 default; document).
- `apply_config`: when `mode` or `dock_edge` changed → destroy all docks (ids) and call `reconcile_surfaces()`.
- Pointer path (`on_pointer`) must ignore dock surface ids (`client_for_surface` returns `None` for them — it already does since dock ids are not in any `Client.surface`).
- `Layer(Done)` for a dock id → remove it from `docks` and `reconcile_surfaces()`.

- [ ] **Step 5: Build, test, smoke**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head -40; cargo test 2>&1 | grep 'test result'` → 44 passed.
Smoke (EVE running): default config → floating as before. Then `printf '(mode: Dock)\n' > ~/.config/yutani/config.ron` while running (live reload) → log shows the floating surface destroyed and a dock surface created (add `tracing::info!` in `reconcile_docks` for create/destroy). Delete the config file afterwards; `pkill -f target/debug/yutani`.

- [ ] **Step 6: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: dock mode — per-output strip with widget-level interaction and in-place hover zoom"
```

---

### Task 5: Hands-on acceptance (Daniel) and spec update

- [ ] **Step 1: Hands-on checklist** (run `cargo run -q`; EVE running)
  1. Plain left-click on a floating thumbnail: focus changes, **no visible flash/resize** of the thumbnail.
  2. Drag still works (canvas engages after a few pixels).
  3. Tray: an icon appears in the COSMIC panel's status area; menu shows *Show thumbnails* / *Hide thumbnails* / *Quit*; Hide removes all thumbnails, Show restores them at the same spots; Quit exits cleanly.
  4. `(mode: Dock)` in `~/.config/yutani/config.ron` (live): thumbnails move into a strip along the bottom edge; hover grows the hovered one; left-click focuses; right-click minimises. `(mode: Dock, dock_edge: Left)` → vertical strip on the left. Delete the file → back to floating at the remembered positions.
  5. With `(hide_active: true)`: the hidden client's capture is paused — `RUST_LOG=yutani=debug` should show no `frame`-related activity for it (there is no frame log; instead check `top`: CPU stays ~4 % or lower).
- [ ] **Step 2: Spec update** — §6: dock hover zoom grows in place; tray items; §5: pause implemented. Commit `docs: spec status after plan 3`.

---

## Self-review

**Spec coverage:** §6 dock mode (Task 4: per-output strip, edge, order by name, no drag/pin); §6 tray (Task 3: Show/Hide/Quit — Layouts submenu and Settings come with plans 4/5); §5 pause hidden clients (Task 3); plan-2 deferred polish: click-without-blink, resize dedupe, alloc-failure stop (Task 2), pure rules + tests, snap docs/test (Task 1). Not here: settings window (plan 4), hotkeys/IPC/layouts (plan 5).

**Placeholder scan:** none; API names flagged where they might differ (`row::with_children`, `run_with` fn-pointer) carry a concrete fallback.

**Type consistency:** `rules::should_show` signature matches its call in Task 1 Step 3; `Phase::Idle`/`Outcome::StartDrag` used consistently across Task 2 steps; `Cmd::PauseCapture/ResumeCapture` defined in Task 3 Step 1 and used in Steps 2 and Task 4; `thumbnail::Callbacks` defined in Task 4 Step 3 and used in Step 2; `dock::thickness`/`settings`/`view` signatures consistent between Steps 2 and 4.


---

### Task 6 (addendum, decided with Daniel after hands-on): Dock v2 — per-thumbnail surfaces, centred, new defaults

**Why:** cosmic-comp's `cosmic_corner_radius_layer_v1` rounds a whole layer surface *including its subsurfaces* (verified by eye 2026-09-11), so rounded corners work only when each thumbnail is its own surface. A single wide strip can't round per thumbnail and needs input-region bookkeeping. Daniel's requested default is "one thumbnail, top centre, at the hover size, always".

**Files:** modify `src/ui/dock.rs`, `src/ui/mod.rs`, `src/model/config.rs`, `src/ui/thumbnail.rs` (pin glyph gating).

**Interfaces:**
- `dock::layout(edge: Edge, output: (i32, i32), sizes: &[(u32, u32)], gap: i32, inset: i32) -> Vec<(i32, i32)>` — pure: positions (top-left, logical px) of N thumbnails laid along `edge`, centred along the edge's axis, `inset` px from the edge, `gap` px apart. Tests: 1 and 3 items on Top/Bottom/Left/Right of a 2560×1440 output; odd total widths centre by integer division; empty input → empty.
- Dock mode reuses the floating per-client surfaces: `create_surface` gets its position from `dock::layout` when `mode == Dock` (`placed` is ignored in dock mode); `reconcile_surfaces` in dock mode, after create/destroy, recomputes the layout for each output's shown clients (ordered by `rules::dock_order`) and `set_margin`s any surface whose position changed; hover enter/leave (per-surface, via the existing pointer path — `Msg::Dock`/`mouse_area` are removed) changes that client's size and re-runs the layout so neighbours shift.
- In dock mode: no drag (pointer press on a dock-positioned surface never starts a `DragState` — return `Task::none()`; clicks still Activate/Minimize), no pin (middle click ignored; pin glyph hidden), positions are not persisted.
- Delete: `DockSurface`, `App.docks`, `dock::settings/thickness/thickness_for_strip/item_rects/size_for/anchor_for`, `set_input_zone` use, `Msg::Dock`, `thumbnail::interactive/Callbacks`, `Client.docked` (replace with `mode == Dock` checks), the re-anchor path in `apply_config` (edge change = relayout). Keep `rules::dock_order` and `DOCK_PADDING` (rename `DOCK_GAP = 8`, `DOCK_INSET = 8`).
- Corner radius is requested for every thumbnail surface (already in `create_surface`).
- Config defaults change to: `mode: Dock`, `dock_edge: Top`, `thumb_width: 480`, `zoom_factor: 1.0`, `corner_radius: 8`. Update `defaults_match_spec`/`plan2_defaults`/`plan3_defaults` tests accordingly and `validate` ranges (thumb_width 80..=1600 still fine; zoom 1.0..=4.0 fine).
- Spec: update §6 dock paragraph and §9 defaults to match; note the per-surface design.

**Verify:** `cargo test` green; smoke with EVE and no config file: one `create_surface` at x = (2560−484)/2 = 1038, y = 8 on the EVE output; `(mode: Floating)` → floating at remembered position; delete → back to dock. Hands-on (Daniel): centred at top, rounded corners, 480 px wide, no hover growth; a second client (if available) appears beside it, both centred as a pair.
