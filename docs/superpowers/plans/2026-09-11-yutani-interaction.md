# Yutani Interaction & Robustness — Implementation Plan (plan 2 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the floating thumbnails from plan 1 usable day-to-day: drag to reposition (with grid/edge snapping), pin, remembered positions per character, hover zoom, visibility rules, a greyed "unavailable" state, recovery when an output disappears, and a config file that validates and hot-reloads.

**Architecture:** No new threads or protocols. Pointer input arrives through iced's `listen_with` with the layer-surface id; the UI moves surfaces with `set_margin`/`set_size`. Positions live in a pure `model::layout` module persisted to `~/.config/yutani/layouts/current.ron`. The backend gains one event (`CaptureUnavailable`) and gates `Minimize` on advertised capabilities. Config gains the plan-2 fields, a `validate()` pass, and a `notify`-based reload subscription.

**Tech Stack:** as plan 1 (Rust 2024, libcosmic pinned, `cosmic::cctk`), plus `notify = "8"`.

**Spec:** `docs/superpowers/specs/2026-09-11-yutani-design.md` §4–§6, §9–§10. **Spec deviations decided for this plan** (record in the spec when the plan is done): (1) *Right-click without drag = minimize* replaces Ctrl+click, because layer surfaces with `KeyboardInteractivity::None` never receive modifier state; right-drag still moves. (2) Hover zoom is immediate, not animated over 120 ms (YAGNI). (3) "Unmap" is implemented as destroy + recreate of the layer surface; positions are kept in `Client`, so nothing is lost.

**Prior state:** plan 1 merged on `master`. Files: `src/{main,doctor,cli?}.rs`, `src/model/{client,config}.rs`, `src/backend/{mod,toplevels,capture,buffer,dmabuf,gbm_devices}.rs`, `src/ui/{mod,thumbnail}.rs`. 22 tests. Read `src/ui/mod.rs` and `src/ui/thumbnail.rs` before Tasks 3–6; they are modified heavily.

## Global Constraints

- All Wayland types via `cosmic::cctk::{wayland_client, wayland_protocols, sctk}`; no direct `wayland-client`/`wayland-protocols`/`smithay-client-toolkit` dependency. libcosmic stays pinned at rev `a401af8b1c54a8abd393b8c5b7c8809402f83850`.
- The UI's Wayland event subscription forwards **only** handled variants (`Output`, and from this plan `Layer`). Never forward `RequestResize`/`Frame` (spec §3: measured 40 000 redraws/s loop).
- Layer surfaces: layer Overlay, anchor top-left, exclusive zone 0, keyboard interactivity None, namespace `yutani`. Position = margins (top, left).
- Interaction (spec §6, amended above): left/right drag > 4 px moves (unpinned only); left release without drag → `Cmd::Activate`; right release without drag → `Cmd::Minimize`; middle press → toggle pin; hover enter/leave → zoom to `thumb_width × zoom_factor` and back, growing right/down from the anchored top-left corner.
- Snapping: 32 px grid when `snap_grid`; snap to other thumbnails' edges within 12 px when `snap_edges`.
- Identity across sessions is the **character name**; positions keyed by it in `~/.config/yutani/layouts/current.ron`, saved on every drag end / pin change; outputs referenced by connector name, falling back to the first known output.
- Visibility: `Always` (default) | `EveFocusedOnly` (shown iff some client is activated); `hide_active` hides the activated client's own thumbnail.
- Config: unparseable → warn + defaults, never overwrite; invalid field values → warn + that field's default; hand edits apply live.
- Yutani never sends input to EVE; only `activate` / `set_minimized`, and `set_minimized` only if the compositor advertised the `Minimize` capability.
- Logging via `tracing`. Commit prefix `feat:`/`fix:`/`test:`/`chore:`. Work on a branch off `master`.

## File Structure

| Path | Responsibility |
| --- | --- |
| `src/model/config.rs` (modify) | new fields `zoom_factor`, `visibility`, `hide_active`, `snap_grid`, `snap_edges`; `Config::validate()` |
| `src/model/layout.rs` (new) | `Rect`, `snap()`, `ThumbPos`, `Layout` load/save (`current.ron`) — pure + file I/O, no Wayland |
| `src/model/mod.rs` (modify) | `pub mod layout;` |
| `src/backend/capture.rs` (modify) | emit `Event::CaptureUnavailable` on allocation failure and give-up |
| `src/backend/mod.rs` (modify) | `Event::CaptureUnavailable(Handle)`; store manager `capabilities`; gate `Cmd::Minimize` |
| `src/backend/toplevels.rs` (modify) | store capabilities on `AppData` |
| `src/ui/pointer.rs` (new) | pure drag state machine: `Drag`, `PointerOutcome`, `DragState::on_press/on_move/on_release` |
| `src/ui/mod.rs` (modify) | `Msg::Pointer`, `Msg::Layer`, `Msg::ConfigChanged`; positions from layout; visibility; zoom; output-loss recovery |
| `src/ui/thumbnail.rs` (modify) | unavailable placeholder; pin glyph; `zoomed_size()` |
| `src/ui/config_watch.rs` (new) | `notify` subscription → `Msg::ConfigChanged(Config)` |
| `Cargo.toml` (modify) | add `notify = "8"` |

---

### Task 1: Config fields, validation, and the pure layout model

**Files:**
- Modify: `src/model/config.rs`, `src/model/mod.rs`
- Create: `src/model/layout.rs`

**Interfaces:**
- Produces: `Config { …, zoom_factor: f32, visibility: Visibility, hide_active: bool, snap_grid: bool, snap_edges: bool }`, `enum Visibility { Always, EveFocusedOnly }`, `Config::validate(self) -> Config` (also applied inside `load_from`); `layout::Rect { x, y, w, h: i32 }`, `layout::snap(rect: Rect, others: &[Rect], grid: Option<i32>, edge_threshold: Option<i32>) -> (i32, i32)`, `layout::ThumbPos { output: String, x: i32, y: i32, pinned: bool }`, `layout::Layout { thumbs: BTreeMap<String, ThumbPos> }` with `Layout::load() -> Layout`, `Layout::load_from(&Path)`, `Layout::save(&self) -> anyhow::Result<()>`, `Layout::save_to(&self, &Path)`, `layout::current_path() -> PathBuf` (`~/.config/yutani/layouts/current.ron`).

