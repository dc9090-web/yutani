# Settings Window & Named Layouts Implementation Plan (plan 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `yutani settings` (and the applet's *Preferences…* row) opens a three-page libcosmic settings window whose every change applies live and is written back to `config.ron`; `yutani layout <name>` / `yutani layouts` and the window's Layouts page save, apply, rename and delete named layouts under `~/.config/yutani/layouts/`, with `current.ron` gaining the `order` and `new_client_anchor` fields the spec has always described.

**Architecture:** All decision logic stays pure and unit-tested. `src/model/layout.rs` (library, shared with the applet since plan B) grows the full named-layout file model — `order`, `new_client_anchor`, name validation, list/save/load/rename/delete, the spec §9 output-fallback rule and a pure `placement()` that says where a thumbnail goes and whether its layer surface must be recreated. `src/ui/rules.rs` collapses `dock_order` and `focus_order`'s Dock arm into one comparator that honours the saved `order`. `src/ui/settings.rs` is a new module owning the settings window's state, messages and view; `src/ui/mod.rs` opens/closes the window, routes `Msg::Settings`, and answers the IPC `layout`/`layouts`/`settings` requests that plan 4 left stubbed. Every write to `config.ron` and to a layout file is atomic (tmp + rename) through one helper in `src/model/mod.rs`.

**Tech Stack:** Rust 2024; libcosmic pinned at `a401af8b1c54a8abd393b8c5b7c8809402f83850` with features `tokio, wayland, multi-window, winit, wgpu, single-instance, applet` (all already on after plan B — `multi-window` is what makes the settings window possible, so no feature and no `[[package]]` changes); `ron` 0.10; `serde`/`serde_json` 1; `clap` 4; `dirs` 6.

**This plan:** `docs/superpowers/plans/2026-09-12-yutani-settings-layouts.md`
**Spec:** `docs/superpowers/specs/2026-09-11-yutani-design.md` §3 (crate layout: `ui/settings.rs`), §4 (`new_client_anchor` stacking), §6 (Settings window pages), §7 (shortcut install buttons), §8 (`layout <name>`, `settings`), §9 (config + layout files, output fallback, "applying a named layout copies it to `current.ron`"), §11 (testing).
**Runs after:** `docs/superpowers/plans/2026-09-12-yutani-tunnel.md` (plan A) and `docs/superpowers/plans/2026-09-12-yutani-applet.md` (plan B), on top of both. This plan consumes their names exactly and redefines none of them: plan A's `ipc::Response::OkData(String)`, `ipc::Request::{Status, TunnelConnect, TunnelDisconnect}` and the `handle_request` result type `(Result<Option<String>, String>, Task<cosmic::Action<Msg>>)`; plan B's `src/lib.rs` (`pub mod applet; pub mod assets; pub mod ipc; pub mod model; pub mod tunnel;`), the `use yutani::{ipc, model, tunnel};` re-binding at the binary's crate root (so `crate::model::…` paths inside `src/ui/` keep working), `yutani::applet::Action` and the deletion of `src/ui/tray.rs`/`Msg::Tray`.

### What replaces the tray (plan B removed it)

The spec was written when a `ksni` tray existed. Plan B deleted it, so the two tray entry points the spec mentions are re-homed by this plan:

| Spec §6 text | After plan B + this plan |
|---|---|
| "Settings window … opened from the tray or `yutani settings`" | Opened by the applet's **Preferences…** row (which this plan flips from `xdg-open config.ron` to `ipc::Request::Settings`, Task 5 Step 6) and by `yutani settings` → IPC `settings`. |
| Tray "Layouts submenu (apply)" | `yutani layout <name>` / `yutani layouts` (CLI → IPC) **and** the settings window's Layouts page. No applet menu row is added — the applet popup is specified in full by `2026-09-12-yutani-applet-design.md` and this plan does not change its row list. |

## Global Constraints

- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9
  ```
- Every commit: `cargo build -q` warning-free for new code, `cargo test -q` green.
- **No new `[[package]]` entries in `Cargo.lock`.** Nothing in this plan adds a dependency: `multi-window` is already an enabled libcosmic feature, `serde_json` is a direct dep since plan A, and every widget used comes from `cosmic::widget`. Check with `git diff Cargo.lock | grep '^[-+]name ='` → no output.
- **Test counts are deltas, not absolutes.** After plan B the workspace has three test binaries (the `yutani` lib, the `yutani` bin, the `yutani-applet` bin), so `cargo test -q 2>&1 | grep 'test result'` prints **three** lines. Every expectation in this plan is therefore either a per-filter count (`cargo test -q model::layout 2>&1 | tail -3`) or a stated delta on `cargo test -q 2>&1 | grep -h 'test result' | awk '{s+=$4} END {print s}'` (the sum). Plan B's nominal end state is 149; if plan A or B finished on a different total, the delta is what matters.
- **Config writes are atomic and never clobber an unparseable file.** Both `Config::save_to` and `Layout::save_to` go through `model::write_atomic` (write `<file>.tmp`, `rename` over the target). The settings window never writes `config.ron` while the on-disk file fails to parse: it shows a warning banner, disables every control, and rechecks on each config-watcher event. Serde behaviour, verified against the current code: `Config` is `#[derive(Serialize, Deserialize)] #[serde(default)]` with **no** `deny_unknown_fields`, so unknown/hand-edited keys parse without error but are **dropped on the next save** (the `stale_opacity_field_is_ignored` test depends on the tolerant read). Comments in the user's RON are lost on save too. That is the accepted behaviour and the window says so in one caption line; the only protection the spec demands ("never overwrite the user's file") is the unparseable case, which is enforced above.
- The COSMIC custom-shortcuts file keeps its deliberate **in-place** write (`src/shortcuts.rs`, commit `80d93da`): cosmic-config's watcher ignores paired rename events. `model::write_atomic` is for `config.ron` and layout files only; do not route `shortcuts.rs` through it.
- Layout file names are file **stems** under `~/.config/yutani/layouts/`: non-empty after trimming, ≤ 64 chars, no `/`, no `\`, no NUL or other control characters, not `.` or `..`, and not the reserved name `current` (that is `current.ron`, the auto-saved layout). Rejected names never touch the filesystem.
- Applying a named layout copies it to `current.ron` (spec §9) and repositions live thumbnails. A layout naming an output that is not connected places those thumbnails on the **primary output** — defined here as `self.outputs.first()`, the first output iced reported, because COSMIC exposes no primary-output protocol at this rev — at the same x/y.
- Hands-on steps (opening the window, the visual check, applying layouts across two monitors) are **Task 6** and belong to Daniel. Every other task is verifiable with `cargo test -q` and `cargo build -q` on a machine with no compositor running.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/model/mod.rs` (modify) | `pub fn write_atomic(path, text)` — the one atomic-write helper for `config.ron` and layout files. |
| `src/model/layout.rs` (modify) | The layout file model: `Layout { thumbs, order, new_client_anchor }`, `Anchor`, `Placement`, name validation, `layouts_dir`, list/load/save/rename/delete of named layouts, `stacked_position`, `resolve_output`, `placement`. Pure + temp-dir fs tests. |
| `src/model/config.rs` (modify) | `Config::try_load_from` (strict parse, for the settings window's "is the file broken?" check); `save_to` through `write_atomic`. |
| `src/ui/rules.rs` (modify) | One ordering comparator: `dock_rank`, `focus_order(mode, order, items)`; `dock_order` deleted. |
| `src/ui/mod.rs` (modify) | `Msg::Settings`, settings-window lifecycle, `App::settings`, `apply_layout`, `save_current_layout`, `output_for_thumb`, `handle_request` arms for `Layout`/`Layouts`/`Settings`, `view_window`/`on_close_requested` routing. |
| `src/ui/settings.rs` (new) | Settings-window state, messages, the three pages' views, and the pure field→`Config` mapping (`apply_field`, `prefix_choices`, `parse_border_field`). |
| `src/ui/dock.rs` (modify) | `dock_order_for` uses the new comparator with `self.layout.order`. |
| `src/ui/thumbnail.rs` (modify) | `next_free_x` deleted (replaced by `layout::stacked_position`). |
| `src/ipc.rs` (modify) | `Request::Layouts`; `layout`/`layouts`/`settings` wire forms. |
| `src/cli.rs` (modify) | `pub fn layouts() -> ExitCode` — send `layouts`, print one name per line. |
| `src/main.rs` (modify) | `yutani layout <name>`, `yutani layouts`, `yutani settings` subcommands. |
| `src/bin/yutani-applet/app.rs` (modify) | *Preferences…* flipped from `xdg-open` to IPC `settings`. |
| `src/applet/mod.rs` (modify) | `Action::Preferences.request()` → `Some(Request::Settings)`. |
| `docs/superpowers/specs/2026-09-11-yutani-design.md` (modify) | §6/§8/§9 status notes after acceptance. |

---

### Task 1: The layout file model — `order`, `new_client_anchor`, named files, atomic writes

**Files:**
- Modify: `src/model/mod.rs`, `src/model/layout.rs`, `src/model/config.rs`
- Test: the `mod tests` blocks in `src/model/layout.rs` and `src/model/config.rs`

**Interfaces:**
- Consumes: nothing from plans A/B beyond `src/lib.rs` existing (this is `yutani::model`, the library).
- Produces (every later task depends on these exact names):
  ```rust
  // src/model/mod.rs
  pub fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()>;
  // src/model/layout.rs
  pub const STACK_STEP: i32 = 24;
  pub const MAX_ORDER: usize = 64;
  pub const MAX_NAME: usize = 64;
  pub const CURRENT: &str = "current";
  pub struct Anchor { pub output: String, pub x: i32, pub y: i32 }
  pub struct Layout { pub thumbs: BTreeMap<String, ThumbPos>, pub order: Vec<String>, pub new_client_anchor: Anchor }
  pub struct Placement { pub output: String, pub x: i32, pub y: i32, pub pinned: bool, pub recreate: bool }
  pub fn derive_anchor(thumbs: &BTreeMap<String, ThumbPos>) -> Anchor;
  pub fn merge_order(live: &[String], previous: &[String], cap: usize) -> Vec<String>;
  pub fn stacked_position(anchor: (i32, i32), taken: &[(i32, i32)], step: i32) -> (i32, i32);
  pub fn resolve_output<'a>(saved: &str, connected: &'a [String]) -> Option<&'a str>;
  pub fn placement(saved: &ThumbPos, connected: &[String], current_output: &str) -> Option<Placement>;
  pub fn validate_name(name: &str) -> Result<String, String>;
  pub fn layouts_dir() -> PathBuf;
  pub fn named_path_in(dir: &Path, name: &str) -> Result<PathBuf, String>;
  pub fn named_path(name: &str) -> Result<PathBuf, String>;
  pub fn list_names_in(dir: &Path) -> Vec<String>;
  pub fn list_names() -> Vec<String>;
  pub fn delete_named_in(dir: &Path, name: &str) -> Result<(), String>;
  pub fn delete_named(name: &str) -> Result<(), String>;
  pub fn rename_named_in(dir: &Path, from: &str, to: &str) -> Result<(), String>;
  pub fn rename_named(from: &str, to: &str) -> Result<(), String>;
  impl Layout {
      pub fn load_named_in(dir: &Path, name: &str) -> Result<Layout, String>;
      pub fn load_named(name: &str) -> Result<Layout, String>;
      pub fn save_named_in(&self, dir: &Path, name: &str) -> Result<(), String>;
      pub fn save_named(&self, name: &str) -> Result<(), String>;
  }
  // src/model/config.rs
  impl Config {
      pub fn try_load_from(path: &Path) -> Result<Option<Config>, String>;
      pub fn try_load() -> Result<Option<Config>, String>;
  }
  ```

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block at the bottom of `src/model/layout.rs` (keep every existing test):

```rust
    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pos(output: &str, x: i32, y: i32) -> ThumbPos {
        ThumbPos { output: output.into(), x, y, pinned: false }
    }

    #[test]
    fn layout_round_trips_with_order_and_anchor() {
        let dir = tmpdir("layout-rt");
        let path = dir.join("current.ron");
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), ThumbPos { output: "DP-1".into(), x: 40, y: 40, pinned: true });
        l.thumbs.insert("Kel Draven".into(), pos("DP-1", 380, 40));
        l.order = vec!["Aria Vex".into(), "Kel Draven".into()];
        l.new_client_anchor = Anchor { output: "DP-1".into(), x: 40, y: 40 };
        l.save_to(&path).unwrap();
        assert_eq!(Layout::load_from(&path), l);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_layout_file_without_the_new_fields_still_loads() {
        // Every `current.ron` written before this plan has only `thumbs`.
        let l: Layout = ron::from_str(r#"(thumbs: {"Aria Vex": (output: "DP-1", x: 40, y: 40)})"#).unwrap();
        assert_eq!(l.thumbs.len(), 1);
        assert!(l.order.is_empty());
        assert_eq!(l.new_client_anchor, Anchor::default());
        assert_eq!(Anchor::default(), Anchor { output: String::new(), x: 40, y: 40 });
    }

    #[test]
    fn anchor_is_the_top_left_most_saved_thumbnail() {
        let mut thumbs = BTreeMap::new();
        assert_eq!(derive_anchor(&thumbs), Anchor::default());
        thumbs.insert("Kel".to_string(), pos("DP-1", 380, 40));
        thumbs.insert("Aria".to_string(), pos("DP-1", 40, 40));
        thumbs.insert("Zoe".to_string(), pos("DP-2", 10, 900));
        assert_eq!(derive_anchor(&thumbs), Anchor { output: "DP-1".into(), x: 40, y: 40 });
    }

    #[test]
    fn merge_order_keeps_logged_out_characters_behind_the_live_ones() {
        let live = ["Kel".to_string(), "Aria".to_string()];
        let previous = ["Aria".to_string(), "Zoe".to_string()];
        assert_eq!(merge_order(&live, &previous, MAX_ORDER), vec!["Kel", "Aria", "Zoe"]);
        // The cap bounds the file even after years of characters.
        assert_eq!(merge_order(&live, &previous, 2), vec!["Kel", "Aria"]);
        assert!(merge_order(&[], &[], MAX_ORDER).is_empty());
    }

    #[test]
    fn stacked_position_steps_down_right_past_occupied_slots() {
        assert_eq!(stacked_position((40, 40), &[], 24), (40, 40));
        assert_eq!(stacked_position((40, 40), &[(40, 40)], 24), (64, 64));
        assert_eq!(stacked_position((40, 40), &[(40, 40), (64, 64)], 24), (88, 88));
        // A saved position a few pixels away still counts as occupied, so a
        // new client no longer lands on top of a character's saved spot.
        assert_eq!(stacked_position((40, 40), &[(50, 45)], 24), (64, 64));
        // Far away on one axis only: not a collision.
        assert_eq!(stacked_position((40, 40), &[(400, 45)], 24), (40, 40));
    }

    #[test]
    fn resolve_output_falls_back_to_the_primary_when_the_connector_is_gone() {
        let connected = ["DP-1".to_string(), "HDMI-A-1".to_string()];
        assert_eq!(resolve_output("HDMI-A-1", &connected), Some("HDMI-A-1"));
        assert_eq!(resolve_output("DP-9", &connected), Some("DP-1"));
        assert_eq!(resolve_output("", &connected), Some("DP-1"));
        assert_eq!(resolve_output("DP-1", &[]), None);
    }

    #[test]
    fn placement_recreates_only_when_the_output_changes() {
        let connected = ["DP-1".to_string(), "HDMI-A-1".to_string()];
        let saved = ThumbPos { output: "HDMI-A-1".into(), x: 12, y: 34, pinned: true };
        assert_eq!(
            placement(&saved, &connected, "HDMI-A-1"),
            Some(Placement { output: "HDMI-A-1".into(), x: 12, y: 34, pinned: true, recreate: false })
        );
        assert_eq!(
            placement(&saved, &connected, "DP-1"),
            Some(Placement { output: "HDMI-A-1".into(), x: 12, y: 34, pinned: true, recreate: true })
        );
        // Disconnected connector: the primary output at the same x/y (spec §9).
        let saved = ThumbPos { output: "DP-9".into(), x: 12, y: 34, pinned: false };
        assert_eq!(
            placement(&saved, &connected, "DP-1"),
            Some(Placement { output: "DP-1".into(), x: 12, y: 34, pinned: false, recreate: false })
        );
        assert_eq!(placement(&saved, &[], "DP-1"), None);
    }

    #[test]
    fn layout_names_must_be_plain_file_stems() {
        assert_eq!(validate_name("  pvp fleet  ").unwrap(), "pvp fleet");
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("a\nb").is_err());
        assert!(validate_name("current").is_err());
        assert!(validate_name("CURRENT").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME)).is_ok());
        assert!(validate_name(&"x".repeat(MAX_NAME + 1)).is_err());
    }

    #[test]
    fn named_layouts_save_list_load_rename_and_delete() {
        let dir = tmpdir("layout-named");
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), pos("DP-1", 40, 40));
        l.order = vec!["Aria Vex".into()];

        assert_eq!(list_names_in(&dir), Vec::<String>::new());
        assert_eq!(Layout::load_named_in(&dir, "pvp").unwrap_err(), "no such layout pvp");

        l.save_named_in(&dir, "pvp").unwrap();
        l.save_named_in(&dir, "Mining").unwrap();
        // `current.ron` is not a named layout.
        Layout::default().save_to(&dir.join("current.ron")).unwrap();
        assert_eq!(list_names_in(&dir), vec!["Mining".to_string(), "pvp".to_string()]);
        assert_eq!(Layout::load_named_in(&dir, "pvp").unwrap(), l);

        rename_named_in(&dir, "pvp", "pvp fleet").unwrap();
        assert_eq!(list_names_in(&dir), vec!["Mining".to_string(), "pvp fleet".to_string()]);
        assert!(rename_named_in(&dir, "pvp", "x").unwrap_err().contains("no such layout"));
        assert!(rename_named_in(&dir, "pvp fleet", "Mining").unwrap_err().contains("already exists"));
        assert!(rename_named_in(&dir, "pvp fleet", "a/b").is_err());

        delete_named_in(&dir, "Mining").unwrap();
        assert_eq!(list_names_in(&dir), vec!["pvp fleet".to_string()]);
        assert_eq!(delete_named_in(&dir, "Mining").unwrap_err(), "no such layout Mining");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_is_atomic_and_leaves_no_temporary_behind() {
        let dir = tmpdir("layout-atomic");
        let path = dir.join("current.ron");
        Layout::default().save_to(&path).unwrap();
        let left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["current.ron".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
```

Append to the `mod tests` block at the bottom of `src/model/config.rs`:

```rust
    #[test]
    fn try_load_from_distinguishes_missing_from_broken() {
        let dir = std::env::temp_dir().join(format!("yutani-try-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        assert_eq!(Config::try_load_from(&path), Ok(None));
        std::fs::write(&path, "(fps: 60)").unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap().unwrap().fps, 60);
        // Out-of-range values are still repaired, not an error.
        std::fs::write(&path, "(fps: 17)").unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap().unwrap().fps, 30);
        std::fs::write(&path, "(this is not ron").unwrap();
        assert!(Config::try_load_from(&path).unwrap_err().contains("cannot parse"));
        // A broken file is never rewritten by a failed read.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(this is not ron");
        std::fs::remove_dir_all(&dir).unwrap();
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -q model:: 2>&1 | tail -20`
Expected: FAIL to **compile**, with `error[E0433]`/`error[E0425]` for `Anchor`, `derive_anchor`, `merge_order`, `stacked_position`, `resolve_output`, `placement`, `validate_name`, `list_names_in`, `rename_named_in`, `delete_named_in`, `load_named_in`, `save_named_in`, `MAX_ORDER`, `MAX_NAME`, `Config::try_load_from`, and `no field order on Layout`.

- [ ] **Step 3: Write the implementation**

Replace the whole of `src/model/mod.rs` with:

```rust
//! Pure data model: client identity, user config and saved layouts.

pub mod client;
pub mod config;
pub mod layout;

use std::path::Path;

/// Write `text` to `path` atomically: a sibling temporary file, then a
/// rename over the target. A reader (or the `notify` config watcher) never
/// sees a half-written file, and a failed write leaves the previous
/// contents intact. The temporary deliberately does **not** keep the
/// target's extension, so the config watcher — which keys on the file
/// name — ignores it.
///
/// Not for `~/.config/cosmic/…/custom`: that one must be written in place
/// (see `src/shortcuts.rs`), because cosmic-config's watcher ignores the
/// paired rename events.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
```

In `src/model/layout.rs`, make two surgical replacements and keep everything else (`Rect`, `ThumbPos`, `round_to`, `snap`, and the existing tests) byte-for-byte:

**(a)** replace the four lines
```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Keyed by character name.
    pub thumbs: BTreeMap<String, ThumbPos>,
}
```
with the `Anchor` + `Layout` block below (the first item in the listing);

**(b)** replace everything from `pub fn current_path() -> PathBuf {` down to the closing brace of `impl Layout` (which sits between `snap` and `#[cfg(test)]`) with the rest of the listing.

The listing, in order:

```rust
/// Where a thumbnail whose character name is not known yet is put (spec
/// §4/§9). An empty `output` means "the client's own output, else the
/// primary one".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Anchor {
    pub output: String,
    pub x: i32,
    pub y: i32,
}

impl Default for Anchor {
    fn default() -> Self {
        Self { output: String::new(), x: 40, y: 40 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Keyed by character name.
    pub thumbs: BTreeMap<String, ThumbPos>,
    /// Layout order by character name (spec §9). A character listed here
    /// ranks by its place in the list; anyone else follows by label. This
    /// is what makes a named layout able to fix the **dock** arrangement,
    /// where no positions are stored.
    pub order: Vec<String>,
    /// Where the next unnamed client's thumbnail goes.
    pub new_client_anchor: Anchor,
}

/// Successive unnamed clients stack this far down-right of the anchor.
pub const STACK_STEP: i32 = 24;
/// Upper bound on the recorded `order`, so a long history of characters
/// cannot grow the file without end.
pub const MAX_ORDER: usize = 64;
/// Longest layout name, in characters.
pub const MAX_NAME: usize = 64;
/// Reserved stem: `current.ron` is the auto-saved layout, not a named one.
pub const CURRENT: &str = "current";

/// The anchor a saved layout records: the top-left-most saved thumbnail
/// (smallest y, then x, then connector name), on the output that holds it.
/// With nothing saved, the default (40, 40) on no particular output.
pub fn derive_anchor(thumbs: &BTreeMap<String, ThumbPos>) -> Anchor {
    thumbs
        .values()
        .min_by(|a, b| (a.y, a.x, &a.output).cmp(&(b.y, b.x, &b.output)))
        .map(|t| Anchor { output: t.output.clone(), x: t.x, y: t.y })
        .unwrap_or_default()
}

/// The `order` a save records: every live character in layout order, then
/// the names the file already carried that are not running right now (a
/// character who is merely logged out keeps their place), deduplicated and
/// capped at `cap`.
pub fn merge_order(live: &[String], previous: &[String], cap: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(live.len() + previous.len());
    for name in live.iter().chain(previous.iter()) {
        if !out.iter().any(|n| n == name) {
            out.push(name.clone());
        }
    }
    out.truncate(cap);
    out
}

/// Where an unnamed client's thumbnail goes (spec §4): the anchor, then
/// `step` px down-right per occupied slot. "Occupied" means within `step`
/// on **both** axes of a shown thumbnail or of a saved position, so a new
/// client no longer lands on top of a character's saved spot.
pub fn stacked_position(anchor: (i32, i32), taken: &[(i32, i32)], step: i32) -> (i32, i32) {
    let step = step.max(1);
    (0..)
        .map(|k| (anchor.0 + k * step, anchor.1 + k * step))
        .find(|&(x, y)| !taken.iter().any(|&(tx, ty)| (x - tx).abs() < step && (y - ty).abs() < step))
        .expect("the candidate walks away from a finite set of occupied slots")
}

/// Spec §9: a thumbnail whose saved connector is not connected goes to the
/// primary output — the first one we know of — at the same x/y. `None`
/// when no output is connected at all.
pub fn resolve_output<'a>(saved: &str, connected: &'a [String]) -> Option<&'a str> {
    connected
        .iter()
        .find(|o| o.as_str() == saved)
        .or_else(|| connected.first())
        .map(String::as_str)
}

/// Where a live thumbnail goes when a layout is applied, and whether its
/// layer surface has to be destroyed and recreated: a layer surface is
/// bound to one output for life, so a saved position on another output is
/// a recreate, not a `set_margin`.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub pinned: bool,
    pub recreate: bool,
}

pub fn placement(saved: &ThumbPos, connected: &[String], current_output: &str) -> Option<Placement> {
    let output = resolve_output(&saved.output, connected)?;
    Some(Placement {
        recreate: output != current_output,
        output: output.to_string(),
        x: saved.x,
        y: saved.y,
        pinned: saved.pinned,
    })
}

/// Validate a layout name as a file stem; `Ok` is the trimmed name. The
/// error text is what the IPC `layout` request sends back after `err `.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("layout name is empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("layout name is longer than {MAX_NAME} characters"));
    }
    if name.contains(['/', '\\']) {
        return Err("layout name cannot contain / or \\".into());
    }
    if name.chars().any(char::is_control) {
        return Err("layout name cannot contain control characters".into());
    }
    if name == "." || name == ".." {
        return Err("layout name cannot be . or ..".into());
    }
    if name.eq_ignore_ascii_case(CURRENT) {
        return Err(format!("{CURRENT:?} is reserved for the auto-saved layout"));
    }
    Ok(name.to_string())
}

pub fn layouts_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("yutani").join("layouts")
}

pub fn current_path() -> PathBuf {
    layouts_dir().join("current.ron")
}

pub fn named_path_in(dir: &Path, name: &str) -> Result<PathBuf, String> {
    Ok(dir.join(format!("{}.ron", validate_name(name)?)))
}

pub fn named_path(name: &str) -> Result<PathBuf, String> {
    named_path_in(&layouts_dir(), name)
}

/// Saved layout names, sorted case-insensitively. `current` is not one of
/// them, and neither is any file whose stem would fail `validate_name`.
pub fn list_names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| validate_name(s).is_ok_and(|v| v == *s))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn list_names() -> Vec<String> {
    list_names_in(&layouts_dir())
}

pub fn delete_named_in(dir: &Path, name: &str) -> Result<(), String> {
    let path = named_path_in(dir, name)?;
    std::fs::remove_file(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("no such layout {name}"),
        _ => format!("cannot delete {}: {e}", path.display()),
    })
}

pub fn delete_named(name: &str) -> Result<(), String> {
    delete_named_in(&layouts_dir(), name)
}

/// Rename a saved layout, refusing to overwrite an existing one.
pub fn rename_named_in(dir: &Path, from: &str, to: &str) -> Result<(), String> {
    let from_path = named_path_in(dir, from)?;
    let to_path = named_path_in(dir, to)?;
    if from_path == to_path {
        return Ok(());
    }
    if !from_path.exists() {
        return Err(format!("no such layout {from}"));
    }
    if to_path.exists() {
        return Err(format!("layout {} already exists", validate_name(to)?));
    }
    std::fs::rename(&from_path, &to_path).map_err(|e| format!("cannot rename {}: {e}", from_path.display()))
}

pub fn rename_named(from: &str, to: &str) -> Result<(), String> {
    rename_named_in(&layouts_dir(), from, to)
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
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        super::write_atomic(path, &text)?;
        Ok(())
    }

    /// Read a named layout. `Err` is the text the IPC `layout` request
    /// sends back after `err `.
    pub fn load_named_in(dir: &Path, name: &str) -> Result<Layout, String> {
        let path = named_path_in(dir, name)?;
        let text = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("no such layout {name}"),
            _ => format!("cannot read {}: {e}", path.display()),
        })?;
        ron::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))
    }

    pub fn load_named(name: &str) -> Result<Layout, String> {
        Self::load_named_in(&layouts_dir(), name)
    }

    pub fn save_named_in(&self, dir: &Path, name: &str) -> Result<(), String> {
        let path = named_path_in(dir, name)?;
        self.save_to(&path).map_err(|e| format!("cannot save {}: {e:#}", path.display()))
    }

    pub fn save_named(&self, name: &str) -> Result<(), String> {
        self.save_named_in(&layouts_dir(), name)
    }
}
```

In `src/model/config.rs`, replace the body of `save_to` and add the strict loader, inside `impl Config`:

```rust
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        super::write_atomic(path, &text)?;
        Ok(())
    }

    /// Strict load, for the settings window's "is the user's file broken?"
    /// check: `Ok(None)` = no file yet (defaults are in force and saving is
    /// fine), `Ok(Some(config))` = it parsed (and was validated), `Err` =
    /// it exists but cannot be read or parsed, in which case nothing may
    /// overwrite it (spec §9/§10).
    pub fn try_load_from(path: &Path) -> Result<Option<Config>, String> {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
            Ok(text) => ron::from_str::<Config>(&text)
                .map(|c| Some(c.validate()))
                .map_err(|e| format!("cannot parse {}: {e}", path.display())),
        }
    }

    pub fn try_load() -> Result<Option<Config>, String> {
        Self::try_load_from(&config_path())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -q model::layout 2>&1 | tail -3     # test result: ok. 17 passed (7 existing + 10 new)
cargo test -q model::config 2>&1 | tail -3     # test result: ok. the existing count + 1
cargo build -q 2>&1 | tail -5                  # no output
```
The whole suite gains **+11** (9 in `model::layout`, 1 in `model::config`, plus `saving_is_atomic…`): `cargo test -q 2>&1 | grep -h 'test result' | awk '{s+=$4} END {print s}'`.

- [ ] **Step 5: Commit**

```bash
git add src/model
git commit -m "feat(layout): order, new_client_anchor, named layout files and atomic writes

current.ron gains the two fields spec §9 always described; layouts are now
named files under ~/.config/yutani/layouts with validated stems, and both
config.ron and every layout file are written tmp+rename so a crash or a
concurrent reader never sees half a file. Adds the pure output-fallback
rule (§9) and the new-client stacking rule (§4) the daemon needs next.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 2: The daemon writes `order`/anchor, honours the saved output, and has one ordering rule

**Files:**
- Modify: `src/ui/rules.rs`, `src/ui/mod.rs`, `src/ui/dock.rs`, `src/ui/thumbnail.rs`
- Test: the `mod tests` block in `src/ui/rules.rs`

**Interfaces:**
- Consumes (Task 1): `layout::{STACK_STEP, MAX_ORDER, derive_anchor, merge_order, stacked_position, resolve_output}`, `Layout { thumbs, order, new_client_anchor }`, `Anchor { output, x, y }`.
- Produces:
  ```rust
  // src/ui/rules.rs
  pub fn dock_rank(order: &[String], label: &str) -> (usize, String);
  pub fn focus_order<H>(mode: Mode, order: &[String], items: Vec<FocusItem<H>>) -> Vec<H>;   // `dock_order` is gone
  // src/ui/mod.rs (private to the binary's `ui` module tree)
  fn ordered(&self, mode: Mode, keep: impl Fn(&Handle, &Client) -> bool) -> Vec<Handle>;
  fn output_for_thumb(&self, handle: &Handle) -> Option<WlOutput>;
  fn output_name_of(&self, handle: &Handle) -> Option<String>;   // now takes a Handle, not a &Client
  fn layout_order_names(&self) -> Vec<String>;
  fn save_current_layout(&mut self);
  ```

- [ ] **Step 1: Write the failing tests**

In `src/ui/rules.rs`'s `mod tests`, **replace** `dock_focus_order_is_by_label_case_insensitive_and_stable` and `floating_focus_order_is_output_then_row_then_column` with these five tests (the other tests stay untouched):

```rust
    #[test]
    fn dock_rank_matches_by_character_name_case_insensitively() {
        let order = ["Aria Vex".to_string()];
        assert_eq!(dock_rank(&order, "aria vex"), (0, "aria vex".to_string()));
        assert_eq!(dock_rank(&order, "Kel"), (1, "kel".to_string()));
        assert_eq!(dock_rank(&[], "Kel"), (0, "kel".to_string()));
    }

    #[test]
    fn dock_order_is_by_label_when_the_layout_records_no_order() {
        let items = vec![item(1, "kel", "DP-1", 0, 0), item(2, "Aria", "DP-1", 0, 0), item(3, "Kel", "DP-2", 0, 0)];
        assert_eq!(focus_order(Mode::Dock, &[], items), vec![2, 1, 3]);
    }

    #[test]
    fn a_recorded_order_comes_first_and_the_rest_follow_by_label() {
        let order = ["Kel".to_string(), "Zoe".to_string()];
        let items = vec![
            item(1, "Aria", "DP-1", 0, 0),
            item(2, "Zoe", "DP-1", 0, 0),
            item(3, "Kel", "DP-1", 0, 0),
            item(4, "bob", "DP-1", 0, 0),
        ];
        assert_eq!(focus_order(Mode::Dock, &order, items), vec![3, 2, 1, 4]);
    }

    #[test]
    fn floating_focus_order_is_output_then_row_then_column_and_ignores_the_recorded_order() {
        let order = ["z".to_string()];
        let items = vec![
            item(1, "z", "DP-2", 10, 10),
            item(2, "y", "DP-1", 500, 40),
            item(3, "x", "DP-1", 40, 40),
            item(4, "w", "DP-1", 40, 400),
        ];
        assert_eq!(focus_order(Mode::Floating, &order, items), vec![3, 2, 4, 1]);
    }

    #[test]
    fn a_name_the_order_lists_but_nobody_is_playing_does_not_disturb_the_rest() {
        // A logged-out character keeps its slot in `order`; the live ones
        // still sort among themselves in that same relative order.
        let order = ["Zoe".to_string(), "Kel".to_string(), "Aria".to_string()];
        let items = vec![item(1, "Aria", "DP-1", 0, 0), item(2, "Kel", "DP-1", 0, 0)];
        assert_eq!(focus_order(Mode::Dock, &order, items), vec![2, 1]);
    }
```

In `src/ui/thumbnail.rs`'s `mod tests`, **delete** the whole `next_free_x_fills_the_first_gap` test.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -q rules:: 2>&1 | tail -20`
Expected: FAIL to compile — `error[E0425]: cannot find function 'dock_rank' in this scope` and `error[E0061]: this function takes 3 arguments but 2 arguments were supplied` for every `focus_order` call.

- [ ] **Step 3: Write the implementation**

In `src/ui/rules.rs`, **delete** `dock_order` entirely and replace `focus_order` with:

```rust
/// Rank used for dock placement, `focus <n>` and the `order` list saved in
/// `current.ron` (spec §7 and §9): a character the layout's `order` names
/// ranks by its place in that list; anyone else follows, by label,
/// case-insensitively. One comparator, so the dock arrangement and the
/// focus order can never disagree.
pub fn dock_rank(order: &[String], label: &str) -> (usize, String) {
    let index = order.iter().position(|n| n.eq_ignore_ascii_case(label)).unwrap_or(order.len());
    (index, label.to_lowercase())
}

/// Layout order used by `focus <n>`, `next`, `prev` and the dock (spec
/// §7): dock mode by `dock_rank`; floating by (output, y, x). Stable for
/// ties, so equal ranks keep their existing relative order.
pub fn focus_order<H>(mode: Mode, order: &[String], mut items: Vec<FocusItem<H>>) -> Vec<H> {
    match mode {
        Mode::Dock => items.sort_by(|a, b| dock_rank(order, &a.label).cmp(&dock_rank(order, &b.label))),
        Mode::Floating => items.sort_by(|a, b| {
            (a.output.as_str(), a.position.1, a.position.0).cmp(&(b.output.as_str(), b.position.1, b.position.0))
        }),
    }
    items.into_iter().map(|i| i.handle).collect()
}
```
(`sort_by` is the stable sort; `sort_by_cached_key` is **not** stable and would break the tie tests. The `H: Clone` bound the old signature carried is gone — `into_iter` never needed it. `use crate::backend::Handle;` at the top of the file is now unused: delete that import line.)

In `src/ui/thumbnail.rs`, **delete** `next_free_x` (the whole function and its doc comment).

In `src/ui/mod.rs`, replace `fn focus_order` with:

```rust
    /// Layout order (spec §7) of every client for which `keep` is true,
    /// honouring the layout's recorded `order` in dock mode.
    fn ordered(&self, mode: Mode, keep: impl Fn(&Handle, &Client) -> bool) -> Vec<Handle> {
        let items = self
            .clients
            .iter()
            .filter(|(h, c)| keep(h, c))
            .map(|(h, c)| rules::FocusItem {
                handle: h.clone(),
                label: c.info.login.label().to_string(),
                output: self.output_name_of(h).unwrap_or_default(),
                position: c.position,
            })
            .collect();
        rules::focus_order(mode, &self.layout.order, items)
    }

    /// Layout order of every known client (spec §7).
    fn focus_order(&self) -> Vec<Handle> {
        self.ordered(self.config.mode, |_, _| true)
    }

    /// Character names of every known client in layout order — the `order`
    /// list `current.ron` records (spec §9).
    fn layout_order_names(&self) -> Vec<String> {
        self.focus_order()
            .iter()
            .filter_map(|h| match &self.clients.get(h)?.info.login {
                Login::LoggedIn(name) => Some(name.clone()),
                Login::LoggingIn => None,
            })
            .collect()
    }

    /// Write `current.ron` — spec §9 auto-saves it on every drag, pin or
    /// order change — refreshing the recorded `order` and
    /// `new_client_anchor` first.
    fn save_current_layout(&mut self) {
        let live = self.layout_order_names();
        let order = layout::merge_order(&live, &self.layout.order, layout::MAX_ORDER);
        self.layout.order = order;
        self.layout.new_client_anchor = layout::derive_anchor(&self.layout.thumbs);
        if let Err(e) = self.layout.save() {
            tracing::warn!("cannot save layout: {e:#}");
        }
    }
```

Replace `fn next_position` with:

```rust
    /// Where a thumbnail whose character is not known yet goes (spec §4):
    /// the layout's `new_client_anchor`, then `STACK_STEP` px down-right
    /// per occupied slot — occupied being any shown thumbnail **and** any
    /// saved position, so a new client no longer lands on top of a
    /// character's saved spot.
    fn next_position(&self) -> (i32, i32) {
        let anchor = &self.layout.new_client_anchor;
        let mut taken: Vec<(i32, i32)> =
            self.clients.values().filter(|c| c.surface.is_some()).map(|c| c.position).collect();
        taken.extend(self.layout.thumbs.values().map(|t| (t.x, t.y)));
        layout::stacked_position((anchor.x, anchor.y), &taken, layout::STACK_STEP)
    }
```

Replace `fn output_name_of` with the handle-taking pair (keep `fn output_for` exactly as it is — `output_for_thumb` calls it):

```rust
    /// The output this client's thumbnail belongs on. In floating mode the
    /// layout decides: the character's saved connector, else the anchor's,
    /// resolved against what is actually connected — an unplugged
    /// connector falls back to the primary output at the same x/y (spec
    /// §9). Otherwise (dock mode, or nothing saved) the output the
    /// client's own window is on, else the first output we know of.
    fn output_for_thumb(&self, handle: &Handle) -> Option<WlOutput> {
        let client = self.clients.get(handle)?;
        if self.config.mode == Mode::Floating {
            let saved = match &client.info.login {
                Login::LoggedIn(name) => self.layout.thumbs.get(name).map(|t| t.output.as_str()),
                Login::LoggingIn => None,
            };
            let anchor = Some(self.layout.new_client_anchor.output.as_str()).filter(|o| !o.is_empty());
            if let Some(wanted) = saved.or(anchor) {
                let connected: Vec<String> = self.outputs.iter().map(|o| o.name.clone()).collect();
                if let Some(resolved) = layout::resolve_output(wanted, &connected) {
                    return self.outputs.iter().find(|o| o.name == resolved).map(|o| o.handle.clone());
                }
            }
        }
        self.output_for(&client.info)
    }

    fn output_name_of(&self, handle: &Handle) -> Option<String> {
        self.output_for_thumb(handle)
            .and_then(|o| self.outputs.iter().find(|k| k.handle == o).map(|k| k.name.clone()))
    }
```

Four call sites change in `src/ui/mod.rs`:

1. In `create_surface`, `let Some(output) = self.output_for(&client.info) else {` becomes:
```rust
        let Some(output) = self.output_for_thumb(handle) else {
```
2. In `persist_position`, `let Some(output) = self.output_name_of(client) else { return };` becomes `let Some(output) = self.output_name_of(handle) else { return };` — and because that line now borrows `self` while `client` is still borrowed, reorder the body so the name is cloned first. The whole function becomes:
```rust
    /// Remember this client's position (and pin state) under its character
    /// name, then rewrite `current.ron`. Floating only: dock positions come
    /// from the layout policy.
    fn persist_position(&mut self, handle: &Handle) {
        if self.config.mode == Mode::Dock {
            return;
        }
        let Some(client) = self.clients.get(handle) else { return };
        let Login::LoggedIn(name) = client.info.login.clone() else {
            tracing::debug!("position not saved: character name not resolved yet");
            return;
        };
        let (position, pinned) = (client.position, client.pinned);
        let Some(output) = self.output_name_of(handle) else { return };
        self.layout
            .thumbs
            .insert(name, ThumbPos { output, x: position.0, y: position.1, pinned });
        self.save_current_layout();
    }
```
3. In `refloat_surfaces`, the first two lines become:
```rust
        let handles = self.ordered(Mode::Dock, |_, c| c.surface.is_some());
```
(deleting the `let with_surface = …;` line above it).
4. In `on_backend`, the `became_named` branch and `ClientRemoved` both save, because the order changed:
```rust
                if became_named {
                    // A new character name changes the layout order, and a
                    // saved position must apply even if the surface was just
                    // created (or doesn't exist yet): `apply_saved_position`
                    // updates position/pinned regardless, and is a no-op on
                    // margin if there's no surface.
                    self.save_current_layout();
                    Task::batch([reconciled, self.apply_saved_position(&handle)])
                } else {
                    reconciled
                }
```
```rust
            Event::ClientRemoved(handle) => {
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                // The layout order changed; the saved position stays, so the
                // character comes back to the same spot next launch.
                self.save_current_layout();
                // Dock mode: its neighbours close the gap.
                Task::batch([task, self.relayout_dock()])
            }
```

In `src/ui/dock.rs`, replace `dock_order_for` and the output lookup in `dock_position_of`:

```rust
    /// Dock mode: the clients on `output` that have a surface, in dock order.
    pub(super) fn dock_order_for(&self, output: &WlOutput) -> Vec<Handle> {
        self.ordered(super::Mode::Dock, |h, c| {
            c.surface.is_some() && self.output_for_thumb(h).as_ref() == Some(output)
        })
    }
```
and in `dock_position_of`:
```rust
        let output = self
            .output_for_thumb(handle)
            .and_then(|o| self.outputs.iter().find(|k| k.handle == o));
```
(`use super::{App, Msg, Output, rules};` — `rules` is now unused in this file; change the import to `use super::{App, Msg, Output};`.)

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -q rules:: 2>&1 | tail -3       # test result: ok. 10 passed (7 existing - 2 replaced + 5 new)
cargo test -q thumbnail:: 2>&1 | tail -3   # test result: ok. 3 passed (4 existing - 1 deleted)
cargo build -q 2>&1 | tail -5              # no output, no warnings
cargo test -q 2>&1 | grep -h 'test result' | awk '{s+=$4} END {print s}'
```
Suite delta on Task 1's total: **+3** (five new rules tests replacing two, minus the deleted `next_free_x` test).

- [ ] **Step 5: Commit**

```bash
git add src/ui/rules.rs src/ui/mod.rs src/ui/dock.rs src/ui/thumbnail.rs
git commit -m "feat(layout): record order/anchor, place thumbnails on the saved output

current.ron is now written with `order` and `new_client_anchor` on every
drag, pin and order change, and a saved thumbnail is created on the output
its ThumbPos names (primary output at the same x/y when that connector is
gone, spec §9) instead of wherever the client's window happens to be. A new
client stacks 24 px down-right of the anchor past occupied slots rather
than landing on a saved position. dock_order and focus_order's Dock arm are
one comparator (rules::dock_rank) that honours the recorded order.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 3: `yutani layout <name>` and `yutani layouts` — IPC, CLI, and applying a layout live

**Files:**
- Modify: `src/ipc.rs`, `src/cli.rs`, `src/main.rs`, `src/ui/mod.rs`
- Test: the `mod tests` block in `src/ipc.rs`

**Interfaces:**
- Consumes: plan A's `ipc::Response::OkData(String)` and `handle_request`'s result type `(Result<Option<String>, String>, Task<cosmic::Action<Msg>>)`; Task 1's `Layout::load_named`, `layout::{list_names, placement}`; Task 2's `output_name_of`, `save_current_layout`.
- Produces:
  ```rust
  // src/ipc.rs
  pub enum Request { Focus(usize), Next, Prev, Show, Hide, Toggle, Layout(String), Layouts, Settings, Status, TunnelConnect, TunnelDisconnect, Quit }
  // src/cli.rs
  fn exchange(request: &Request) -> Result<Response, String>;
  pub fn layouts() -> ExitCode;
  // src/ui/mod.rs
  pub struct Client { /* … */ pub output: String }   // connector the surface was created on
  fn apply_layout(&mut self, name: &str) -> Result<Task<cosmic::Action<Msg>>, String>;
  fn reposition_to_layout(&mut self) -> Task<cosmic::Action<Msg>>;
  ```

- [ ] **Step 1: Write the failing test**

Add to `src/ipc.rs`'s `mod tests`:

```rust
    #[test]
    fn layouts_is_its_own_command_not_a_layout_named_s() {
        assert_eq!(Request::parse("layouts"), Ok(Request::Layouts));
        assert_eq!(Request::parse("layouts now").unwrap_err(), "layouts takes no argument");
        assert_eq!(Request::parse("layout current"), Ok(Request::Layout("current".into())));
        assert_eq!(Request::Layouts.to_line(), "layouts\n");
    }
```
and add `Request::Layouts` to the array in `request_lines_round_trip`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -q ipc:: 2>&1 | tail -10`
Expected: FAIL to compile — `error[E0599]: no variant or associated item named 'Layouts' found for enum 'Request'`.

- [ ] **Step 3: Write the implementation**

`src/ipc.rs`: add `Layouts,` to `Request` directly after `Layout(String)`; in `parse`, add above the `("", _)` arm:
```rust
            ("layouts", None) => Ok(Request::Layouts),
```
add `"layouts"` to the takes-no-argument list so the arm reads:
```rust
            (cmd, Some(_))
                if matches!(cmd, "next" | "prev" | "show" | "hide" | "toggle" | "layouts" | "settings" | "status" | "quit") =>
            {
                Err(format!("{cmd} takes no argument"))
            }
```
and in `to_line`, `Request::Layouts => "layouts\n".into(),`.

`src/cli.rs`: replace `pub fn send` with the split below and add `layouts` (the rest of the file, `TIMEOUT`/`is_running`/imports, is unchanged):

```rust
/// Send one request and return the reply. `Err` is a message already
/// formatted for stderr.
fn exchange(request: &Request) -> Result<Response, String> {
    let path = socket_path();
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            return Err("yutani is not running".into());
        }
        Err(err) => return Err(format!("yutani: cannot connect to {}: {err}", path.display())),
    };
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and(stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|err| format!("yutani: socket setup failed: {err}"))?;
    let mut writer = &stream;
    writer.write_all(request.to_line().as_bytes()).map_err(|err| format!("yutani: send failed: {err}"))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|err| format!("yutani: no reply: {err}"))?;
    Ok(Response::parse(&line))
}