- [ ] **Step 1: Write the failing tests — config**

Append to the `tests` module in `src/model/config.rs`:

```rust
    #[test]
    fn plan2_defaults() {
        let c = Config::default();
        assert_eq!(c.zoom_factor, 1.5);
        assert_eq!(c.visibility, Visibility::Always);
        assert!(!c.hide_active);
        assert!(c.snap_grid);
        assert!(c.snap_edges);
    }

    #[test]
    fn validate_replaces_bad_values_with_defaults() {
        let c = Config {
            thumb_width: 10,
            opacity: 7.0,
            fps: 17,
            zoom_factor: 0.2,
            active_border: "nope".into(),
            border_px: 99,
            ..Config::default()
        }
        .validate();
        let d = Config::default();
        assert_eq!(c.thumb_width, d.thumb_width);
        assert_eq!(c.opacity, d.opacity);
        assert_eq!(c.fps, d.fps);
        assert_eq!(c.zoom_factor, d.zoom_factor);
        assert_eq!(c.active_border, d.active_border);
        assert_eq!(c.border_px, d.border_px);
    }

    #[test]
    fn validate_keeps_good_values() {
        let c = Config { thumb_width: 480, opacity: 0.5, fps: 60, zoom_factor: 2.0, border_px: 0, ..Config::default() };
        assert_eq!(c.clone().validate(), c);
    }

    #[test]
    fn load_from_validates() {
        let dir = std::env::temp_dir().join(format!("yutani-val-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        std::fs::write(&path, "(fps: 17, thumb_width: 400)").unwrap();
        let c = Config::load_from(&path);
        assert_eq!(c.fps, 30);
        assert_eq!(c.thumb_width, 400);
        std::fs::remove_dir_all(&dir).unwrap();
    }
```

- [ ] **Step 2: Write the failing tests — layout**

Create `src/model/layout.rs`:

```rust
//! Thumbnail geometry (snapping) and persisted per-character positions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThumbPos {
    pub output: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Keyed by character name.
    pub thumbs: BTreeMap<String, ThumbPos>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32) -> Rect {
        Rect { x, y, w: 100, h: 60 }
    }

    #[test]
    fn no_snapping_returns_input() {
        assert_eq!(snap(r(37, 51), &[], None, None), (37, 51));
    }

    #[test]
    fn grid_snaps_to_nearest_multiple() {
        assert_eq!(snap(r(37, 51), &[], Some(32), None), (32, 64));
        assert_eq!(snap(r(47, 15), &[], Some(32), None), (32, 0));
        assert_eq!(snap(r(49, 17), &[], Some(32), None), (64, 32));
    }

    #[test]
    fn edge_snaps_flush_against_neighbour_within_threshold() {
        let other = r(200, 40); // occupies x 200..300, y 40..100
        // our right edge (x+100) near other's left edge (200)
        assert_eq!(snap(r(93, 40), &[other], None, Some(12)), (100, 40));
        // our left edge near other's right edge (300)
        assert_eq!(snap(r(308, 40), &[other], None, Some(12)), (300, 40));
        // our top near other's bottom (100)
        assert_eq!(snap(r(200, 109), &[other], None, Some(12)), (200, 100));
        // our bottom (y+60) near other's top (40)
        assert_eq!(snap(r(200, -25), &[other], None, Some(12)), (200, -20));
        // beyond threshold: untouched
        assert_eq!(snap(r(80, 40), &[other], None, Some(12)), (80, 40));
    }

    #[test]
    fn edge_snapping_also_aligns_same_side_edges() {
        let other = r(200, 40);
        // our left edge near other's left edge
        assert_eq!(snap(r(205, 300), &[other], None, Some(12)), (200, 300));
        // our top near other's top
        assert_eq!(snap(r(500, 45), &[other], None, Some(12)), (500, 40));
    }

    #[test]
    fn edge_snap_wins_over_grid_when_both_enabled() {
        let other = r(200, 40);
        assert_eq!(snap(r(93, 51), &[other], Some(32), Some(12)), (100, 64));
    }

    #[test]
    fn layout_round_trips_and_missing_is_empty() {
        let dir = std::env::temp_dir().join(format!("yutani-layout-{}", std::process::id()));
        let path = dir.join("current.ron");
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), ThumbPos { output: "DP-1".into(), x: 40, y: 40, pinned: true });
        l.save_to(&path).unwrap();
        assert_eq!(Layout::load_from(&path), l);
        assert_eq!(Layout::load_from(&dir.join("nope.ron")), Layout::default());
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(Layout::load_from(&path), Layout::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

Add `pub mod layout;` to `src/model/mod.rs`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd ~/Yutani && cargo test model 2>&1 | grep -E '^error' | head -5`
Expected: errors — `zoom_factor`, `Visibility`, `validate`, `snap`, `load_from`, `save_to` not found.

- [ ] **Step 4: Implement the config changes**

In `src/model/config.rs`, add after the imports:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Always,
    EveFocusedOnly,
}
```

Add the fields to `Config` (after `show_names`):

```rust
    /// Hover zoom multiplier, 1.0–4.0.
    pub zoom_factor: f32,
    pub visibility: Visibility,
    /// Hide the thumbnail of the client that currently has focus.
    pub hide_active: bool,
    /// Snap to a 32 px grid while dragging.
    pub snap_grid: bool,
    /// Snap flush against other thumbnails while dragging.
    pub snap_edges: bool,
```

and to `Default`:

```rust
            zoom_factor: 1.5,
            visibility: Visibility::Always,
            hide_active: false,
            snap_grid: true,
            snap_edges: true,