/// Send `request`; print the reply; map it to an exit code.
pub fn send(request: &Request) -> ExitCode {
    match exchange(request) {
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Ok(Response::Ok) => ExitCode::SUCCESS,
        Ok(Response::OkData(data)) => {
            println!("{data}");
            ExitCode::SUCCESS
        }
        Ok(Response::Err(msg)) => {
            eprintln!("yutani: {msg}");
            ExitCode::from(1)
        }
    }
}

/// `yutani layouts`: the saved layout names, one per line (and nothing at
/// all when none are saved).
pub fn layouts() -> ExitCode {
    match exchange(&Request::Layouts) {
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Ok(Response::OkData(json)) => match serde_json::from_str::<Vec<String>>(&json) {
            Ok(names) => {
                for name in names {
                    println!("{name}");
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("yutani: cannot read the layout list: {err}");
                ExitCode::from(1)
            }
        },
        Ok(Response::Ok) => ExitCode::SUCCESS,
        Ok(Response::Err(msg)) => {
            eprintln!("yutani: {msg}");
            ExitCode::from(1)
        }
    }
}
```

`src/main.rs`: add to `enum Command`, after `Toggle`:
```rust
    /// Apply a saved layout by name
    Layout {
        /// Name of a layout in ~/.config/yutani/layouts
        name: String,
    },
    /// List the saved layouts, one per line
    Layouts,
```
and the match arms, after the `Toggle` one:
```rust
        Some(Command::Layout { name }) => Ok(cli::send(&ipc::Request::Layout(name))),
        Some(Command::Layouts) => Ok(cli::layouts()),
```

`src/ui/mod.rs`:

(a) `Client` gains the connector it was created on — add the field at the end of the struct:
```rust
    /// Connector name of the output this client's surface was created on;
    /// empty when it has none. Applying a layout compares this with the
    /// layout's output: a layer surface is bound to one output for life, so
    /// a different one means destroy + recreate, not `set_margin`.
    pub output: String,
```
and `output: String::new(),` to the `Client { … }` literal in `on_backend`'s `or_insert_with`.

(b) In `create_surface`, right after the `let Some(output) = self.output_for_thumb(handle) else { … };` block, remember its name; the assignment goes next to the other `client.*` writes:
```rust
        let output_name =
            self.outputs.iter().find(|o| o.handle == output).map(|o| o.name.clone()).unwrap_or_default();
```
and in the `let client = self.clients.get_mut(handle).unwrap();` block below it, add:
```rust
        client.output = output_name;
```

(c) In `forget_surface`, inside the `if let Some(c) = self.clients.get_mut(handle)` block, add:
```rust
            c.output.clear();
```

(d) Add the two layout methods to `impl App` (next to `apply_saved_position`):
```rust
    /// Apply a named layout (spec §9): copy it to `current.ron` and move
    /// every live thumbnail to the spot it names. `Err` is the text the IPC
    /// reply sends after `err `.
    fn apply_layout(&mut self, name: &str) -> Result<Task<cosmic::Action<Msg>>, String> {
        let loaded = Layout::load_named(name)?;
        tracing::info!(name, thumbs = loaded.thumbs.len(), "applying layout");
        self.layout = loaded;
        if let Err(e) = self.layout.save() {
            tracing::warn!("cannot copy the layout to current.ron: {e:#}");
        }
        Ok(self.reposition_to_layout())
    }

    /// Move every live thumbnail to where `self.layout` puts it. A
    /// thumbnail whose output changed is destroyed and left to
    /// `reconcile_surfaces`, which recreates it — `output_for_thumb` now
    /// resolves to the layout's output. In dock mode positions are a
    /// policy, so only the arrangement (the recorded `order`) changes.
    fn reposition_to_layout(&mut self) -> Task<cosmic::Action<Msg>> {
        if self.config.mode == Mode::Dock {
            return self.relayout_dock();
        }
        let connected: Vec<String> = self.outputs.iter().map(|o| o.name.clone()).collect();
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        let mut tasks = Vec::new();
        for h in handles {
            let Some(client) = self.clients.get(&h) else { continue };
            let Login::LoggedIn(name) = client.info.login.clone() else { continue };
            let current = client.output.clone();
            let Some(saved) = self.layout.thumbs.get(&name).cloned() else { continue };
            let Some(p) = layout::placement(&saved, &connected, &current) else { continue };
            let surface = {
                let c = self.clients.get_mut(&h).unwrap();
                c.position = (p.x, p.y);
                c.pinned = p.pinned;
                c.placed = true;
                c.surface
            };
            match surface {
                Some(_) if p.recreate => tasks.push(self.destroy_surface(&h)),
                // A drag canvas keeps its margin; `leave_canvas` applies the
                // new position at drag end.
                Some(id) if !self.in_canvas(id) => tasks.push(set_margin(id, p.y, 0, 0, p.x)),
                _ => {}
            }
        }
        tasks.push(self.reconcile_surfaces());
        Task::batch(tasks)
    }
```

(e) In `handle_request`, replace the `Request::Layout(_) | Request::Settings` stub arm with:
```rust
            Request::Layout(name) => match self.apply_layout(name) {
                Ok(task) => (Ok(None), task),
                Err(msg) => (Err(msg), Task::none()),
            },
            Request::Layouts => match serde_json::to_string(&layout::list_names()) {
                Ok(json) => (Ok(Some(json)), Task::none()),
                Err(e) => (Err(format!("layouts: {e}")), Task::none()),
            },
            // Task 4 replaces this arm with the settings window.
            Request::Settings => {
                (Err("not supported yet (the settings window lands in the next commit)".into()), Task::none())
            }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -q ipc:: 2>&1 | tail -3    # test result: ok. 6 passed
cargo build -q 2>&1 | tail -5         # no output, no warnings
```
Suite delta on Task 2's total: **+1**.

- [ ] **Step 5: Check the CLI surface without a compositor**

```bash
./target/debug/yutani layout 2>&1 | head -3        # clap: "error: the following required arguments were not provided: <NAME>"
./target/debug/yutani layouts; echo "exit=$?"      # "yutani is not running", exit=1
./target/debug/yutani layout pvp; echo "exit=$?"   # "yutani is not running", exit=1
```

- [ ] **Step 6: Commit**

```bash
git add src/ipc.rs src/cli.rs src/main.rs src/ui/mod.rs
git commit -m "feat(ipc): layout <name> applies a saved layout; layouts lists them

Replaces plan 4's stub: `layout` loads ~/.config/yutani/layouts/<name>.ron,
copies it to current.ron (spec §9) and moves every live thumbnail there,
recreating the surfaces whose output changed. `layouts` replies ok <json>
and `yutani layouts` prints one name per line.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 4: The settings window — lifecycle, the broken-config guard, and the Display page

**Files:**
- Create: `src/ui/settings.rs`
- Modify: `src/ui/mod.rs`, `src/main.rs`
- Test: the `mod tests` block in `src/ui/settings.rs`

**Interfaces:**
- Consumes: Task 1's `Config::try_load`, `model::write_atomic` (through `Config::save`); Task 3's `handle_request` arms; the existing `App::apply_config(Config) -> Task<cosmic::Action<Msg>>`.
- Produces:
  ```rust
  // src/ui/settings.rs
  pub struct State { pub window: SurfaceId, pub config_error: Option<String>,
                     pub active_border_field: String, pub inactive_border_field: String, pub note: Option<String> }
  impl State { pub fn new(window: SurfaceId, config: &Config) -> Self; pub fn refresh(&mut self); }
  pub enum Msg { Page/* Task 5 */, Opened, Raise(Option<String>), Close, Closed, Drag, Recheck, Commit,
                 ThumbWidth(u32), Zoom(f32), BorderPx(u32), CornerRadius(u32), ShowNames(bool),
                 ActiveBorder(String), InactiveBorder(String) }
  pub fn window_settings(app_id: &str) -> cosmic::iced::window::Settings;
  pub fn apply_config_field(config: &mut Config, msg: &Msg) -> Result<bool, String>;
  pub fn is_live_only(msg: &Msg) -> bool;
  pub fn parse_optional_color(text: &str) -> Result<Option<String>, String>;
  pub fn parse_required_color(text: &str) -> Result<String, String>;
  pub fn view<'a>(state: &'a State, config: &'a Config, focused: bool) -> Element<'a, super::Msg>;
  // src/ui/mod.rs
  pub enum Msg { /* … */ Settings(settings::Msg) }
  pub struct App { /* … */ pub settings: Option<settings::State> }
  fn open_settings(&mut self) -> Task<cosmic::Action<Msg>>;
  fn on_settings(&mut self, msg: settings::Msg) -> Task<cosmic::Action<Msg>>;
  fn save_config(&mut self);
  fn settings_note(&mut self, note: String);
  fn settings_note_clear(&mut self);
  ```

**libcosmic API notes (verified in the pinned checkout `a401af8`; do not substitute other signatures):**
- `cosmic::iced::window::open(Settings) -> (Id, Task<Id>)` (`iced/runtime/src/window.rs:303`) is the only way to open a real toplevel at this rev — there is no `SctkWindowSettings` and no `commands::window` module. It works with the `wayland` feature (`examples/multi-window` builds with exactly Yutani's feature set).
- `cosmic::iced::window::{close, drag}` (`iced/runtime/src/window.rs:315`, `:330`), both `fn(Id) -> Task<T>`.
- `cosmic::iced::window::Settings` fields (`iced/core/src/window/settings.rs:34`) and `PlatformSpecific { application_id, override_redirect }` (`…/settings/linux.rs:5`).
- Because `no_main_window(true)` makes `Core::main_window_id()` return `None`, **every** surface — layer surfaces and this window alike — is rendered through `Application::view_window(id)` (`src/app/cosmic.rs:706`). `App::view` stays `unreachable!()`.
- `Application::on_close_requested(&self, id) -> Option<Self::Message>` (`src/app/mod.rs:427`) actually fires on `window::Event::Closed` (`src/app/cosmic.rs:558`), and `exit_on_close(false)` means it never exits the app.
- `ApplicationExt::set_window_title(&mut self, title: String, id: window::Id) -> Task<Self::Message>` (`src/app/mod.rs:596`) — the multi-window feature's two-argument form.
- `cosmic::iced::window::gain_focus` is a **no-op on Wayland** (winit's `focus_window` is empty). Raising is xdg-activation: `cosmic::iced::platform_specific::shell::commands::activation::{request_token(Option<String>, Option<Id>) -> Task<Option<String>>, activate(Id, String) -> Task<Message>}` — the same pair libcosmic itself uses at `src/app/cosmic.rs:1168`.
- Widgets: `settings::section() -> Section` (needs `.into()` to become an `Element`), `settings::item(title, control)`, `settings::item_row(Vec<Element>)`, `settings::view_column(Vec<Element>)` (there is **no** `view_section` at this rev); `slider(range, value, on_change).step(..).on_release(Message).width(..)`; `spin_button(label, value, step, min, max, on_press)` — **6 arguments**, because the 7th (`name`) exists only under the `a11y` feature, which Yutani does not enable; `toggler(is_checked).on_toggle(f)` — it takes **no** label argument; `text_input(placeholder, value).on_input(f)`, whose `.on_submit` takes a **closure** `Fn(String) -> Message`, not a bare message; `button::{standard, suggested, destructive, text}(label).on_press(msg)` / `.on_press_maybe(Option<msg>)`; `text::{body, caption}`; `header_bar().title(..).on_close(msg).on_drag(msg).focused(bool)`; `column::with_children(Vec<Element>)`.

- [ ] **Step 1: Write the failing test**

Create `src/ui/settings.rs` containing **only** this test module for now (the implementation follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_fields_accept_hex_and_clearing_the_active_one_means_theme_accent() {
        assert_eq!(parse_optional_color("  "), Ok(None));
        assert_eq!(parse_optional_color(" #ff8800 "), Ok(Some("#ff8800".to_string())));
        assert_eq!(parse_optional_color("#00000080"), Ok(Some("#00000080".to_string())));
        assert!(parse_optional_color("red").is_err());
        assert_eq!(parse_required_color("#404040"), Ok("#404040".to_string()));
        assert!(parse_required_color("").is_err());
        assert!(parse_required_color("#gg0000").is_err());
    }

    #[test]
    fn display_fields_land_in_the_config_and_say_whether_it_changed() {
        let mut c = Config::default();
        assert_eq!(apply_config_field(&mut c, &Msg::ThumbWidth(320)), Ok(true));
        assert_eq!(c.thumb_width, 320);
        // The same value again is not a change, so nothing is applied or written.
        assert_eq!(apply_config_field(&mut c, &Msg::ThumbWidth(320)), Ok(false));
        // Out-of-range values from a misbehaving control are clamped, never stored raw.
        assert_eq!(apply_config_field(&mut c, &Msg::ThumbWidth(9999)), Ok(true));
        assert_eq!(c.thumb_width, 1600);
        assert_eq!(apply_config_field(&mut c, &Msg::BorderPx(99)), Ok(true));
        assert_eq!(c.border_px, 16);
        assert_eq!(apply_config_field(&mut c, &Msg::CornerRadius(99)), Ok(true));
        assert_eq!(c.corner_radius, 64);
        assert_eq!(apply_config_field(&mut c, &Msg::Zoom(9.0)), Ok(true));
        assert_eq!(c.zoom_factor, 4.0);
        assert_eq!(apply_config_field(&mut c, &Msg::ShowNames(false)), Ok(true));
        assert!(!c.show_names);
        assert_eq!(apply_config_field(&mut c, &Msg::ActiveBorder("#ff8800".into())), Ok(true));
        assert_eq!(c.active_border.as_deref(), Some("#ff8800"));
        assert_eq!(apply_config_field(&mut c, &Msg::ActiveBorder(String::new())), Ok(true));
        assert_eq!(c.active_border, None);
        // A bad value is a note, not a write: the config is untouched.
        let before = c.clone();
        assert!(apply_config_field(&mut c, &Msg::InactiveBorder("nope".into())).is_err());
        assert_eq!(c, before);
        // Window-level messages are not config fields.
        assert_eq!(apply_config_field(&mut c, &Msg::Close), Ok(false));
        assert_eq!(apply_config_field(&mut c, &Msg::Commit), Ok(false));
    }

    #[test]
    fn sliders_apply_live_but_only_their_release_writes_the_file() {
        assert!(is_live_only(&Msg::ThumbWidth(300)));
        assert!(is_live_only(&Msg::Zoom(1.5)));
        assert!(!is_live_only(&Msg::BorderPx(2)));
        assert!(!is_live_only(&Msg::ShowNames(false)));
        assert!(!is_live_only(&Msg::ActiveBorder("#ff8800".into())));
    }
}
```
Add `pub mod settings;` to the module list at the top of `src/ui/mod.rs` (keeping it alphabetical: `config_watch, dock, ipc, pointer, rules, settings, thumbnail`).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -q ui::settings 2>&1 | tail -10`
Expected: FAIL to compile — `error[E0432]: unresolved import 'super'` / `cannot find function 'parse_optional_color' in this scope` / `cannot find type 'Msg' in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend all of this to `src/ui/settings.rs`, **above** the test module from Step 1:

```rust
//! The settings window (spec §6). Opened by `yutani settings` and by the
//! applet's Preferences… row, both through the IPC `settings` request; a
//! second request raises the window that is already open.
//!
//! Every change applies live (`App::apply_config`) and is written back to
//! `config.ron` — except while that file fails to parse, when the window
//! shows what is wrong and writes nothing at all (spec §9: "never overwrite
//! the user's file").

use cosmic::iced::Length;
use cosmic::iced::window::Id as SurfaceId;
use cosmic::widget;
use cosmic::Element;

use crate::model::config::{Config, parse_color};

/// Everything the settings window owns. `None` on `App` while it is closed.
pub struct State {
    pub window: SurfaceId,
    /// Set when `config.ron` exists but does not parse: the window shows
    /// the reason instead of the pages and writes nothing.
    pub config_error: Option<String>,
    /// Text fields are kept as typed, so a half-typed value never reaches
    /// the config; they are seeded when the window opens and are not
    /// re-seeded by a later hand edit (which would eat what is being typed).
    pub active_border_field: String,
    pub inactive_border_field: String,
    /// One line of feedback under the page.
    pub note: Option<String>,
}

impl State {
    pub fn new(window: SurfaceId, config: &Config) -> Self {
        let mut state = Self {
            window,
            config_error: None,
            active_border_field: config.active_border.clone().unwrap_or_default(),
            inactive_border_field: config.inactive_border.clone(),
            note: None,
        };
        state.refresh();
        state
    }

    /// Re-read what lives outside `Config`: whether `config.ron` parses.
    pub fn refresh(&mut self) {
        self.config_error = Config::try_load().err();
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// The window finished opening.
    Opened,
    /// An xdg-activation token for raising the window (`None`: the
    /// compositor refused, so nothing to do).
    Raise(Option<String>),
    /// The header bar's close button.
    Close,
    /// The window is gone.
    Closed,
    /// The header bar is being dragged.
    Drag,
    /// Re-read `config.ron` after the user fixed it.
    Recheck,
    /// A slider was released: write what the drag already applied.
    Commit,
    ThumbWidth(u32),
    Zoom(f32),
    BorderPx(u32),
    CornerRadius(u32),
    ShowNames(bool),
    ActiveBorder(String),
    InactiveBorder(String),
}

/// The settings window: an ordinary xdg-toplevel. Undecorated because
/// libcosmic draws the header bar itself (`widget::header_bar`), and
/// `exit_on_close_request: true` so the compositor's own close gesture
/// closes it and the app only reacts to `Closed`.
pub fn window_settings(app_id: &str) -> cosmic::iced::window::Settings {
    cosmic::iced::window::Settings {
        size: cosmic::iced::Size::new(620.0, 700.0),
        min_size: Some(cosmic::iced::Size::new(420.0, 380.0)),
        resizable: true,
        decorations: false,
        exit_on_close_request: true,
        platform_specific: cosmic::iced::window::PlatformSpecific {
            application_id: app_id.to_string(),
            override_redirect: false,
        },
        ..Default::default()
    }
}

/// `active_border`: empty means "follow the COSMIC theme accent" (`None`);
/// anything else must parse as `#rrggbb[aa]` (spec §9).
pub fn parse_optional_color(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match parse_color(text) {
        Some(_) => Ok(Some(text.to_string())),
        None => Err(format!("{text:?} is not #rrggbb or #rrggbbaa")),
    }
}

/// `inactive_border` has no "follow the theme" option: it must parse.
pub fn parse_required_color(text: &str) -> Result<String, String> {
    let text = text.trim();
    match parse_color(text) {
        Some(_) => Ok(text.to_string()),
        None => Err(format!("{text:?} is not #rrggbb or #rrggbbaa")),
    }
}

/// True for the messages a slider fires continuously while dragging: they
/// apply live but must not write `config.ron` on every pixel. The slider's
/// `on_release` sends `Commit`, which writes once.
pub fn is_live_only(msg: &Msg) -> bool {
    matches!(msg, Msg::ThumbWidth(_) | Msg::Zoom(_))
}

/// Apply one settings change to `config`. `Ok(true)` = the config changed
/// and must be applied live and written; `Ok(false)` = the message was not
/// a config field; `Err` = a message for the note line, nothing changed.
pub fn apply_config_field(config: &mut Config, msg: &Msg) -> Result<bool, String> {
    let before = config.clone();
    match msg {
        Msg::ThumbWidth(v) => config.thumb_width = (*v).clamp(80, 1600),
        Msg::Zoom(v) => config.zoom_factor = (*v).clamp(1.0, 4.0),
        Msg::BorderPx(v) => config.border_px = (*v).min(16),
        Msg::CornerRadius(v) => config.corner_radius = (*v).min(64),
        Msg::ShowNames(v) => config.show_names = *v,
        Msg::ActiveBorder(text) => config.active_border = parse_optional_color(text)?,
        Msg::InactiveBorder(text) => config.inactive_border = parse_required_color(text)?,
        _ => return Ok(false),
    }
    Ok(*config != before)
}

/// The whole window: header bar, the page (or the broken-config notice),
/// and the note line.
pub fn view<'a>(state: &'a State, config: &'a Config, focused: bool) -> Element<'a, super::Msg> {
    let body: Element<'a, Msg> = match &state.config_error {
        Some(error) => broken_config(error),
        None => display_page(state, config),
    };
    let note: Element<'a, Msg> = match &state.note {
        Some(note) => widget::text::caption(note.as_str()).into(),
        None => widget::text::caption(
            "Changes apply at once and are written to ~/.config/yutani/config.ron. \
             Comments and unknown keys in that file are not preserved.",
        )
        .into(),
    };
    let page = widget::container(widget::column::with_children(vec![body, note]).spacing(12))
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill);
    let content: Element<'a, Msg> = widget::container(widget::column::with_children(vec![
        widget::header_bar()
            .title("Yutani Settings")
            .on_close(Msg::Close)
            .on_drag(Msg::Drag)
            .focused(focused)
            .into(),
        page.into(),
    ]))
    .class(cosmic::theme::Container::WindowBackground)
    .width(Length::Fill)
    .height(Length::Fill)
    .into();
    content.map(super::Msg::Settings)
}

/// Shown instead of the pages while `config.ron` does not parse: nothing is
/// editable, because the only safe thing to do with the user's broken file
/// is leave it alone (spec §9/§10).
fn broken_config(error: &str) -> Element<'_, Msg> {
    widget::settings::view_column(vec![
        widget::settings::section()
            .title("config.ron cannot be read")
            .add(widget::text::body(error))
            .add(widget::text::body(
                "Yutani is running with default settings. Nothing here can be changed until the file parses — \
                 it is never overwritten. Fix or delete it, then press Re-check.",
            ))
            .add(widget::settings::item_row(vec![
                widget::button::standard("Re-check").on_press(Msg::Recheck).into(),
            ]))
            .into(),
    ])
    .into()
}

fn display_page<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let width = widget::settings::item(
        format!("Thumbnail width: {} px", config.thumb_width),
        widget::slider(80..=1600, config.thumb_width, Msg::ThumbWidth)
            .step(10u32)
            .on_release(Msg::Commit)
            .width(Length::Fixed(260.0)),
    );
    let zoom = widget::settings::item(
        format!("Hover zoom: {:.1}x", config.zoom_factor),
        widget::slider(1.0..=4.0, config.zoom_factor, Msg::Zoom)
            .step(0.1f32)
            .on_release(Msg::Commit)
            .width(Length::Fixed(260.0)),
    );
    let names =
        widget::settings::item("Show character names", widget::toggler(config.show_names).on_toggle(Msg::ShowNames));
    let border = widget::settings::item(
        "Border width (px)",
        widget::spin_button(config.border_px.to_string(), config.border_px, 1, 0, 16, Msg::BorderPx),
    );
    let radius = widget::settings::item(
        "Corner radius (px)",
        widget::spin_button(config.corner_radius.to_string(), config.corner_radius, 1, 0, 64, Msg::CornerRadius),
    );
    let active = widget::settings::item(
        "Active border colour",
        widget::text_input("theme accent", state.active_border_field.as_str())
            .on_input(Msg::ActiveBorder)
            .width(Length::Fixed(160.0)),
    );
    let inactive = widget::settings::item(
        "Inactive border colour",
        widget::text_input("#404040", state.inactive_border_field.as_str())
            .on_input(Msg::InactiveBorder)
            .width(Length::Fixed(160.0)),
    );
    widget::settings::view_column(vec![
        widget::settings::section().title("Thumbnails").add(width).add(zoom).add(names).into(),
        widget::settings::section().title("Frame").add(border).add(radius).add(active).add(inactive).into(),
    ])
    .into()
}
```

In `src/ui/mod.rs`:

(a) Imports — add next to the existing ones:
```rust
use cosmic::app::ApplicationExt;
use cosmic::iced::platform_specific::shell::commands::activation;
```

(b) `App` gains a field (after `hidden`):
```rust
    /// The settings window while it is open (spec §6).
    pub settings: Option<settings::State>,