```

Add to `impl Config`:

```rust
    /// Replace out-of-range values with defaults, warning about each.
    pub fn validate(mut self) -> Self {
        let d = Config::default();
        macro_rules! check {
            ($field:ident, $ok:expr, $why:literal) => {
                if !$ok(&self.$field) {
                    tracing::warn!(concat!("config: ", stringify!($field), " {:?} is invalid (", $why, "); using default"), self.$field);
                    self.$field = d.$field.clone();
                }
            };
        }
        check!(thumb_width, |v: &u32| (80..=1600).contains(v), "80..=1600");
        check!(opacity, |v: &f32| (0.0..=1.0).contains(v), "0.0..=1.0");
        check!(fps, |v: &u32| [10, 15, 30, 60].contains(v), "10|15|30|60");
        check!(zoom_factor, |v: &f32| (1.0..=4.0).contains(v), "1.0..=4.0");
        check!(border_px, |v: &u32| *v <= 16, "0..=16");
        check!(active_border, |v: &String| parse_color(v).is_some(), "#rrggbb[aa]");
        check!(inactive_border, |v: &String| parse_color(v).is_some(), "#rrggbb[aa]");
        if self.app_ids.is_empty() {
            tracing::warn!("config: app_ids is empty; using default");
            self.app_ids = d.app_ids.clone();
        }
        self
    }
```

In `load_from`, change the success arm `Ok(config) => config,` to `Ok(config) => Config::validate(config),`.

- [ ] **Step 5: Implement the layout module**

Insert between the `Layout` struct and the tests in `src/model/layout.rs`:

```rust
fn round_to(v: i32, grid: i32) -> i32 {
    ((v as f64 / grid as f64).round() as i32) * grid
}

/// Snap a dragged rect's top-left. Edge snapping (flush against, or aligned
/// with, another rect's edges within `edge_threshold`) takes precedence over
/// the grid, per axis.
pub fn snap(rect: Rect, others: &[Rect], grid: Option<i32>, edge_threshold: Option<i32>) -> (i32, i32) {
    let mut x = rect.x;
    let mut y = rect.y;
    if let Some(g) = grid.filter(|g| *g > 0) {
        x = round_to(x, g);
        y = round_to(y, g);
    }
    if let Some(t) = edge_threshold.filter(|t| *t > 0) {
        let mut best_x: Option<(i32, i32)> = None; // (distance, snapped x)
        let mut best_y: Option<(i32, i32)> = None;
        for o in others {
            // candidate x values: flush right-of-o, flush left-of-o, aligned left edges
            for cand in [o.x + o.w, o.x - rect.w, o.x] {
                let d = (rect.x - cand).abs();
                if d <= t && best_x.is_none_or(|(bd, _)| d < bd) {
                    best_x = Some((d, cand));
                }
            }
            for cand in [o.y + o.h, o.y - rect.h, o.y] {
                let d = (rect.y - cand).abs();
                if d <= t && best_y.is_none_or(|(bd, _)| d < bd) {
                    best_y = Some((d, cand));
                }
            }
        }
        if let Some((_, sx)) = best_x {
            x = sx;
        }
        if let Some((_, sy)) = best_y {
            y = sy;
        }
    }
    (x, y)
}

pub fn current_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yutani")
        .join("layouts")
        .join("current.ron")
}

impl Layout {
    pub fn load() -> Self {
        Self::load_from(&current_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("cannot read {}: {e}; starting with an empty layout", path.display());
                return Self::default();
            }
        };
        ron::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("cannot parse {}: {e}; starting with an empty layout", path.display());
            Self::default()
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&current_path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?)?;
        Ok(())
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd ~/Yutani && cargo test model 2>&1 | grep 'test result'`
Expected: `test result: ok. 22 passed` (12 config + 7 client + … — the total for the crate becomes 33; the `model` filter shows the model subset). Full suite: `cargo test 2>&1 | grep 'test result'` → 33 passed.

Note for the edge test `snap(r(200, -25), …)`: rect bottom = -25+60 = 35, other's top = 40, distance 5 ≤ 12 → y = 40−60 = −20. ✓

- [ ] **Step 7: Commit**

```bash
cd ~/Yutani && git add src/model && git commit -m "feat: plan-2 config fields with validation; pure layout model with snapping and current.ron"
```

---

### Task 2: Backend — `CaptureUnavailable` event and capability-gated minimize

**Files:**
- Modify: `src/backend/mod.rs`, `src/backend/capture.rs`, `src/backend/toplevels.rs`

**Interfaces:**
- Produces: `Event::CaptureUnavailable(Handle)`; `AppData.capabilities: HashSet<ZcosmicToplelevelManagementCapabilitiesV1>`; `Cmd::Minimize` is a no-op with a `warn!` unless `capabilities` contains `Minimize`.

- [ ] **Step 1: Add the event and capability storage in `src/backend/mod.rs`**

In `pub enum Event` add, after `Frame(Handle, CaptureImage),`:

```rust
    /// Capture for this client could not be (re)started; the UI should grey it out
    /// until the next `Frame`.
    CaptureUnavailable(Handle),
```

Add to `AppData` (and initialise in `start` with `HashSet::new()`):

```rust
    pub capabilities: HashSet<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>,
```

with the import `use cosmic::cctk::cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1;` and `use std::collections::HashSet;`.

Change the `Cmd::Minimize` arm of `handle_cmd` to:

```rust
            Cmd::Minimize(handle) => {
                use zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1 as Cap;
                if !self.capabilities.contains(&Cap::Minimize) {
                    tracing::warn!("compositor does not advertise the Minimize capability; ignoring");
                    return;
                }
                let Some(cosmic) = self.cosmic_handle(&handle) else { return };
                let Some(manager) = &self.toplevel_manager_state else { return };
                manager.manager.set_minimized(&cosmic);
            }