```
and `init`'s `App { … }` literal gains `settings: None,`.

(c) `Msg` gains a variant:
```rust
    Settings(settings::Msg),
```

(d) Add to `impl App`:
```rust
    /// Open the settings window, or raise the one already open (spec §6: a
    /// second `settings` request focuses it).
    fn open_settings(&mut self) -> Task<cosmic::Action<Msg>> {
        if let Some(state) = &self.settings {
            // `window::gain_focus` does nothing on Wayland; raising a
            // window is an xdg-activation token handed back to the
            // compositor (the same dance libcosmic does for `Activate`).
            let id = state.window;
            return activation::request_token(Some(Self::APP_ID.to_string()), Some(id))
                .map(|token| cosmic::Action::App(Msg::Settings(settings::Msg::Raise(token))));
        }
        let (id, open) = cosmic::iced::window::open(settings::window_settings(Self::APP_ID));
        self.settings = Some(settings::State::new(id, &self.config));
        let title = self.set_window_title("Yutani Settings".to_string(), id);
        Task::batch([title, open.map(|_| cosmic::Action::App(Msg::Settings(settings::Msg::Opened)))])
    }

    fn on_settings(&mut self, msg: settings::Msg) -> Task<cosmic::Action<Msg>> {
        use settings::Msg as S;
        // The window is gone: only `Closed` still means anything.
        if matches!(msg, S::Closed) {
            self.settings = None;
            return Task::none();
        }
        let Some(window) = self.settings.as_ref().map(|s| s.window) else { return Task::none() };

        // Messages that only move the window around.
        match &msg {
            S::Close => return cosmic::iced::window::close(window),
            S::Drag => return cosmic::iced::window::drag(window),
            S::Raise(Some(token)) => return activation::activate(window, token.clone()),
            _ => {}
        }

        // Messages that only touch the window's own state.
        if let Some(state) = self.settings.as_mut() {
            match &msg {
                S::Opened | S::Raise(None) | S::Recheck => {
                    state.refresh();
                    return Task::none();
                }
                S::ActiveBorder(text) => state.active_border_field = text.clone(),
                S::InactiveBorder(text) => state.inactive_border_field = text.clone(),
                _ => {}
            }
        }

        if matches!(msg, S::Commit) {
            self.save_config();
            return Task::none();
        }

        // A config field: apply live, and write `config.ron` unless this is
        // a slider still being dragged (its release sends `Commit`).
        let mut config = self.config.clone();
        match settings::apply_config_field(&mut config, &msg) {
            Err(note) => {
                self.settings_note(note);
                Task::none()
            }
            Ok(false) => Task::none(),
            Ok(true) => {
                self.settings_note_clear();
                let task = self.apply_config(config);
                if !settings::is_live_only(&msg) {
                    self.save_config();
                }
                task
            }
        }
    }

    fn settings_note(&mut self, note: String) {
        if let Some(state) = self.settings.as_mut() {
            state.note = Some(note);
        }
    }

    fn settings_note_clear(&mut self) {
        if let Some(state) = self.settings.as_mut() {
            state.note = None;
        }
    }

    /// Write `config.ron` (atomically, through `model::write_atomic`) —
    /// unless the file on disk does not parse, in which case it is left
    /// exactly as the user wrote it (spec §9/§10).
    fn save_config(&mut self) {
        if self.settings.as_ref().is_some_and(|s| s.config_error.is_some()) {
            return;
        }
        if let Err(e) = self.config.save() {
            tracing::warn!("cannot save config: {e:#}");
            self.settings_note(format!("cannot save config.ron: {e:#}"));
        }
    }
```

(e) `handle_request`'s `Request::Settings` arm (the stub Task 3 left) becomes:
```rust
            Request::Settings => (Ok(None), self.open_settings()),
```

(f) `update` gains an arm next to `Msg::ConfigChanged`, and `ConfigChanged` itself learns to refresh the window:
```rust
            Msg::ConfigChanged(config) => {
                let task = self.apply_config(config);
                // A hand edit may have fixed or broken the file; the text
                // fields deliberately keep whatever is being typed.
                if let Some(state) = self.settings.as_mut() {
                    state.refresh();
                }
                task
            }
            Msg::Settings(msg) => self.on_settings(msg),
```

(g) `Application` gains the close hook, and `view_window` dispatches on the id:
```rust
    /// Fires when a window is actually gone (libcosmic maps
    /// `window::Event::Closed` here). `exit_on_close(false)` means closing
    /// the settings window never exits the daemon.
    fn on_close_requested(&self, id: SurfaceId) -> Option<Msg> {
        self.settings.as_ref().filter(|s| s.window == id).map(|_| Msg::Settings(settings::Msg::Closed))
    }
```
and at the very top of `view_window`:
```rust
        if let Some(state) = self.settings.as_ref().filter(|s| s.window == id) {
            let focused = self.core.focused_window() == Some(id);
            return settings::view(state, &self.config, focused);
        }
```
(The existing `let Some((_, client)) = …` line follows unchanged. `Msg::Pointer` is also emitted for this window — `on_pointer` looks the id up in `clients`, finds nothing and returns `Task::none()`, so nothing more is needed.)

In `src/main.rs`, add to `enum Command` after `Layouts`:
```rust
    /// Open the settings window
    Settings,
```
and the arm:
```rust
        Some(Command::Settings) => Ok(cli::send(&ipc::Request::Settings)),
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -q ui::settings 2>&1 | tail -3   # test result: ok. 3 passed
cargo build -q 2>&1 | tail -10              # no output, no warnings
```
Suite delta on Task 3's total: **+3**.

- [ ] **Step 5: Commit**

```bash
git add src/ui/settings.rs src/ui/mod.rs src/main.rs
git commit -m "feat(settings): the settings window and its Display page

`yutani settings` (IPC `settings`) opens a real toplevel through
iced::window::open; a second request raises it with an xdg-activation token,
because gain_focus does nothing on Wayland. Width, hover zoom, names, border
width, corner radius and both border colours apply live and are written back
atomically — sliders only on release. While config.ron does not parse the
window shows why and writes nothing at all.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 5: Pages — the tab strip, Behavior, Layouts — and the applet's Preferences… row

**Files:**
- Modify: `src/ui/settings.rs`, `src/ui/mod.rs`, `src/applet/mod.rs`, `src/bin/yutani-applet/app.rs`
- Test: the `mod tests` blocks in `src/ui/settings.rs` and `src/applet/mod.rs`

**Interfaces:**
- Consumes: Task 4's `State`, `Msg`, `apply_config_field`, `is_live_only`, `view`, `App::{on_settings, settings_note, save_config}`; Task 3's `App::apply_layout`; Task 1's `layout::{list_names, rename_named, delete_named}` and `Layout::save_named`; `crate::shortcuts::{install, uninstall}` (`install(&ShortcutsConfig) -> anyhow::Result<(usize, usize)>` — installed and wanted; `uninstall() -> anyhow::Result<usize>`); plan B's `yutani::applet::Action` and `yutani::ipc::Request::Settings`.
- Produces:
  ```rust
  // src/ui/settings.rs
  pub enum Page { Display, Behavior, Layouts }
  pub const MODES: [(&str, Mode); 2]; pub const EDGES: [(&str, Edge); 4];
  pub const FPS: [u32; 4]; pub const FPS_LABELS: [&str; 4];
  pub const VISIBILITIES: [(&str, Visibility); 2]; pub const PREFIXES: [(&str, &[Modifier]); 5];
  pub fn prefix_index(prefix: &[Modifier]) -> Option<usize>;
  pub fn prefix_at(index: usize) -> Option<Vec<Modifier>>;
  pub fn parse_shortcut_key(text: &str, other: &str) -> Result<String, String>;
  // src/ui/mod.rs
  fn settings_save_as(&mut self);
  fn settings_apply_layout(&mut self, name: String) -> Task<cosmic::Action<Msg>>;
  fn settings_rename_layout(&mut self, from: String);
  fn settings_delete_layout(&mut self, name: String);
  fn settings_shortcuts(&mut self, install: bool);
  ```

- [ ] **Step 1: Write the failing tests**

Add to `src/ui/settings.rs`'s `mod tests`:

```rust
    #[test]
    fn every_enum_value_has_a_dropdown_entry() {
        for m in [Mode::Floating, Mode::Dock] {
            assert!(index_of(&MODES, &m).is_some(), "{m:?}");
        }
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            assert!(index_of(&EDGES, &e).is_some(), "{e:?}");
        }
        for v in [Visibility::Always, Visibility::EveFocusedOnly] {
            assert!(index_of(&VISIBILITIES, &v).is_some(), "{v:?}");
        }
        assert_eq!(FPS.len(), FPS_LABELS.len());
        assert!(FPS.contains(&Config::default().fps));
        // The default prefix is offered, and round-trips through the table.
        let default_prefix = Config::default().shortcuts.focus_prefix;
        let i = prefix_index(&default_prefix).expect("the default prefix is in the list");
        assert_eq!(prefix_at(i), Some(default_prefix));
        assert_eq!(prefix_at(PREFIXES.len()), None);
        // A hand-edited chord the list does not offer simply selects nothing.
        assert_eq!(prefix_index(&[Modifier::Shift]), None);
    }

    #[test]
    fn behavior_fields_land_in_the_config() {
        let mut c = Config::default();
        let floating = index_of(&MODES, &Mode::Floating).unwrap();
        assert_eq!(apply_config_field(&mut c, &Msg::Mode(floating)), Ok(true));
        assert_eq!(c.mode, Mode::Floating);
        let left = index_of(&EDGES, &Edge::Left).unwrap();
        assert_eq!(apply_config_field(&mut c, &Msg::DockEdge(left)), Ok(true));
        assert_eq!(c.dock_edge, Edge::Left);
        assert_eq!(apply_config_field(&mut c, &Msg::Fps(0)), Ok(true));
        assert_eq!(c.fps, 10);
        let always = index_of(&VISIBILITIES, &Visibility::Always).unwrap();
        assert_eq!(apply_config_field(&mut c, &Msg::Visibility(always)), Ok(true));
        assert_eq!(c.visibility, Visibility::Always);
        assert_eq!(apply_config_field(&mut c, &Msg::HideActive(true)), Ok(true));
        assert_eq!(apply_config_field(&mut c, &Msg::SnapGrid(false)), Ok(true));
        assert_eq!(apply_config_field(&mut c, &Msg::SnapEdges(false)), Ok(true));
        assert!(c.hide_active && !c.snap_grid && !c.snap_edges);
        assert_eq!(apply_config_field(&mut c, &Msg::Prefix(1)), Ok(true));
        assert_eq!(c.shortcuts.focus_prefix, prefix_at(1).unwrap());
        // An index no table entry has is a note, not a panic and not a write.
        let before = c.clone();
        assert!(apply_config_field(&mut c, &Msg::Fps(9)).is_err());
        assert!(apply_config_field(&mut c, &Msg::Mode(9)).is_err());
        assert!(apply_config_field(&mut c, &Msg::Prefix(9)).is_err());
        assert_eq!(c, before);
    }

    #[test]
    fn shortcut_keys_must_be_keysym_names_distinct_from_the_other_one() {
        assert_eq!(parse_shortcut_key("  Tab  ", "Left"), Ok("Tab".to_string()));
        assert_eq!(parse_shortcut_key("F12", "Left"), Ok("F12".to_string()));
        assert!(parse_shortcut_key("", "Left").is_err());
        assert!(parse_shortcut_key("left", "Left").unwrap_err().contains("other shortcut key"));
        assert!(parse_shortcut_key("3", "Left").unwrap_err().contains("focus"));
        assert!(parse_shortcut_key("Rihgt", "Left").unwrap_err().contains("keysym"));
        let mut c = Config::default();
        assert_eq!(apply_config_field(&mut c, &Msg::NextKey("Tab".into())), Ok(true));
        assert_eq!(c.shortcuts.next, "Tab");
        assert!(apply_config_field(&mut c, &Msg::PrevKey("tab".into())).is_err());
        assert_eq!(c.shortcuts.prev, "Left");
    }
```

In `src/applet/mod.rs`'s `mod tests`, replace the line `assert_eq!(Action::Preferences.request(), None);` with:
```rust
        assert_eq!(Action::Preferences.request(), Some(crate::ipc::Request::Settings));
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -q 'settings::' 2>&1 | tail -10` and `cargo test -q applet:: 2>&1 | tail -10`
Expected: FAIL to compile — `cannot find value 'MODES' in this scope`, `no variant named 'Mode' found for enum 'Msg'`, `cannot find function 'index_of'`; and for the applet, `assertion left == right failed: left: None, right: Some(Settings)`.

- [ ] **Step 3: Write the implementation — `src/ui/settings.rs`**

Extend the imports at the top of the file:
```rust
use cosmic::widget::segmented_button;

use crate::model::config::{Config, Edge, Mode, Modifier, Visibility, parse_color, resolve_keysym};
use crate::model::layout;
```
(replacing the Task 4 `use crate::model::config::{Config, parse_color};` line).

Add the page enum and the choice tables, above `State`:
```rust
/// The three pages of spec §6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Display,
    Behavior,
    Layouts,
}

pub const MODES: [(&str, Mode); 2] = [("Floating", Mode::Floating), ("Dock", Mode::Dock)];
pub const EDGES: [(&str, Edge); 4] =
    [("Top", Edge::Top), ("Bottom", Edge::Bottom), ("Left", Edge::Left), ("Right", Edge::Right)];
pub const FPS: [u32; 4] = [10, 15, 30, 60];
pub const FPS_LABELS: [&str; 4] = ["10", "15", "30", "60"];
pub const VISIBILITIES: [(&str, Visibility); 2] =
    [("Always", Visibility::Always), ("Only while EVE has focus", Visibility::EveFocusedOnly)];

/// The `focus_prefix` chords the dropdown offers (spec §6, "shortcut
/// prefix"): a fixed list keeps this one control instead of four
/// checkboxes, and every entry is a chord cosmic-comp accepts. A config
/// hand-edited to some other chord simply selects nothing.
pub const PREFIXES: [(&str, &[Modifier]); 5] = [
    ("Ctrl + Alt", &[Modifier::Ctrl, Modifier::Alt]),
    ("Super", &[Modifier::Super]),
    ("Super + Shift", &[Modifier::Super, Modifier::Shift]),
    ("Ctrl + Shift", &[Modifier::Ctrl, Modifier::Shift]),
    ("Super + Alt", &[Modifier::Super, Modifier::Alt]),
];

fn labels<T>(table: &[(&'static str, T)]) -> Vec<&'static str> {
    table.iter().map(|(label, _)| *label).collect()
}

fn index_of<T: PartialEq>(table: &[(&'static str, T)], value: &T) -> Option<usize> {
    table.iter().position(|(_, v)| v == value)
}

pub fn prefix_index(prefix: &[Modifier]) -> Option<usize> {
    PREFIXES.iter().position(|(_, p)| *p == prefix)
}

pub fn prefix_at(index: usize) -> Option<Vec<Modifier>> {
    PREFIXES.get(index).map(|(_, p)| p.to_vec())
}

/// A `next`/`prev` binding key: an xkb keysym name (what COSMIC's shortcut
/// file stores), distinct from the other one, and not a digit 1-9 —
/// `focus 1..9` takes those. The same rule `Config::validate` enforces,
/// repeated here so the field can refuse before anything is written: an
/// invalid name fails cosmic-comp's whole custom-shortcut map.
pub fn parse_shortcut_key(text: &str, other: &str) -> Result<String, String> {
    let key = text.trim();
    if key.is_empty() {
        return Err("a shortcut key cannot be empty".into());
    }
    if key.eq_ignore_ascii_case(other) {
        return Err(format!("{key:?} is already the other shortcut key"));
    }
    if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() && key != "0" {
        return Err("1-9 are taken by focus 1..9".into());
    }
    if resolve_keysym(key).is_none() {
        return Err(format!("{key:?} is not an xkb keysym name (try \"Right\", \"Tab\", \"a\", \"F12\")"));
    }
    Ok(key.to_string())
}
```