```

- [ ] **Step 2: Store capabilities in `src/backend/toplevels.rs`**

In `ToplevelManagerHandler::capabilities`, after building `caps`, replace the log line with:

```rust
        tracing::info!(?caps, "toplevel manager capabilities");
        self.capabilities = caps;
```

- [ ] **Step 3: Emit `CaptureUnavailable` in `src/backend/capture.rs`**

- In `init_done`, where `allocate` returns `None` (buffers could not be created), send `Event::CaptureUnavailable(capture.handle.clone())` (after dropping any session guard).
- In `failed`'s `BufferConstraints` arm, if `allocate` returns `None`, likewise send it.
- In the give-up branch (`n >= 5`), after `capture.stop()`, send `Event::CaptureUnavailable(capture.handle.clone())`.

Make sure no `capture.session` guard is held while calling `self.send_event` (it blocks on the channel).

- [ ] **Step 4: Log it in the UI for now**

In `src/ui/mod.rs` `on_backend`, add an arm:

```rust
            Event::CaptureUnavailable(handle) => {
                if let Some(c) = self.clients.get(&handle) {
                    tracing::warn!(label = c.info.login.label(), "capture unavailable");
                }
                Task::none()
            }
```

- [ ] **Step 5: Build, test, smoke**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head -30; cargo test 2>&1 | grep 'test result'`
Expected: clean; 33 passed.

Smoke (EVE or a Firefox window must be running; the default config detects only EVE — if no EVE, temporarily write `(app_ids: ["firefox"])` to `~/.config/yutani/config.ron` and delete it afterwards): `RUST_LOG=yutani=info timeout 6 ./target/debug/yutani 2>&1 | grep -E 'capabilities|client added' | cut -c1-140` → capabilities line and at least one client.

- [ ] **Step 6: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: CaptureUnavailable event; gate minimize on compositor capability"
```

---

### Task 3: Pointer interaction — drag with snapping, click-to-focus, right-click minimize, middle-click pin, persisted positions

**Files:**
- Create: `src/ui/pointer.rs`
- Modify: `src/ui/mod.rs`, `src/ui/thumbnail.rs`

**Interfaces:**
- Produces (`ui::pointer`): `pub struct DragState { surface: SurfaceId, button: Button, press_abs: (f32, f32), start_pos: (i32, i32), moved: bool }`; `pub enum Outcome { None, Move((i32, i32)), Click(Button), DragEnd }`; `pub fn on_press(surface, button, cursor: Point, pos: (i32, i32), pinned: bool) -> Option<DragState>`; `pub fn on_move(drag: &mut DragState, cursor: Point, current_pos: (i32, i32)) -> Outcome`; `pub fn on_release(drag: DragState) -> Outcome`; `pub const DRAG_THRESHOLD: f32 = 4.0;`
- Produces (`ui`): `Msg::Pointer(SurfaceId, mouse::Event)`, `Msg::Minimize(Handle)`; `Client` gains `pinned: bool`; `App` gains `layout: Layout`, `drag: Option<DragState>`; `App::rect_of(&Client) -> Rect`, `App::apply_saved_position(&Handle) -> Task<Msg>`, `App::persist_position(&Handle)`.
- `thumbnail::view` no longer wraps in `mouse_area`; it takes `pinned` from `Client` and draws a 📌 glyph top-right when pinned.

- [ ] **Step 1: Write the failing tests for the drag state machine**

Create `src/ui/pointer.rs`:

```rust
//! Pure drag/click state machine for thumbnails. Cursor positions are
//! surface-local (iced `Point`); we convert to absolute output coordinates by
//! adding the surface's current position so a moving surface does not
//! confuse the delta.

use cosmic::iced::Point;
use cosmic::iced::mouse::Button;
use cosmic::iced::window::Id as SurfaceId;