Replace `State` and its `impl` with the full version:
```rust
pub struct State {
    pub window: SurfaceId,
    /// The three-page tab strip; its active entity carries a `Page`.
    pub pages: segmented_button::SingleSelectModel,
    /// Set when `config.ron` exists but does not parse: the Display and
    /// Behavior pages are replaced by the reason and nothing is written.
    /// The Layouts page still works — it writes layout files, not this one.
    pub config_error: Option<String>,
    /// Saved layout names, refreshed after every Layouts-page action.
    pub layouts: Vec<String>,
    /// The Save-as / Rename target.
    pub name_field: String,
    /// Text fields are kept as typed, so a half-typed value never reaches
    /// the config; they are seeded when the window opens and are not
    /// re-seeded by a later hand edit (which would eat what is being typed).
    pub active_border_field: String,
    pub inactive_border_field: String,
    pub next_field: String,
    pub prev_field: String,
    /// One line of feedback under the page.
    pub note: Option<String>,
}

impl State {
    pub fn new(window: SurfaceId, config: &Config) -> Self {
        let mut pages = segmented_button::SingleSelectModel::default();
        pages.insert().text("Display").data(Page::Display).activate();
        pages.insert().text("Behavior").data(Page::Behavior);
        pages.insert().text("Layouts").data(Page::Layouts);
        let mut state = Self {
            window,
            pages,
            config_error: None,
            layouts: Vec::new(),
            name_field: String::new(),
            active_border_field: config.active_border.clone().unwrap_or_default(),
            inactive_border_field: config.inactive_border.clone(),
            next_field: config.shortcuts.next.clone(),
            prev_field: config.shortcuts.prev.clone(),
            note: None,
        };
        state.refresh();
        state
    }

    /// Re-read what lives outside `Config`: the saved layout names and
    /// whether `config.ron` currently parses.
    pub fn refresh(&mut self) {
        self.layouts = layout::list_names();
        self.config_error = Config::try_load().err();
    }
}
```

Add the new message variants to `Msg`, after `InactiveBorder(String)`:
```rust
    /// A tab was pressed.
    Page(segmented_button::Entity),
    Mode(usize),
    DockEdge(usize),
    Fps(usize),
    Visibility(usize),
    HideActive(bool),
    SnapGrid(bool),
    SnapEdges(bool),
    Prefix(usize),
    NextKey(String),
    PrevKey(String),
    InstallShortcuts,
    UninstallShortcuts,
    /// The Save-as / Rename name field.
    Name(String),
    SaveAs,
    Apply(String),
    Rename(String),
    Delete(String),
```

Extend `apply_config_field`'s match, before the `_ => return Ok(false),` arm:
```rust
        Msg::Mode(i) => config.mode = MODES.get(*i).ok_or_else(|| "unknown mode".to_string())?.1,
        Msg::DockEdge(i) => config.dock_edge = EDGES.get(*i).ok_or_else(|| "unknown dock edge".to_string())?.1,
        Msg::Fps(i) => config.fps = *FPS.get(*i).ok_or_else(|| "unknown frame rate".to_string())?,
        Msg::Visibility(i) => {
            config.visibility = VISIBILITIES.get(*i).ok_or_else(|| "unknown visibility".to_string())?.1;
        }
        Msg::HideActive(v) => config.hide_active = *v,
        Msg::SnapGrid(v) => config.snap_grid = *v,
        Msg::SnapEdges(v) => config.snap_edges = *v,
        Msg::Prefix(i) => {
            config.shortcuts.focus_prefix = prefix_at(*i).ok_or_else(|| "unknown shortcut prefix".to_string())?;
        }
        Msg::NextKey(text) => {
            let key = parse_shortcut_key(text, &config.shortcuts.prev)?;
            config.shortcuts.next = key;
        }
        Msg::PrevKey(text) => {
            let key = parse_shortcut_key(text, &config.shortcuts.next)?;
            config.shortcuts.prev = key;
        }
```

Replace the body of `view` that picks the page, so the tab strip is drawn and each page has its own arm:
```rust
pub fn view<'a>(state: &'a State, config: &'a Config, focused: bool) -> Element<'a, super::Msg> {
    let page = state.pages.active_data::<Page>().copied().unwrap_or(Page::Display);
    let body: Element<'a, Msg> = match (page, &state.config_error) {
        // Layout files are not `config.ron`; this page works either way.
        (Page::Layouts, _) => layouts_page(state),
        (_, Some(error)) => broken_config(error),
        (Page::Display, None) => display_page(state, config),
        (Page::Behavior, None) => behavior_page(state, config),
    };
    let note: Element<'a, Msg> = match &state.note {
        Some(note) => widget::text::caption(note.as_str()).into(),
        None => widget::text::caption(
            "Changes apply at once and are written to ~/.config/yutani/config.ron. \
             Comments and unknown keys in that file are not preserved.",
        )
        .into(),
    };
    let tabs: Element<'a, Msg> = widget::segmented_control::horizontal(&state.pages).on_activate(Msg::Page).into();
    let page = widget::container(widget::column::with_children(vec![tabs, body, note]).spacing(12))
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill);
    let content: Element<'a, Msg> = widget::container(widget::column::with_children(vec![
        widget::header_bar()
            .title("Yutani Settings")
            .on_close(Msg::Close)
            .on_drag(Msg::Drag)
            .focused(focused)
            .into(),
        page.into(),
    ]))
    .class(cosmic::theme::Container::WindowBackground)
    .width(Length::Fill)
    .height(Length::Fill)
    .into();
    content.map(super::Msg::Settings)
}
```

Append the two new pages after `display_page`:
```rust
fn behavior_page<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let mode =
        widget::settings::item("Mode", widget::dropdown(labels(&MODES), index_of(&MODES, &config.mode), Msg::Mode));
    let edge = widget::settings::item(
        "Dock edge",
        widget::dropdown(labels(&EDGES), index_of(&EDGES, &config.dock_edge), Msg::DockEdge),
    );
    let fps = widget::settings::item(
        "Capture frame rate",
        widget::dropdown(FPS_LABELS.to_vec(), FPS.iter().position(|f| *f == config.fps), Msg::Fps),
    );
    let visibility = widget::settings::item(
        "Show thumbnails",
        widget::dropdown(labels(&VISIBILITIES), index_of(&VISIBILITIES, &config.visibility), Msg::Visibility),
    );
    let hide_active = widget::settings::item(
        "Hide the focused client's own thumbnail",
        widget::toggler(config.hide_active).on_toggle(Msg::HideActive),
    );
    let snap_grid = widget::settings::item(
        "Snap to a 32 px grid while dragging",
        widget::toggler(config.snap_grid).on_toggle(Msg::SnapGrid),
    );
    let snap_edges = widget::settings::item(
        "Snap flush against other thumbnails",
        widget::toggler(config.snap_edges).on_toggle(Msg::SnapEdges),
    );
    let prefix = widget::settings::item(
        "Shortcut prefix",
        widget::dropdown(labels(&PREFIXES), prefix_index(&config.shortcuts.focus_prefix), Msg::Prefix),
    );
    let next = widget::settings::item(
        "Next client key",
        widget::text_input("Right", state.next_field.as_str()).on_input(Msg::NextKey).width(Length::Fixed(160.0)),
    );
    let prev = widget::settings::item(
        "Previous client key",
        widget::text_input("Left", state.prev_field.as_str()).on_input(Msg::PrevKey).width(Length::Fixed(160.0)),
    );
    let buttons = widget::settings::item_row(vec![
        widget::button::standard("Install shortcuts").on_press(Msg::InstallShortcuts).into(),
        widget::button::destructive("Uninstall shortcuts").on_press(Msg::UninstallShortcuts).into(),
    ]);
    widget::settings::view_column(vec![
        widget::settings::section().title("Arrangement").add(mode).add(edge).add(fps).into(),
        widget::settings::section().title("Visibility").add(visibility).add(hide_active).into(),
        widget::settings::section().title("Dragging").add(snap_grid).add(snap_edges).into(),
        widget::settings::section()
            .title("Keyboard shortcuts")
            .add(prefix)
            .add(next)
            .add(prev)
            .add(buttons)
            .into(),
    ])
    .into()
}

fn layouts_page(state: &State) -> Element<'_, Msg> {
    let named = !state.name_field.trim().is_empty();
    let mut saved = widget::settings::section().title("Saved layouts");
    if state.layouts.is_empty() {
        saved = saved.add(widget::text::body(
            "None yet. Arrange the thumbnails, type a name below and press Save current as….",
        ));
    }
    for name in &state.layouts {
        saved = saved.add(widget::settings::item(
            name.as_str(),
            widget::settings::item_row(vec![
                widget::button::standard("Apply").on_press(Msg::Apply(name.clone())).into(),
                widget::button::text("Rename").on_press_maybe(named.then(|| Msg::Rename(name.clone()))).into(),
                widget::button::destructive("Delete").on_press(Msg::Delete(name.clone())).into(),
            ]),
        ));
    }
    let save = widget::settings::section().title("Save the current arrangement").add(widget::settings::item_row(vec![
        widget::text_input("Layout name", state.name_field.as_str())
            .on_input(Msg::Name)
            .on_submit(|_| Msg::SaveAs)
            .width(Length::Fill)
            .into(),
        widget::button::suggested("Save current as…").on_press_maybe(named.then_some(Msg::SaveAs)).into(),
    ]));
    widget::settings::view_column(vec![
        saved.into(),
        save.into(),
        widget::text::caption(
            "Rename uses the name typed in the field. Applying a layout copies it to current.ron and moves the \
             thumbnails; a layout that names a disconnected monitor lands on the primary one.",
        )
        .into(),
    ])
    .into()
}
```

- [ ] **Step 4: Write the implementation — `src/ui/mod.rs`**

Add the five handlers to `impl App`, next to `on_settings`:
```rust
    /// Layouts page: save the current arrangement under the typed name.
    /// `current.ron` is refreshed first so the copy carries today's order
    /// and anchor.
    fn settings_save_as(&mut self) {
        let name = self.settings.as_ref().map(|s| s.name_field.clone()).unwrap_or_default();
        self.save_current_layout();
        match self.layout.save_named(&name) {
            Ok(()) => {
                let note = format!("saved layout {:?}", name.trim());
                if let Some(state) = self.settings.as_mut() {
                    state.name_field.clear();
                    state.layouts = layout::list_names();
                    state.note = Some(note);
                }
            }
            Err(e) => self.settings_note(e),
        }
    }

    fn settings_apply_layout(&mut self, name: String) -> Task<cosmic::Action<Msg>> {
        match self.apply_layout(&name) {
            Ok(task) => {
                self.settings_note(format!("applied layout {name:?}"));
                task
            }
            Err(e) => {
                self.settings_note(e);
                Task::none()
            }
        }
    }

    fn settings_rename_layout(&mut self, from: String) {
        let to = self.settings.as_ref().map(|s| s.name_field.clone()).unwrap_or_default();
        match layout::rename_named(&from, &to) {
            Ok(()) => {
                let note = format!("renamed {:?} to {:?}", from, to.trim());
                if let Some(state) = self.settings.as_mut() {
                    state.name_field.clear();
                    state.layouts = layout::list_names();
                    state.note = Some(note);
                }
            }
            Err(e) => self.settings_note(e),
        }
    }

    fn settings_delete_layout(&mut self, name: String) {
        match layout::delete_named(&name) {
            Ok(()) => {
                let note = format!("deleted layout {name:?}");
                if let Some(state) = self.settings.as_mut() {
                    state.layouts = layout::list_names();
                    state.note = Some(note);
                }
            }
            Err(e) => self.settings_note(e),
        }
    }

    /// Behavior page: the *Install shortcuts* / *Uninstall shortcuts*
    /// buttons of spec §6, over the same code path as
    /// `yutani shortcuts install|uninstall`.
    fn settings_shortcuts(&mut self, install: bool) {
        let note = if install {
            match crate::shortcuts::install(&self.config.shortcuts) {
                Ok((installed, wanted)) if installed == wanted => format!("installed {installed} shortcuts"),
                Ok((installed, wanted)) => {
                    format!("installed {installed} of {wanted} shortcuts (the rest are bound by something else)")
                }
                Err(e) => format!("{e:#}"),
            }
        } else {
            match crate::shortcuts::uninstall() {
                Ok(n) => format!("removed {n} shortcuts"),
                Err(e) => format!("{e:#}"),
            }
        };
        self.settings_note(note);
    }
```

In `on_settings`, add `S::Name` to the state-only block (next to `S::ActiveBorder`):
```rust
                S::Name(text) => {
                    state.name_field = text.clone();
                    return Task::none();
                }
```
and insert this block **between** the state-only block and the `if matches!(msg, S::Commit)` line:
```rust
        // Pages and actions that are not config fields.
        match &msg {
            S::Page(entity) => {
                if let Some(state) = self.settings.as_mut() {
                    state.pages.activate(*entity);
                }
                return Task::none();
            }
            S::SaveAs => {
                self.settings_save_as();
                return Task::none();
            }
            S::Apply(name) => return self.settings_apply_layout(name.clone()),
            S::Rename(from) => {
                self.settings_rename_layout(from.clone());
                return Task::none();
            }
            S::Delete(name) => {
                self.settings_delete_layout(name.clone());
                return Task::none();
            }
            S::InstallShortcuts => {
                self.settings_shortcuts(true);
                return Task::none();
            }
            S::UninstallShortcuts => {
                self.settings_shortcuts(false);
                return Task::none();
            }
            _ => {}
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -q 'settings::' 2>&1 | tail -3   # test result: ok. 6 passed
cargo build -q 2>&1 | tail -10              # no output, no warnings
```

- [ ] **Step 6: Flip the applet's Preferences… row to the settings window**

Plan B built that row on `xdg-open`, explicitly "until plan 5's settings window exists". Three edits:

In `src/applet/mod.rs`, the `Action` enum's doc comment:
```rust
    /// Open `~/.config/yutani/config.ron` with `xdg-open`.
    Preferences,
```
becomes
```rust
    /// Ask the daemon to open its settings window (spec §6).
    Preferences,
```
and in `impl Action::request`, the arm
```rust
            Action::Preferences | Action::StartDaemon => None,
```
becomes
```rust
            Action::Preferences => Some(Request::Settings),
            Action::StartDaemon => None,
```

In `src/bin/yutani-applet/app.rs`, delete the whole `fn open_preferences() -> Result<(), String>` (including its doc comment) and delete the update arm:
```rust
            Msg::Press(Action::Preferences) => {
                if let Err(msg) = Self::open_preferences() {
                    self.note(msg);
                }
                Task::none()
            }
```
The generic `Msg::Press(action)` arm below it now handles Preferences: it sends the request and turns any `err …` into the usual 3-second note. (The Preferences… row is only ever rendered from a live `status`, so "yutani is not running" cannot normally be the answer — the offline menu has just the *Start Yutani* row.)