pub const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct DragState {
    pub surface: SurfaceId,
    pub button: Button,
    pub press_abs: (f32, f32),
    pub start_pos: (i32, i32),
    pub pinned: bool,
    pub moved: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    /// Move the surface to this top-left (before snapping).
    Move((i32, i32)),
    /// Button released without dragging.
    Click(Button),
    /// Button released after dragging.
    DragEnd,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> SurfaceId {
        SurfaceId::unique()
    }

    #[test]
    fn press_left_or_right_starts_drag_middle_does_not() {
        assert!(on_press(sid(), Button::Left, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Right, Point::new(5.0, 5.0), (40, 40), false).is_some());
        assert!(on_press(sid(), Button::Middle, Point::new(5.0, 5.0), (40, 40), false).is_none());
    }

    #[test]
    fn small_motion_is_not_a_drag_and_release_is_a_click() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        assert_eq!(on_move(&mut d, Point::new(12.0, 11.0), (40, 40)), Outcome::None);
        assert!(!d.moved);
        assert_eq!(on_release(d), Outcome::Click(Button::Left));
    }

    #[test]
    fn motion_beyond_threshold_moves_by_absolute_delta() {
        let mut d = on_press(sid(), Button::Left, Point::new(10.0, 10.0), (40, 40), false).unwrap();
        // cursor moved +20,+5 within the (not yet moved) surface
        assert_eq!(on_move(&mut d, Point::new(30.0, 15.0), (40, 40)), Outcome::Move((60, 45)));
        // surface now at (60,45); cursor back at local (10,10) means no further motion
        assert_eq!(on_move(&mut d, Point::new(10.0, 10.0), (60, 45)), Outcome::Move((60, 45)));
        // then +3 more in x
        assert_eq!(on_move(&mut d, Point::new(13.0, 10.0), (60, 45)), Outcome::Move((63, 45)));
        assert!(d.moved);
        assert_eq!(on_release(d), Outcome::DragEnd);
    }

    #[test]
    fn pinned_surface_never_moves_but_still_clicks() {
        let mut d = on_press(sid(), Button::Right, Point::new(10.0, 10.0), (40, 40), true).unwrap();
        assert_eq!(on_move(&mut d, Point::new(90.0, 90.0), (40, 40)), Outcome::None);
        assert_eq!(on_release(d), Outcome::Click(Button::Right));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/Yutani && cargo test ui::pointer 2>&1 | grep -E '^error' | head -3`
Expected: `on_press`/`on_move`/`on_release` not found (add `pub mod pointer;` to `src/ui/mod.rs` first so the module is compiled).

- [ ] **Step 3: Implement the state machine**

Insert above the tests in `src/ui/pointer.rs`:

```rust
pub fn on_press(surface: SurfaceId, button: Button, cursor: Point, pos: (i32, i32), pinned: bool) -> Option<DragState> {
    if !matches!(button, Button::Left | Button::Right) {
        return None;
    }
    Some(DragState {
        surface,
        button,
        press_abs: (cursor.x + pos.0 as f32, cursor.y + pos.1 as f32),
        start_pos: pos,
        pinned,
        moved: false,
    })
}

pub fn on_move(drag: &mut DragState, cursor: Point, current_pos: (i32, i32)) -> Outcome {
    if drag.pinned {
        return Outcome::None;
    }
    let abs = (cursor.x + current_pos.0 as f32, cursor.y + current_pos.1 as f32);
    let dx = abs.0 - drag.press_abs.0;
    let dy = abs.1 - drag.press_abs.1;
    if !drag.moved && (dx * dx + dy * dy).sqrt() < DRAG_THRESHOLD {
        return Outcome::None;
    }
    drag.moved = true;
    Outcome::Move((drag.start_pos.0 + dx.round() as i32, drag.start_pos.1 + dy.round() as i32))
}

pub fn on_release(drag: DragState) -> Outcome {
    if drag.moved { Outcome::DragEnd } else { Outcome::Click(drag.button) }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ~/Yutani && cargo test ui::pointer 2>&1 | grep 'test result'`
Expected: `4 passed`.

- [ ] **Step 5: Wire pointer events into `src/ui/mod.rs`**

Add imports:

```rust
use cosmic::iced::mouse;
use cosmic::iced::platform_specific::shell::commands::layer_surface::set_margin;
use crate::model::layout::{self, Layout, Rect, ThumbPos};
use crate::model::client::Login;
pub mod pointer;
```

Extend `Client`:

```rust
    pub pinned: bool,
```

(initialise `pinned: false` where `Client` is constructed).

Extend `App`:

```rust
    pub layout: Layout,
    pub drag: Option<pointer::DragState>,
```

(initialise in `init` with `layout: Layout::load()` and `drag: None`).

Extend `Msg`:

```rust
    Pointer(SurfaceId, mouse::Event),
    Minimize(Handle),
```

Extend `subscription()` — the `listen_with` closure must also forward mouse events with their surface id. Replace the closure with:

```rust
        let events = iced::event::listen_with(|event, _status, id| match event {
            iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(
                event @ WaylandEvent::Output(..),
            )) => Some(Msg::Wayland(event)),
            iced::Event::Mouse(m) => Some(Msg::Pointer(id, m)),
            // Every other event (RequestResize, Frame, keyboard, …) must not become
            // a message: update → redraw → same event again is a hot loop.
            _ => None,
        });
```

Add helpers to `impl App`:

```rust
    fn client_for_surface(&self, id: SurfaceId) -> Option<Handle> {
        self.clients.iter().find(|(_, c)| c.surface == Some(id)).map(|(h, _)| h.clone())
    }

    fn rect_of(&self, client: &Client) -> Rect {
        let (w, h) = thumbnail::size(&self.config, client.image.as_ref());
        Rect { x: client.position.0, y: client.position.1, w: w as i32, h: h as i32 }
    }

    fn output_name_of(&self, client: &Client) -> Option<String> {
        self.output_for(&client.info)
            .and_then(|o| self.outputs.iter().find(|k| k.handle == o).map(|k| k.name.clone()))
    }

    /// Remember this client's position (and pin state) under its character name.
    fn persist_position(&mut self, handle: &Handle) {
        let Some(client) = self.clients.get(handle) else { return };
        let Login::LoggedIn(name) = &client.info.login else { return };
        let Some(output) = self.output_name_of(client) else { return };
        self.layout.thumbs.insert(
            name.clone(),
            ThumbPos { output, x: client.position.0, y: client.position.1, pinned: client.pinned },
        );
        if let Err(e) = self.layout.save() {
            tracing::warn!("cannot save layout: {e}");
        }
    }

    /// If we have a saved position for this character, move there.
    fn apply_saved_position(&mut self, handle: &Handle) -> Task<Msg> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Login::LoggedIn(name) = &client.info.login else { return Task::none() };
        let Some(saved) = self.layout.thumbs.get(name).cloned() else { return Task::none() };
        let client = self.clients.get_mut(handle).unwrap();
        client.position = (saved.x, saved.y);
        client.pinned = saved.pinned;
        match client.surface {
            Some(id) => set_margin(id, saved.y, 0, 0, saved.x),
            None => Task::none(),
        }
    }

    fn on_pointer(&mut self, id: SurfaceId, event: mouse::Event) -> Task<Msg> {
        let Some(handle) = self.client_for_surface(id) else { return Task::none() };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Middle) => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.pinned = !c.pinned;
                }
                self.persist_position(&handle);
                Task::none()
            }
            mouse::Event::ButtonPressed(button) => {
                let c = &self.clients[&handle];
                // Position of the cursor at press time is not part of ButtonPressed;
                // use the last CursorMoved we saw for this surface.
                let cursor = c.last_cursor;
                self.drag = pointer::on_press(id, button, cursor, c.position, c.pinned);
                Task::none()
            }
            mouse::Event::CursorMoved { position } => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.last_cursor = position;
                }
                let Some(drag) = self.drag.as_mut().filter(|d| d.surface == id) else { return Task::none() };
                let current = self.clients[&handle].position;
                match pointer::on_move(drag, position, current) {
                    pointer::Outcome::Move(raw) => {
                        let me = self.rect_of(&self.clients[&handle]);
                        let others: Vec<Rect> = self
                            .clients
                            .iter()
                            .filter(|(h, c)| *h != &handle && c.surface.is_some())
                            .map(|(_, c)| self.rect_of(c))
                            .collect();
                        let grid = self.config.snap_grid.then_some(32);
                        let edges = self.config.snap_edges.then_some(12);
                        let (x, y) = layout::snap(Rect { x: raw.0, y: raw.1, ..me }, &others, grid, edges);
                        let (x, y) = (x.max(0), y.max(0));
                        self.clients.get_mut(&handle).unwrap().position = (x, y);
                        set_margin(id, y, 0, 0, x)
                    }
                    _ => Task::none(),
                }
            }
            mouse::Event::ButtonReleased(button) => {
                let Some(drag) = self.drag.take().filter(|d| d.surface == id && d.button == button) else {
                    return Task::none();
                };
                match pointer::on_release(drag) {
                    pointer::Outcome::Click(mouse::Button::Left) => {
                        self.send(Cmd::Activate(handle));
                    }
                    pointer::Outcome::Click(mouse::Button::Right) => {
                        self.send(Cmd::Minimize(handle));
                    }
                    pointer::Outcome::DragEnd => self.persist_position(&handle),
                    _ => {}
                }
                Task::none()
            }
            mouse::Event::CursorLeft => {
                // Releasing outside is delivered to us anyway (implicit grab); nothing to do.
                Task::none()
            }
            _ => Task::none(),
        }
    }
```

`Client` needs `pub last_cursor: Point` (init `Point::ORIGIN`); import `cosmic::iced::Point`.

In `update`, add arms:

```rust
            Msg::Pointer(id, event) => self.on_pointer(id, event),
            Msg::Minimize(handle) => {
                self.send(Cmd::Minimize(handle));
                Task::none()
            }
```

In `on_backend`'s `ClientAdded | ClientUpdated` arm, after updating `entry.info`, detect a newly resolved name and apply the saved position:

```rust
                let became_named = !was_named && matches!(entry.info.login, Login::LoggedIn(_));
                let create = self.create_surface(&handle);
                if became_named {
                    Task::batch([create, self.apply_saved_position(&handle)])
                } else {
                    create
                }
```

where `was_named` is computed before `entry.info = info;` as `matches!(entry.info.login, Login::LoggedIn(_))` (for a brand-new entry it is false). Also, in `create_surface`, before choosing `next_position`, prefer a saved position: if the client is `LoggedIn(name)` and `self.layout.thumbs` has `name`, use `(saved.x, saved.y)` and `pinned`; otherwise `next_position()`.

In `view_window`, pass nothing new — `thumbnail::view(client, &self.config)`; the `on_press` parameter is removed (Step 6).

- [ ] **Step 6: Update the widget in `src/ui/thumbnail.rs`**

- Change the signature to `pub fn view<'a>(client: &'a Client, config: &Config) -> Element<'a, Msg>` and drop the `mouse_area` wrapper: return `framed.into()`.
- Add a pin glyph layer when `client.pinned`:

```rust
    if client.pinned {
        layers.push(
            widget::container(widget::text("📌").size(14))
                .align_top(Length::Fill)
                .align_right(Length::Fill)
                .padding(4)
                .into(),
        );
    }
```

(If `align_top`/`align_right` do not exist on this container, use `.align_x(Horizontal::Right).align_y(Vertical::Top)` with `cosmic::iced::alignment::{Horizontal, Vertical}` — check what plan 1 ended up using for the name label and mirror it.)

- [ ] **Step 7: Build and test**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head -40; cargo test 2>&1 | grep 'test result'`
Expected: clean; 37 passed.

- [ ] **Step 8: Hands-on test (Daniel) — record results in the report**

Run `RUST_LOG=yutani=info cargo run -q` with EVE running:
1. Left-drag the thumbnail: it follows the cursor; snaps to 32 px steps; `~/.config/yutani/layouts/current.ron` contains the character name and position after release.
2. Left-click (no drag): EVE gets focus. Right-click (no drag): EVE minimises (thumbnail stays; border goes grey). Click the thumbnail again: EVE restores and focuses.
3. Middle-click: 📌 appears; dragging no longer moves it; middle-click again unpins.
4. Quit, relaunch: thumbnail appears at the saved position (and pinned if it was).
5. With two clients: drag one near the other — it snaps flush at ≤ 12 px.

- [ ] **Step 9: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: drag with grid/edge snapping, click-to-focus, right-click minimize, middle-click pin, persisted positions"
```

---

### Task 4: Hover zoom

**Files:**
- Modify: `src/ui/thumbnail.rs`, `src/ui/mod.rs`

**Interfaces:**
- Produces: `thumbnail::zoomed_size(config, image, zoomed: bool) -> (u32, u32)` (= `size` scaled by `zoom_factor` when zoomed, border unscaled); `Client.hovered: bool`; `App::surface_size(&Client) -> (u32, u32)` used everywhere a size is needed (create, Frame resize, zoom).

- [ ] **Step 1: Write the failing test**

In `src/ui/thumbnail.rs` tests:

```rust
    #[test]
    fn zoomed_size_scales_content_not_border() {
        let config = Config { thumb_width: 320, border_px: 2, zoom_factor: 1.5, ..Config::default() };
        assert_eq!(zoomed_size(&config, None, false), (324, 184));
        assert_eq!(zoomed_size(&config, None, true), (484, 274));
    }
```