- [ ] **Step 7: Run everything**

```bash
cargo test -q applet:: 2>&1 | tail -3    # test result: ok. <unchanged count> passed
cargo build -q 2>&1 | tail -10           # no output, no warnings
cargo build -q --bin yutani-applet 2>&1 | tail -5
cargo test -q 2>&1 | grep -h 'test result' | awk '{s+=$4} END {print s}'
```
Suite delta on Task 4's total: **+3** (all in `ui::settings`; the applet test changed an assertion, not its count).

- [ ] **Step 8: Commit**

```bash
git add src/ui/settings.rs src/ui/mod.rs src/applet/mod.rs src/bin/yutani-applet/app.rs
git commit -m "feat(settings): Behavior and Layouts pages; applet Preferences opens the window

The window gets its three-page tab strip (spec §6). Behavior covers mode,
dock edge, frame rate, visibility, hide-active, both snapping toggles, the
shortcut prefix and next/prev keys (refused unless they are real xkb keysym
names) plus the Install/Uninstall shortcuts buttons. Layouts saves the
current arrangement under a name, applies, renames and deletes — and keeps
working while config.ron is broken, because layout files are not that file.
The applet's Preferences… row now sends the IPC `settings` request instead
of xdg-open.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 6: Hands-on acceptance (Daniel) and the spec status notes

Everything up to here is verifiable without a compositor. This task needs a COSMIC session, EVE running (two clients if possible) and, for the last step, a second monitor.

- [ ] **Step 1: Open the window**

```bash
cargo build -q
(RUST_LOG=yutani=info setsid nohup ./target/debug/yutani >/tmp/yutani-plan5.log 2>&1 &); sleep 4
./target/debug/yutani settings; echo "exit=$?"
```
The window appears with its header bar, three tabs and the Display page. Then:
- `./target/debug/yutani settings` again → the **same** window is raised, not a second one.
- Click the header bar's ✕ → it closes and the thumbnails keep running. `./target/debug/yutani settings` reopens it.
- Drag the header bar → the window moves.

- [ ] **Step 2: Display page**

Drag *Thumbnail width* — the thumbnails resize while dragging, and `~/.config/yutani/config.ron` gains the new `thumb_width` **once**, on release (`watch -n1 'stat -c %y ~/.config/yutani/config.ron'` should not tick per pixel). Then check each control changes what it says it does: hover zoom (hover a thumbnail), show names, border width, corner radius, `#ff8800` in *Active border colour* (the focused thumbnail's border turns orange), clearing that field (it goes back to the theme accent), a bad value such as `red` (the note line explains, and the file is unchanged).

- [ ] **Step 3: Behavior page**

Switch Mode to Floating and back to Dock; change the dock edge; drop the frame rate to 10 and watch `yutani status` / the thumbnails; flip *Show thumbnails* between Always and Only-while-EVE-has-focus; toggle hide-active and both snapping toggles (drag a floating thumbnail to confirm). Set the shortcut prefix to Super, press **Install shortcuts**, then check `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom` and press Super+1. Type `Rihgt` into *Next client key* → the note refuses it and `config.ron` is untouched. Press **Uninstall shortcuts**, then re-install the Ctrl+Alt set you actually want.

- [ ] **Step 4: Layouts page and the CLI**

Arrange the thumbnails (floating mode), type `pvp` and press **Save current as…**:
```bash
cat ~/.config/yutani/layouts/pvp.ron          # thumbs + order + new_client_anchor
./target/debug/yutani layouts                 # pvp
```
Move the thumbnails somewhere else, then `./target/debug/yutani layout pvp` → they jump back. Press **Apply** on the row → same thing. Type `fleet`, press **Rename** on the `pvp` row → the file is renamed and the list updates. Press **Delete** → it is gone. `./target/debug/yutani layout nope; echo "exit=$?"` → `yutani: no such layout nope`, exit 1.

- [ ] **Step 5: Two monitors and the output fallback (spec §9)**

With both monitors on, drag one thumbnail to the second monitor, save the layout as `two-screens`, then unplug (or disable in COSMIC Settings → Displays) that monitor. `./target/debug/yutani layout two-screens` → that thumbnail appears on the remaining (primary) monitor at the same x/y, and the others are where they were. Plug the monitor back in and apply again → it returns to the second monitor. Confirm in the log that a surface was recreated rather than merely moved.

- [ ] **Step 6: The broken-config guard**

```bash
cp ~/.config/yutani/config.ron /tmp/yutani-config.bak
printf '(this is not ron' > ~/.config/yutani/config.ron
```
Within a second the open window replaces the Display and Behavior pages with the "config.ron cannot be read" notice; the **Layouts** tab still works. Restore the file (`cp /tmp/yutani-config.bak ~/.config/yutani/config.ron`) and press **Re-check** (or wait for the watcher) → the pages come back. Confirm `~/.config/yutani/config.ron` was never overwritten while broken.

- [ ] **Step 7: The applet's Preferences… row**

With the panel applet added (plan B), open the popup and press **Preferences…** → the settings window opens (it no longer opens `config.ron` in a text editor). Press it again → the window is raised.

- [ ] **Step 8: Spec status notes and commit**

In `docs/superpowers/specs/2026-09-11-yutani-design.md`:

1. §6, replace `All changes apply live and write `config.ron`.` with:
```
All changes apply live and write `config.ron`.

*Status after plan 5:* implemented (`src/ui/settings.rs`), opened by
`yutani settings` or the applet's *Preferences…* row — both the IPC
`settings` request — and a second request raises the open window with an
xdg-activation token (`window::gain_focus` does nothing on Wayland). Pages
are a `segmented_control` tab strip. Writes are atomic (tmp + rename) and
drop comments and unknown keys; while `config.ron` does not parse, the
Display and Behavior pages are replaced by a notice and nothing is written
at all (the Layouts page still works — layout files are separate).
```
2. §7, in the *Status after plan 4* paragraph, replace `(CLI; the settings-window buttons come with plan 5)` with `(CLI, and the *Install shortcuts* / *Uninstall shortcuts* buttons on the settings window's Behavior page since plan 5)`.
3. §8, the command list and reply grammar are now out of date (`layouts` is
   not in them, and a reply can carry data). Replace the `Commands:` /
   `Reply:` paragraph with:
```
Commands: `focus <n>`, `next`, `prev`, `show`, `hide`, `toggle` (show if hidden,
hide if shown),
`layout <name>`, `layouts`, `settings`, `quit`.
Reply: `ok\n`, `ok <data>\n` for a request that answers with something
(`layouts` answers a JSON array of names) or `err <message>\n`. The CLI
prints the error and exits 1; if the socket is absent it prints
"yutani is not running" and exits 1.
```
   and, after the *Status after plan 4* paragraph, add:
```
*Status after plan 5:* `layout <name>` and `settings` are implemented, and
`layouts` was added (reply `ok <json array of names>`, the `OkData` form;
`yutani layouts` prints one per line). `layout` answers
`err no such layout <name>` for an unknown one and `err <reason>` for a
name that is not a plain file stem.
```
4. §9, replace the whole *Status after plan 2* paragraph (`*Status after plan 2:* `output` is saved but not yet used for placement … Both land with named layouts in plan 4.`) with:
```
*Status after plan 5:* implemented. `order` is rewritten on every drag, pin
and order change; previously recorded names keep their slots (a logged-out
character does not lose its place), live characters not yet recorded are
appended after them in layout order, and when the 64-name cap bites,
recorded names that are neither live nor have a saved position are evicted
first, oldest first. `thumbs` itself is uncapped: an entry is exactly what
brings a character back to their own spot, and one costs ~50 bytes.
`new_client_anchor` is rewritten at the same moments and is the
top-left-most saved thumbnail; a new client stacks 24 px down-right of it
past occupied slots. A saved `output` now decides which output a floating
thumbnail is created on, falling back to the primary output — the first one
iced reports, since COSMIC advertises no primary-output protocol — at the
same x/y. Named layouts live in `~/.config/yutani/layouts/<name>.ron`
(`<name>`: non-empty, ≤ 64 characters, no path separators or control
characters, and not `current`, which is reserved for the auto-saved
layout). Both files are written atomically: a sibling temporary, then a
rename over the target.
```
5. §10, after the `Config/layout parse errors → warn and fall back to
   defaults; never overwrite the offending file.` bullet, add:
```
- The one exception is `yutani layout <name>` / the Layouts page's *Apply*:
  an explicit request to replace `current.ron`, so it does overwrite an
  unparseable one. A `current.ron` that fails to parse suspends auto-save
  until the file is fixed or deleted; the settings window shows a warning
  and saving resumes automatically once it parses again.
```

```bash
git add docs/superpowers/specs/2026-09-11-yutani-design.md
git commit -m "docs(spec): settings window, named layouts and the §9 gaps are implemented

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

## Self-review

**Spec coverage**

| Spec item | Task |
|---|---|
| §3 crate layout — `ui/settings.rs` "settings window pages" | 4 (created), 5 (all three pages) |
| §4 unnamed clients use `new_client_anchor`, stacking 24 px down-right | 1 (`stacked_position`), 2 (`next_position`) |
| §6 Settings window is a libcosmic window, opened from the applet or `yutani settings`; single instance | 4 (`open_settings`, `window_settings`, raise-by-activation), 5 (applet row) |
| §6 Display page — thumb width, active/inactive border colour, border px, show names, zoom factor, corner radius | 4 (`display_page`) |
| §6 Behavior page — mode, dock edge, FPS, visibility, hide active, snap grid, snap edges, shortcut prefix/keys, Install/Uninstall buttons | 5 (`behavior_page`, `settings_shortcuts`) |
| §6 Layouts page — list, Save current as…, Apply, Rename, Delete | 5 (`layouts_page`, `settings_save_as/apply/rename/delete`) |
| §6 "All changes apply live and write `config.ron`" | 4 (`on_settings` → `apply_config` + `save_config`; sliders write on release) |
| §6 tray Settings item / Layouts submenu (tray deleted in plan B) | Mapped in "What replaces the tray": applet *Preferences…* (Task 5) and `yutani layout`/`layouts` + the Layouts page (Tasks 3, 5) |
| §7 settings-window shortcut install buttons | 5 |
| §8 `layout <name>`, `settings` (plan 4 stubs) | 3 (`layout`, plus the added `layouts`), 4 (`settings`) |
| §9 `config.ron` written by the settings UI; unparseable file never overwritten | 1 (`try_load_from`, `write_atomic`), 4 (`config_error` gate, `save_config`) |
| §9 `current.ron` auto-saved with `order` and `new_client_anchor` on every drag/pin/order change | 1 (model), 2 (`save_current_layout` and its three call sites) |
| §9 `~/.config/yutani/layouts/<name>.ron` | 1 (naming, list/load/save/rename/delete) |
| §9 output fallback: a layout naming a disconnected output uses the primary at the same x/y | 1 (`resolve_output`, `placement`), 2 (`output_for_thumb`), 3 (`reposition_to_layout`) |
| §9 "Applying a named layout copies it to `current.ron`" | 3 (`apply_layout`) |
| §11 pure model unit-tested: layout RON round-trip, output fallback, ordering | 1, 2, 4, 5 (every pure function has tests; view code is covered by `cargo build -q` and Task 6) |
| Ledger: `ThumbPos.output` saved but unused | 2 |
| Ledger: `Layout` lacks `order`/`new_client_anchor` | 1, 2 |
| Ledger: new-client placement overlaps saved positions | 1 (`stacked_position`), 2 (`next_position` counts saved positions as taken) |
| Ledger: `dock_order` duplicated by `focus_order`'s Dock arm | 2 (`dock_rank` is the single comparator; `dock_order` deleted) |

**Placeholder scan.** No "TBD", no "add error handling", no "tests for the above". Every type and function a task uses is defined either in an earlier task of this plan, in the current tree, or in plans A/B (and is named there with its exact signature). The one interim behaviour is deliberate and stated: Task 3 leaves `Request::Settings` answering `err not supported yet (the settings window lands in the next commit)`, which Task 4 replaces in the very next commit — it is a shipped, honest reply, not a placeholder. Task 4 deliberately ships the window with only the Display page and no tab strip; Task 5 restates `State`, `Msg` and `view` in full when it adds the strip, so no task has to read another task's diff.

**Type consistency.** `Layout { thumbs, order, new_client_anchor }`, `Anchor { output, x, y }`, `Placement { output, x, y, pinned, recreate }`, `STACK_STEP`, `MAX_ORDER`, `MAX_NAME`, `CURRENT` — defined once in Task 1, used with identical spelling in Tasks 2, 3 and 5. `rules::{dock_rank, focus_order(mode, order, items), FocusItem { handle, label, output, position }, step}` — Task 2, used by Tasks 2 and 3. `Client.output: String` — Task 3, written in `create_surface`, cleared in `forget_surface`, read in `reposition_to_layout`. `settings::{State, Msg, Page, view, window_settings, apply_config_field, is_live_only, parse_optional_color, parse_required_color, parse_shortcut_key, prefix_index, prefix_at}` — Tasks 4 and 5, with Task 5 restating the struct and enum in full rather than describing a delta. `ipc::Request::Layouts` and `Response::OkData` — Task 3, matching plan A's definition exactly (`OkData(String)`, `parse` splits on the space after `ok`). `handle_request`'s `(Result<Option<String>, String>, Task<cosmic::Action<Msg>>)` — plan A's type, used unchanged in Tasks 3 and 4. `applet::Action::Preferences.request()` returns `Some(ipc::Request::Settings)` — Task 5, matching plan B's `Action` definition and its `Msg::Press(action)` fallthrough.

**Fixed inline while writing this plan.**
- `sort_by_cached_key` is **not** a stable sort and would have broken the tie-ordering tests; Task 2 uses `sort_by` with `dock_rank`.
- `save_current_layout` would not borrow-check as `merge_order(&self.layout_order_names(), &self.layout.order, …)` assigned straight into `self.layout.order`; it binds `live` and `order` first.
- `reposition_to_layout` cannot compare against `output_name_of` (which already resolves through the **new** layout, so nothing would ever look like it moved); `Client.output` records the connector the surface was actually created on, which is what `placement` compares.
- `spin_button` takes **6** arguments at this rev, not 7: the `name` parameter is `#[cfg(feature = "a11y")]` and Yutani does not enable `a11y`.
- `text_input::on_submit` takes a closure (`Fn(String) -> Message`), so the Layouts page writes `.on_submit(|_| Msg::SaveAs)`.
- `settings::view_section` does not exist at this rev — only `view_column`, and `section()` must be `.into()`-ed.
- `window::gain_focus` is a no-op on Wayland, so raising uses `activation::request_token` + `activate`.