(320×1.5 = 480 wide; 480×9/16 = 270 high; plus 2×2 border.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd ~/Yutani && cargo test zoomed_size 2>&1 | grep -E '^error' | head -2`
Expected: `zoomed_size` not found.

- [ ] **Step 3: Implement**

In `src/ui/thumbnail.rs`:

```rust
pub fn zoomed_size(config: &Config, image: Option<&CaptureImage>, zoomed: bool) -> (u32, u32) {
    if !zoomed {
        return size(config, image);
    }
    let scaled = Config { thumb_width: (config.thumb_width as f32 * config.zoom_factor).round() as u32, ..config.clone() };
    size(&scaled, image)
}
```

In `src/ui/mod.rs`: add `pub hovered: bool` to `Client` (init false); add

```rust
    fn surface_size(&self, client: &Client) -> (u32, u32) {
        thumbnail::zoomed_size(&self.config, client.image.as_ref(), client.hovered)
    }
```

and use `self.surface_size(client)` in `create_surface` and in the `Frame` arm (both `old` and `new`). In `on_pointer`, add arms:

```rust
            mouse::Event::CursorEntered => {
                let c = self.clients.get_mut(&handle).unwrap();
                c.hovered = true;
                let (w, h) = self.surface_size(&self.clients[&handle]);
                set_size(id, Some(w), Some(h))
            }
            mouse::Event::CursorLeft => {
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    return Task::none(); // keep zoomed while dragging
                }
                let c = self.clients.get_mut(&handle).unwrap();
                c.hovered = false;
                let (w, h) = self.surface_size(&self.clients[&handle]);
                set_size(id, Some(w), Some(h))
            }
```

(replace the earlier `CursorLeft => Task::none()` arm). `rect_of` should use the **unzoomed** size so snapping is stable: keep it calling `thumbnail::size`.

- [ ] **Step 4: Build, test, hands-on**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head; cargo test 2>&1 | grep 'test result'` → 38 passed.
Hands-on (Daniel): hover → thumbnail grows 1.5× from its top-left corner, still live; leave → shrinks back; dragging while zoomed works.

- [ ] **Step 5: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: hover zoom"
```

---

### Task 5: Visibility rules, unavailable placeholder, and output-loss recovery

**Files:**
- Modify: `src/ui/mod.rs`, `src/ui/thumbnail.rs`

**Interfaces:**
- Produces: `App::should_show(&Client) -> bool`; `App::reconcile_surfaces() -> Task<Msg>` (creates/destroys so each client has a surface iff `should_show`); `Client.unavailable: bool`; `Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, _, id))` handled.

- [ ] **Step 1: Visibility**

In `src/ui/mod.rs`:

```rust
    fn any_client_activated(&self) -> bool {
        self.clients.values().any(|c| c.info.activated)
    }

    fn should_show(&self, client: &Client) -> bool {
        use crate::model::config::Visibility;
        let visible = match self.config.visibility {
            Visibility::Always => true,
            Visibility::EveFocusedOnly => self.any_client_activated(),
        };
        visible && !(self.config.hide_active && client.info.activated)
    }

    /// Make every client's surface existence match `should_show`.
    fn reconcile_surfaces(&mut self) -> Task<Msg> {
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        let mut tasks = Vec::new();
        for h in handles {
            let show = self.should_show(&self.clients[&h]);
            let has = self.clients[&h].surface.is_some();
            if show && !has {
                tasks.push(self.create_surface(&h));
            } else if !show && has {
                tasks.push(self.destroy_surface(&h));
            }
        }
        Task::batch(tasks)
    }
```

Change `create_surface` so it returns `Task::none()` without creating when `!self.should_show(client)`. In `on_backend`'s `ClientAdded | ClientUpdated` arm, replace the `create_surface` call with `self.reconcile_surfaces()` (an activation change on one client can hide/show others), still batched with `apply_saved_position` when `became_named`. `destroy_surface` must keep `client.position` (it already does — it only takes `surface`).

- [ ] **Step 2: Unavailable placeholder**

- `Client.unavailable: bool` (init false). In `on_backend`: `CaptureUnavailable(handle)` → set `unavailable = true`; `Frame(..)` → set `unavailable = false` before storing the image.
- In `thumbnail::view`, when `client.unavailable`, replace the `Subsurface`/waiting layer with:

```rust
        widget::container(widget::text("capture unavailable").size(12))
            .center(Length::Fill)
            .class(theme::Container::custom(|_| widget::container::Style {
                background: Some(cosmic::iced::Background::Color(cosmic::iced::Color { r: 0.25, g: 0.25, b: 0.25, a: 0.9 })),
                ..Default::default()
            }))
            .into()
```

- [ ] **Step 3: Output-loss recovery**

Forward `Layer` events in the subscription:

```rust
            iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(
                event @ (WaylandEvent::Output(..) | WaylandEvent::Layer(..)),
            )) => Some(Msg::Wayland(event)),
```

In `update`, add:

```rust
            Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, _, id)) => {
                // The compositor closed this surface (its output went away).
                if let Some(handle) = self.client_for_surface(id) {
                    tracing::info!("layer surface closed by compositor; recreating");
                    if let Some(c) = self.clients.get_mut(&handle) {
                        c.surface = None;
                    }
                    return self.reconcile_surfaces();
                }
                Task::none()
            }
            Msg::Wayland(WaylandEvent::Layer(..)) => Task::none(),
```

with `use cosmic::iced::event::wayland::LayerEvent;`. Also, in the `Output(..)` arm, after `on_output`, if the output was **removed**, call `self.reconcile_surfaces()` too (surfaces on it will get `Done`; clients whose `output_for` now resolves to another output are recreated there). `output_for` already falls back to the first known output.

- [ ] **Step 4: Build, test, hands-on**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head; cargo test 2>&1 | grep 'test result'` → 38 passed.
Hands-on (Daniel), by editing `~/.config/yutani/config.ron` between runs (live reload comes in Task 6):
1. `(visibility: EveFocusedOnly)`: thumbnails vanish when a non-EVE window is focused, return when an EVE client is focused.
2. `(hide_active: true)`: the focused client's thumbnail is gone; the other client's remains; switching swaps them.
3. Unplugging a monitor (or toggling one off in COSMIC Settings → Displays) moves thumbnails to the remaining output.

- [ ] **Step 5: Commit**

```bash
cd ~/Yutani && git add src && git commit -m "feat: visibility rules, hide-active, unavailable placeholder, recover from output loss"
```

---

### Task 6: Live config reload

**Files:**
- Create: `src/ui/config_watch.rs`
- Modify: `src/ui/mod.rs`, `Cargo.toml`

**Interfaces:**
- Produces: `config_watch::subscription() -> Subscription<Config>` emitting a validated `Config` whenever `config.ron` changes (debounced 200 ms); `Msg::ConfigChanged(Config)`; `App::apply_config(Config) -> Task<Msg>`.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml` `[dependencies]`: `notify = "8"`.

- [ ] **Step 2: Write `src/ui/config_watch.rs`**

```rust
//! Re-read `config.ron` when it changes on disk.

use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use notify::{RecursiveMode, Watcher};
use std::time::Duration;

use crate::model::config::{Config, config_path};

pub fn subscription() -> Subscription<Config> {
    Subscription::run(run)
}

fn run() -> impl iced::futures::Stream<Item = Config> {
    let (tx, rx) = mpsc::channel::<()>(8);
    let path = config_path();
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    // The watcher lives as long as the stream: move it into the stream's state.
    let watcher = {
        let mut tx = tx.clone();
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && ev.paths.iter().any(|p| p.file_name() == path.file_name())
            {
                let _ = tx.try_send(());
            }
        })
        .and_then(|mut w| {
            std::fs::create_dir_all(&dir).ok();
            w.watch(&dir, RecursiveMode::NonRecursive)?;
            Ok(w)
        })
    };
    if let Err(e) = &watcher {
        tracing::warn!("config watcher unavailable: {e}");
    }
    iced::futures::stream::unfold((rx, watcher), |(mut rx, watcher)| async move {
        rx.next().await?;
        // Debounce bursts (editors write several events per save).
        futures_timer::Delay::new(Duration::from_millis(200)).await;
        while rx.try_next().is_ok_and(|v| v.is_some()) {}
        Some((Config::load(), (rx, watcher)))
    })
}
```

(`Subscription::run` takes a `fn() -> Stream`. If the pinned iced needs `Subscription::run_with` here, mirror the pattern already used in `backend::subscription`.)

- [ ] **Step 3: Apply config changes in `src/ui/mod.rs`**

Add `pub mod config_watch;`, `Msg::ConfigChanged(Config)`, push `config_watch::subscription().map(Msg::ConfigChanged)` into the subscription batch, and:

```rust
    fn apply_config(&mut self, new: Config) -> Task<Msg> {
        if new == self.config {
            return Task::none();
        }
        tracing::info!("config changed; applying");
        if new.app_ids != self.config.app_ids {
            self.send(Cmd::SetAppIds(new.app_ids.clone()));
        }
        if new.fps != self.config.fps {
            self.send(Cmd::SetFps(new.fps));
        }
        self.config = new;
        // Sizes and visibility may have changed.
        let mut tasks = vec![self.reconcile_surfaces()];
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        for h in handles {
            if let Some(id) = self.clients[&h].surface {
                let (w, hgt) = self.surface_size(&self.clients[&h]);
                tasks.push(set_size(id, Some(w), Some(hgt)));
            }
        }
        Task::batch(tasks)
    }
```

and `Msg::ConfigChanged(c) => self.apply_config(c)` in `update`. Border colours/opacity/names apply on the next redraw automatically because `view_window` reads `self.config`.

- [ ] **Step 4: Build, test, hands-on**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A6 | head; cargo test 2>&1 | grep 'test result'` → 38 passed.
Hands-on (Daniel), with the app running: edit `~/.config/yutani/config.ron` and save —
1. `thumb_width: 480` → thumbnails resize immediately.
2. `active_border: "#00ff00"` → border turns green.
3. `fps: 17` → log shows `config: fps 17 is invalid…`; app keeps running at 30.
4. `hide_active: true` → active thumbnail disappears without restart.

- [ ] **Step 5: Update the spec for the decided deviations and commit**

In `docs/superpowers/specs/2026-09-11-yutani-design.md` §6 interaction table: replace the `Ctrl + left click` row with `Right click (no drag) | Command::Minimize(client) — layer surfaces never receive modifier state, so Ctrl-click is not possible`; change the hover row to "resize immediately (no animation)"; in "Visibility" note that unmapping is implemented as destroy/recreate with positions retained.

```bash
cd ~/Yutani && git add Cargo.toml Cargo.lock src docs && git commit -m "feat: live config reload; spec: right-click minimize, immediate zoom"
```

---

## Self-review

**Spec coverage (plan 2 scope):** §6 interaction table — Task 3 (drag/click/pin), Task 4 (hover); Ctrl-click replaced by right-click (recorded as deviation, Task 6 Step 5). §6 visibility — Task 5. §5 `CaptureState::Unavailable` — Tasks 2 + 5. §9 `current.ron` auto-save with output name + fallback — Tasks 1 + 3 (`output_for` fallback already exists). §9 config watched with `notify` — Task 6. §10 invalid config values → defaults with warning — Task 1 `validate`. Multi-monitor output loss — Task 5 (`LayerEvent::Done`). Deferred review items from plan 1 covered: capabilities gating (Task 2), Unavailable placeholder (Tasks 2/5), `LayerEvent::Done` (Task 5), config validation (Task 1), warn→debug for "no outputs" (already done). Not in this plan (plan 3/4): dock, settings window, tray, hotkeys, IPC, named layouts.

**Placeholder scan:** none. Container alignment method names are given with a concrete fallback (mirror what plan 1 used).

**Type consistency:** `Rect{x,y,w,h}` i32 used by `snap` and `rect_of`; `ThumbPos` fields match `persist_position`/`apply_saved_position`; `Outcome` variants match `on_pointer`; `surface_size` replaces `thumbnail::size` calls introduced in Task 3 by Task 4 (Task 4 says so explicitly); `Msg` variants `Pointer`, `Minimize`, `ConfigChanged` are declared where first used.
