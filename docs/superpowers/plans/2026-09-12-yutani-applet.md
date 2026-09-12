# Yutani COSMIC Panel Applet Implementation Plan (plan B)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `ksni` StatusNotifierItem with a real COSMIC panel applet: the Y mark in the panel with four icon states, and on click the designed popup — tunnel status and location, accounts connected, tunnel IP and handshake age, upload/download totals and rates, and the menu (Connect/Disconnect tunnel, Accounts…, Preferences…, Show/Hide thumbnails, Quit) — all driven by the daemon's IPC `status` reply, with an offline state when the daemon is not running.

**Architecture:** The crate grows a `src/lib.rs` exposing exactly what both binaries need (`assets`, `ipc`, `model`, `tunnel`, and the new pure `applet` model); `src/main.rs` stays the `yutani` daemon/CLI and `src/bin/yutani-applet/` is a second binary built on `cosmic::applet::run`. The applet holds no domain state: every second (popup open) or five seconds (closed) it sends `status` over `$XDG_RUNTIME_DIR/yutani.sock`, turns the reply into a pure `Display` + `Vec<MenuRow>`, and renders those. Menu presses map 1:1 to IPC requests and re-poll immediately. All decision logic (formatting, icon state, rate derivation, display strings, menu rows, install paths) lives in the lib and is unit-tested; the `src/bin/yutani-applet/` code is a dumb renderer verified by `cargo build --bin yutani-applet` and Daniel's hands-on task.

**Tech Stack:** Rust 2024; libcosmic pinned at `a401af8b1c54a8abd393b8c5b7c8809402f83850` with the `applet` feature added; `tokio` (`net`, `io-util`, `rt`) for the IPC client; `serde_json` (a direct dep from plan A) for the `status` payload.

**This plan:** `docs/superpowers/plans/2026-09-12-yutani-applet.md`
**Spec:** `docs/superpowers/specs/2026-09-12-yutani-applet-design.md`
**Visual source of truth:** the `design_handoff_y_wireguard_applet` bundle (`README.md` + `icons/`). Every colour, size and copy string it defines is tabulated in this plan and lands in `src/applet/theme.rs`; the implementer never needs the bundle.
**Runs after:** `docs/superpowers/plans/2026-09-12-yutani-tunnel.md` (plan A). This plan consumes plan A's `ipc::Request::Status`, `ipc::Response::OkData(String)` and `tunnel::status::{Status, ClientStatus, TunnelStatus}` exactly as plan A defines them and redefines none of them.

## Global Constraints

- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9
  ```
- Every commit: `cargo build -q` warning-free for new code (pre-existing: `dead_code` for `Config::save`/`save_to` — Task 1 incidentally cures it by moving `model` into the lib, where `pub` items are never dead), `cargo test -q` green.
- **`Cargo.lock` changes are limited to exactly these, all named and justified here.** Task 1 adds the libcosmic `applet` feature, which pulls `cosmic-panel-config` (the applet's panel-geometry/env config, mandatory for `cosmic::applet::run`); Task 5 drops `ksni`. Net effect, verified by regenerating the lock:
  - **added (7):** `cosmic-panel-config` (git, pop-os/cosmic-panel) — required by the `applet` feature; `xdg-shell-wrapper-config` (same repo) — its dependency; `build_helpers` (libcosmic's own build crate, pulled in by `cosmic-config` when built standalone); and second copies of `cosmic-config`, `cosmic-config-derive`, `iced_core`, `iced_futures` — because `cosmic-panel-config` depends on `pop-os/libcosmic` **without** a rev, so cargo resolves a second (branch-HEAD) source alongside our pinned `rev=a401af8…`. A `[patch."https://github.com/pop-os/libcosmic"]` cannot deduplicate these (cargo: *"patch for `cosmic-config` points to the same source, but patches must point to different sources"*). The duplicate entries are pinned by the lock, so builds stay reproducible; they cost compile time, not correctness.
  - **removed (2):** `ksni`, `pastey` (`pastey` has no other dependent). `zbus` stays — libcosmic's `single-instance` uses it.
  - No other `[[package]]` entry may appear or disappear. Check with `git diff Cargo.lock | grep '^[-+]name ='`.
- Colours, sizes and copy come from the tables in Task 1 and are used only through `src/applet/theme.rs` — no hex literal or px number from the handoff appears anywhere else.
- Polling cadence: 1 s while the popup is open, 5 s while it is closed. Rate = (delta bytes) / (elapsed seconds); a counter that went **down** (interface recreated) yields rate 0, never a negative; the first sample yields 0.
- Totals are cumulative counters: while disconnected they stay frozen at their last value and both rates read `0 KB/s`.
- Hands-on steps (panel add, visual check) are Task 6 and belong to Daniel. Every other task is verifiable with `cargo test -q` and `cargo build --bin yutani-applet` on a machine with no panel running.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/lib.rs` (new) | The `yutani` library: `pub mod applet; pub mod assets; pub mod ipc; pub mod model; pub mod tunnel;` — exactly what both binaries share. |
| `src/main.rs` (modify) | `yutani` daemon/CLI entry; `use yutani::{ipc, model, tunnel};` instead of `mod`; gains the `applet install|uninstall` subcommand. |
| `assets/icons/*.svg` (new, 5) | The handoff's production icons: `y-symbolic`, `y-symbolic-dark`, `y-sync-symbolic`, `y-attention-symbolic`, `y-color`. |
| `assets/glyphs/*.svg` (new, 3) | Popup decorations traced from the handoff prototype: `pin`, `arrow-up`, `arrow-down`. |
| `src/assets.rs` (new) | `include_bytes!` of all eight SVGs + the `ICONS` table `yutani applet install` writes. |
| `src/applet/mod.rs` (new) | `Action` (menu press → `ipc::Request`), `poll_interval`, note timing, `PENDING_S`. |
| `src/applet/theme.rs` (new) | The handoff's colour/size table as consts + the container/button style helpers built from them. |
| `src/applet/format.rs` (new) | Byte, rate, handshake and account-label formatting. Pure. |
| `src/applet/rate.rs` (new) | `Sampler`: two counter samples → `Rates`, with counter-reset handling. Pure. |
| `src/applet/icon.rs` (new) | `IconState` and its selection from a `TunnelStatus` + pending flag. Pure. |
| `src/applet/client.rs` (new) | tokio IPC client: `send_to`/`status_from` against the socket protocol; `IpcError::{Offline, Failed}`. |
| `src/applet/display.rs` (new) | `Display`: every string and flag the popup's read-only area shows, from `Option<&Status>` + `Rates`. Pure. |
| `src/applet/menu.rs` (new) | `MenuRow`/`RowKind` and the menu row list from `Option<&Status>` + expansion state. Pure. |
| `src/applet/install.rs` (new) | `.desktop` text, icon-theme paths, `install_to`/`uninstall_from`, `gtk-update-icon-cache`. |
| `src/bin/yutani-applet/main.rs` (new) | `cosmic::applet::run::<Applet>(())`. |
| `src/bin/yutani-applet/app.rs` (new) | The `cosmic::Application` impl: state, messages, polling, popup lifecycle, actions. |
| `src/bin/yutani-applet/view.rs` (new) | Panel button and popup rendering from `Display` + `Vec<MenuRow>`. |
| `src/ui/tray.rs` (deleted) | — |
| `src/ui/mod.rs` (modify) | Drop `mod tray`, `Msg::Tray` and the tray subscription. |
| `Cargo.toml` (modify) | libcosmic `applet` feature in; `ksni` out. |
| `docs/superpowers/specs/2026-09-11-yutani-design.md` (modify) | §6 "Tray" paragraph → pointer to the applet spec. |
| `docs/superpowers/specs/2026-09-12-yutani-applet-design.md` (modify) | Status note after acceptance. |

**Test-count baseline.** `cargo test -q` on `plan-4-hotkeys-ipc` today prints `81 passed`; plan A adds 26 (its own step deltas: +7, +11, +4, +0, +4), so this plan starts from **107**. If plan A's real total differs, use the **delta** stated in each task, not the absolute.

---

### Task 1: Library/binary split, the `applet` feature, assets and the theme table

**Files:**
- Create: `src/lib.rs`, `assets/icons/y-symbolic.svg`, `assets/icons/y-symbolic-dark.svg`, `assets/icons/y-sync-symbolic.svg`, `assets/icons/y-attention-symbolic.svg`, `assets/icons/y-color.svg`, `assets/glyphs/pin.svg`, `assets/glyphs/arrow-up.svg`, `assets/glyphs/arrow-down.svg`, `src/assets.rs`, `src/applet/mod.rs`, `src/applet/theme.rs`
- Modify: `Cargo.toml`, `src/main.rs`
- Test: the `mod tests` blocks inside `src/assets.rs` and `src/applet/theme.rs`

**Interfaces:**
- Consumes (from plan A): `crate::tunnel::status::{Status, ClientStatus, TunnelStatus}`, `crate::ipc::Request`, `crate::model::config::parse_color`.
- Produces:
  ```rust
  // src/lib.rs
  pub mod applet; pub mod assets; pub mod ipc; pub mod model; pub mod tunnel;
  // src/assets.rs
  pub const Y_SYMBOLIC: &[u8]; pub const Y_SYMBOLIC_DARK: &[u8]; pub const Y_SYNC_SYMBOLIC: &[u8];
  pub const Y_ATTENTION_SYMBOLIC: &[u8]; pub const Y_COLOR: &[u8];
  pub const PIN: &[u8]; pub const ARROW_UP: &[u8]; pub const ARROW_DOWN: &[u8];
  pub const ICONS: [(&str, &[u8]); 5];   // ("y-symbolic.svg", Y_SYMBOLIC), …
  // src/applet/mod.rs
  pub mod theme;   // Tasks 2-5 add client, format, icon, rate, display, menu, install
  pub const PENDING_S: u64 = 10;
  pub const NOTE_MS: u64 = 3_000;
  pub enum Action { Connect, Disconnect, ShowThumbs, HideThumbs, Focus(usize), Quit, Preferences, StartDaemon }
  impl Action { pub fn request(&self) -> Option<crate::ipc::Request>; }
  pub fn poll_interval(popup_open: bool) -> std::time::Duration;
  pub fn note_visible(set_at_ms: u64, now_ms: u64) -> bool;
  // src/applet/theme.rs — colour consts, size consts, style helpers (listed in Step 5)
  ```

**Why the split is shaped this way.** `src/ui/*`, `src/cli.rs`, `src/shortcuts.rs` and plan A's `src/adopt.rs` all say `crate::ipc::…`, `crate::model::…`, `crate::tunnel::…`. A plain `use yutani::{ipc, model, tunnel};` at the root of `src/main.rs` re-binds those exact paths inside the binary crate (a private `use` item is visible to the module it is in *and all its descendants*), so **not one line of `src/ui/`, `src/cli.rs`, `src/shortcuts.rs` or `src/adopt.rs` changes**. Everything else (`backend`, `cli`, `doctor`, `shortcuts`, `ui`, plan A's `adopt`, `launch`) stays a binary-private `mod`, so the applet does not link the capture backend. No `#[allow(dead_code)]` is needed anywhere: the moved modules are `pub` in a library.

- [ ] **Step 1: Cargo.toml — add the `applet` feature**

In `Cargo.toml`, the libcosmic feature list becomes (add `"applet",` as the last entry):

```toml
libcosmic = { git = "https://github.com/pop-os/libcosmic", rev = "a401af8b1c54a8abd393b8c5b7c8809402f83850", default-features = false, features = [
    "tokio",
    "wayland",
    "multi-window",
    "winit",
    "wgpu",
    "single-instance",
    "applet",
] }
```

`single-instance` stays: the *daemon* uses `cosmic::app::run_single_instance`. The applet must **not** use it — `cosmic::applet::run` is a separate entry point with no single-instance logic (cosmic-panel spawns one applet process per panel slot).

Then:

```bash
cargo build -q 2>&1 | tail -5
git diff Cargo.lock | grep '^[-+]name =' | sort | uniq -c
```
Expected: exactly the seven `+name =` lines listed in the Global Constraints (`build_helpers`, `cosmic-config`, `cosmic-config-derive`, `cosmic-panel-config`, `iced_core`, `iced_futures`, `xdg-shell-wrapper-config`) and no `-name =` line. The build is long (a second copy of `iced_core`/`iced_futures` compiles) and must end warning-free.

- [ ] **Step 2: The eight SVG assets**

These are the handoff's production icons. Two deliberate, documented deviations from the bundle's bytes:
1. the `<metadata>` C2PA blob (≈7.8 KB per file) and the now-unused `xmlns:c2pa` attribute are dropped — the drawing is byte-identical and the repo diff stays readable;
2. `y-sync-symbolic.svg`'s badge, drawn in the bundle as a white disc with a black disc on top, becomes **one** path with `fill-rule="evenodd"` and the identical geometry (outer r=15, inner r=7, centre 79,79). A symbolic icon is recoloured by a single colour filter over the whole SVG (`iced/widget/src/svg.rs`: `if self.symbolic && style.color.is_none() { style.color = Some(renderer_style.icon_color) }`), which would flatten a black-on-white hole into a solid dot; an even-odd hole stays a ring under any tint, which is what the handoff's "ring badge" asks for.

Create the files exactly:

`assets/icons/y-symbolic.svg`
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" width="96" height="96">
  <path fill="#ffffff" fill-rule="evenodd" d="M10.1 2 L27.6 2 L27.6 6 L32.2 6 L53.2 31.8 L71.2 2.6 L85.8 2.6 L85.8 6.2 L82.6 6.2 L69.3 28.4 L66.8 28.4 L66.8 31.6 L59.9 44 L59.9 46 L58.5 46 L58.5 49.5 L59.9 49.5 L59.9 80.1 L45.5 94 L40.4 80.4 L40.4 57 L39 57 L39 55.5 L32 32.5 L29.5 32.5 L18.9 10.2 L10.1 10.2 Z M37 80 L63 57.5 L63 59.1 L37 81.6 Z M41 78.6 L41.8 78.6 L41.8 81.8 L41 81.8 Z"></path>
</svg>
```

`assets/icons/y-symbolic-dark.svg` — identical but `fill="#2c2c2c"`:
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" width="96" height="96">
  <path fill="#2c2c2c" fill-rule="evenodd" d="M10.1 2 L27.6 2 L27.6 6 L32.2 6 L53.2 31.8 L71.2 2.6 L85.8 2.6 L85.8 6.2 L82.6 6.2 L69.3 28.4 L66.8 28.4 L66.8 31.6 L59.9 44 L59.9 46 L58.5 46 L58.5 49.5 L59.9 49.5 L59.9 80.1 L45.5 94 L40.4 80.4 L40.4 57 L39 57 L39 55.5 L32 32.5 L29.5 32.5 L18.9 10.2 L10.1 10.2 Z M37 80 L63 57.5 L63 59.1 L37 81.6 Z M41 78.6 L41.8 78.6 L41.8 81.8 L41 81.8 Z"></path>
</svg>
```

`assets/icons/y-color.svg` — identical but `fill="#0A5CFF"` (launcher/about icon, never the panel):
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" width="96" height="96">
  <path fill="#0A5CFF" fill-rule="evenodd" d="M10.1 2 L27.6 2 L27.6 6 L32.2 6 L53.2 31.8 L71.2 2.6 L85.8 2.6 L85.8 6.2 L82.6 6.2 L69.3 28.4 L66.8 28.4 L66.8 31.6 L59.9 44 L59.9 46 L58.5 46 L58.5 49.5 L59.9 49.5 L59.9 80.1 L45.5 94 L40.4 80.4 L40.4 57 L39 57 L39 55.5 L32 32.5 L29.5 32.5 L18.9 10.2 L10.1 10.2 Z M37 80 L63 57.5 L63 59.1 L37 81.6 Z M41 78.6 L41.8 78.6 L41.8 81.8 L41 81.8 Z"></path>
</svg>
```

`assets/icons/y-sync-symbolic.svg`
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" width="96" height="96">
  <path fill="#ffffff" fill-rule="evenodd" d="M10.1 2 L27.6 2 L27.6 6 L32.2 6 L53.2 31.8 L71.2 2.6 L85.8 2.6 L85.8 6.2 L82.6 6.2 L69.3 28.4 L66.8 28.4 L66.8 31.6 L59.9 44 L59.9 46 L58.5 46 L58.5 49.5 L59.9 49.5 L59.9 80.1 L45.5 94 L40.4 80.4 L40.4 57 L39 57 L39 55.5 L32 32.5 L29.5 32.5 L18.9 10.2 L10.1 10.2 Z M37 80 L63 57.5 L63 59.1 L37 81.6 Z M41 78.6 L41.8 78.6 L41.8 81.8 L41 81.8 Z"></path>
  <path fill="#ffffff" fill-rule="evenodd" d="M79 64 a15 15 0 1 0 0 30 a15 15 0 1 0 0 -30 Z M79 72 a7 7 0 1 0 0 14 a7 7 0 1 0 0 -14 Z"></path>
</svg>
```

`assets/icons/y-attention-symbolic.svg`
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" width="96" height="96">
  <path fill="#ffffff" fill-rule="evenodd" d="M10.1 2 L27.6 2 L27.6 6 L32.2 6 L53.2 31.8 L71.2 2.6 L85.8 2.6 L85.8 6.2 L82.6 6.2 L69.3 28.4 L66.8 28.4 L66.8 31.6 L59.9 44 L59.9 46 L58.5 46 L58.5 49.5 L59.9 49.5 L59.9 80.1 L45.5 94 L40.4 80.4 L40.4 57 L39 57 L39 55.5 L32 32.5 L29.5 32.5 L18.9 10.2 L10.1 10.2 Z M37 80 L63 57.5 L63 59.1 L37 81.6 Z M41 78.6 L41.8 78.6 L41.8 81.8 L41 81.8 Z"></path>
  <rect x="72" y="62" width="12" height="20" rx="2" fill="#ffffff"></rect>
  <rect x="72" y="86" width="12" height="12" rx="2" fill="#ffffff"></rect>
</svg>
```

`assets/glyphs/pin.svg` (handoff's map-pin: 10×12, stroke `#9096A0`, width 1.3)
```svg
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="12" viewBox="0 0 10 12" fill="none" stroke="#9096A0" stroke-width="1.3"><path d="M5 11.2C7 8.8 8.6 6.9 8.6 4.9A3.6 3.6 0 0 0 1.4 4.9C1.4 6.9 3 8.8 5 11.2Z" stroke-linejoin="round"></path><circle cx="5" cy="4.8" r="1.3"></circle></svg>
```

`assets/glyphs/arrow-up.svg` (upload, stroke `#2FD6B0`, width 1.8, round caps)
```svg
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="#2FD6B0" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 10V2M2.5 5.5 6 2l3.5 3.5"></path></svg>
```

`assets/glyphs/arrow-down.svg` (download, stroke `#5B9BFF`)
```svg
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="#5B9BFF" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2v8M2.5 6.5 6 10l3.5-3.5"></path></svg>
```

- [ ] **Step 3: Write the failing tests for `src/assets.rs`**

Create `src/assets.rs` containing **only** this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_single_colour_svg_named_for_the_icon_theme() {
        assert_eq!(ICONS.len(), 5);
        for (name, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).expect("svg is utf-8");
            assert!(name.ends_with(".svg"), "{name}");
            assert!(text.starts_with("<svg "), "{name} must start with <svg ");
            assert!(text.contains(r#"viewBox="0 0 96 96""#), "{name} keeps the 96×96 box");
            assert!(!text.contains("c2pa"), "{name} must have the metadata blob stripped");
        }
        assert!(ICONS.iter().any(|(n, _)| *n == "y-symbolic.svg"));
        assert!(ICONS.iter().any(|(n, _)| *n == "y-color.svg"));
    }

    #[test]
    fn the_tinted_panel_icons_are_white_and_the_launcher_icon_is_blue() {
        for bytes in [Y_SYMBOLIC, Y_SYNC_SYMBOLIC, Y_ATTENTION_SYMBOLIC] {
            assert!(std::str::from_utf8(bytes).unwrap().contains(r##"fill="#ffffff""##));
        }
        assert!(std::str::from_utf8(Y_SYMBOLIC_DARK).unwrap().contains(r##"fill="#2c2c2c""##));
        assert!(std::str::from_utf8(Y_COLOR).unwrap().contains(r##"fill="#0A5CFF""##));
        // The sync badge must be one even-odd path, or a symbolic tint fills its hole.
        let sync = std::str::from_utf8(Y_SYNC_SYMBOLIC).unwrap();
        assert!(!sync.contains("<circle"), "sync badge must be an even-odd ring, not two circles");
        assert_eq!(sync.matches("fill-rule=\"evenodd\"").count(), 2);
    }

    #[test]
    fn the_popup_glyphs_carry_the_handoff_stroke_colours() {
        assert!(std::str::from_utf8(PIN).unwrap().contains(r##"stroke="#9096A0""##));
        assert!(std::str::from_utf8(ARROW_UP).unwrap().contains(r##"stroke="#2FD6B0""##));
        assert!(std::str::from_utf8(ARROW_DOWN).unwrap().contains(r##"stroke="#5B9BFF""##));
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -q assets:: 2>&1 | tail -20`
Expected: FAIL — `error[E0433]: failed to resolve: use of undeclared crate or module` / `cannot find value ICONS in this scope` (`src/assets.rs` is not even a module yet, so the compile fails at `src/lib.rs` in the next step; before Step 5 the file is simply not compiled and the filter matches nothing — the definitive failure appears once `src/lib.rs` declares the module, which is why Step 5 adds both).

- [ ] **Step 5: Write the implementation — `src/lib.rs`, `src/assets.rs` body, `src/applet/mod.rs`, `src/applet/theme.rs`, `src/main.rs`**

Create `src/lib.rs`:

```rust
//! The Yutani library: everything shared by the `yutani` daemon/CLI binary
//! and the `yutani-applet` panel applet. Capture, the layer-shell UI and the
//! CLI plumbing stay private to `src/main.rs`; only the IPC protocol, the
//! status types, the user config, the icon assets and the applet's pure
//! model live here.

pub mod applet;
pub mod assets;
pub mod ipc;
pub mod model;
pub mod tunnel;
```

Prepend to `src/assets.rs` (above the test module written in Step 3):

```rust
//! The handoff's SVGs, compiled in. `yutani applet install` writes `ICONS`
//! into the user's icon theme so cosmic-panel can show the applet in its
//! list; the applet itself always draws from these bytes, so it looks right
//! straight from `cargo run` with nothing installed.

pub const Y_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-symbolic.svg");
pub const Y_SYMBOLIC_DARK: &[u8] = include_bytes!("../assets/icons/y-symbolic-dark.svg");
pub const Y_SYNC_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-sync-symbolic.svg");
pub const Y_ATTENTION_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-attention-symbolic.svg");
pub const Y_COLOR: &[u8] = include_bytes!("../assets/icons/y-color.svg");

/// Popup decorations. Not icon-theme icons: they carry fixed colours from
/// the handoff and are never tinted, so they are not installed.
pub const PIN: &[u8] = include_bytes!("../assets/glyphs/pin.svg");
pub const ARROW_UP: &[u8] = include_bytes!("../assets/glyphs/arrow-up.svg");
pub const ARROW_DOWN: &[u8] = include_bytes!("../assets/glyphs/arrow-down.svg");

/// What `yutani applet install` copies to the hicolor symbolic apps dir.
pub const ICONS: [(&str, &[u8]); 5] = [
    ("y-symbolic.svg", Y_SYMBOLIC),
    ("y-symbolic-dark.svg", Y_SYMBOLIC_DARK),
    ("y-sync-symbolic.svg", Y_SYNC_SYMBOLIC),
    ("y-attention-symbolic.svg", Y_ATTENTION_SYMBOLIC),
    ("y-color.svg", Y_COLOR),
];
```

Create `src/applet/mod.rs`:

```rust
//! The applet's pure model. Everything the panel applet decides — what to
//! poll, what to show, which icon, which menu rows, what an action sends —
//! lives here and is unit-tested; `src/bin/yutani-applet/` only renders it.
//! See docs/superpowers/specs/2026-09-12-yutani-applet-design.md.

// Each later task adds its own module here; Task 1 ships only `theme`.
pub mod theme;

use std::time::Duration;

/// How long after a `tunnel connect|disconnect` the icon shows the sync
/// state even if the daemon has not caught up yet (spec §3).
pub const PENDING_S: u64 = 10;

/// How long an `err …` note stays under the menu (spec §7).
pub const NOTE_MS: u64 = 3_000;

/// A menu press. `request()` is `None` for the two actions the applet
/// performs itself instead of asking the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Connect,
    Disconnect,
    ShowThumbs,
    HideThumbs,
    /// 1-based index in the daemon's layout order — the order `status`
    /// lists clients in.
    Focus(usize),
    Quit,
    /// Open `~/.config/yutani/config.ron` with `xdg-open`.
    Preferences,
    /// Spawn the daemon (offline state only).
    StartDaemon,
}

impl Action {
    pub fn request(&self) -> Option<crate::ipc::Request> {
        use crate::ipc::Request;
        match self {
            Action::Connect => Some(Request::TunnelConnect),
            Action::Disconnect => Some(Request::TunnelDisconnect),
            Action::ShowThumbs => Some(Request::Show),
            Action::HideThumbs => Some(Request::Hide),
            Action::Focus(n) => Some(Request::Focus(*n)),
            Action::Quit => Some(Request::Quit),
            Action::Preferences | Action::StartDaemon => None,
        }
    }
}

/// 1 s with the popup open, 5 s with it closed (spec §2). The applet's
/// timer subscription is keyed on this duration, so flipping it restarts
/// the timer — which is exactly the intent.
pub fn poll_interval(popup_open: bool) -> Duration {
    Duration::from_secs(if popup_open { 1 } else { 5 })
}

/// An error note is shown for [`NOTE_MS`] after it was set.
pub fn note_visible(set_at_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(set_at_ms) < NOTE_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Request;

    #[test]
    fn actions_map_onto_the_ipc_protocol() {
        assert_eq!(Action::Connect.request(), Some(Request::TunnelConnect));
        assert_eq!(Action::Disconnect.request(), Some(Request::TunnelDisconnect));
        assert_eq!(Action::ShowThumbs.request(), Some(Request::Show));
        assert_eq!(Action::HideThumbs.request(), Some(Request::Hide));
        assert_eq!(Action::Focus(3).request(), Some(Request::Focus(3)));
        assert_eq!(Action::Quit.request(), Some(Request::Quit));
        assert_eq!(Action::Preferences.request(), None);
        assert_eq!(Action::StartDaemon.request(), None);
    }

    #[test]
    fn poll_is_one_second_open_and_five_closed() {
        assert_eq!(poll_interval(true), Duration::from_secs(1));
        assert_eq!(poll_interval(false), Duration::from_secs(5));
    }

    #[test]
    fn a_note_lives_for_three_seconds() {
        assert!(note_visible(1_000, 1_000));
        assert!(note_visible(1_000, 3_999));
        assert!(!note_visible(1_000, 4_000));
        assert!(!note_visible(1_000, 9_999));
        // A clock that went backwards must not make the note immortal.
        assert!(note_visible(5_000, 1_000));
    }
}
```

Create `src/applet/theme.rs` — the one place the handoff's numbers live:

```rust
//! The handoff's design tokens. Colours are the bundle's hex literals,
//! converted once here; sizes are its px values. Nothing else in the
//! codebase may repeat them.

use cosmic::iced::border::Radius;
use cosmic::iced::{Background, Color, Shadow, Vector};
use cosmic::widget::button;
use cosmic::widget::container;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 1.0)
}
const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, a)
}

// ---- colours (handoff "Design tokens") ----
/// `#F2F4F7` — titles, counts, tile totals.
pub const TEXT_PRIMARY: Color = rgb(0xF2, 0xF4, 0xF7);
/// `#E6E8EC` — the Y mark in the header and menu labels.
pub const TEXT_ON_SURFACE: Color = rgb(0xE6, 0xE8, 0xEC);
/// `#C2C7CF` — the location and the accounts label.
pub const TEXT_SECONDARY: Color = rgb(0xC2, 0xC7, 0xCF);
/// `#8A8F98` — the disconnected dot and status text.
pub const TEXT_MUTED: Color = rgb(0x8A, 0x8F, 0x98);
/// `#7D838C` — the interface chip, tile labels, menu hints, IP/handshake.
pub const TEXT_FAINT: Color = rgb(0x7D, 0x83, 0x8C);
/// `#4A4F57` — the "·" between status and location.
pub const SEPARATOR: Color = rgb(0x4A, 0x4F, 0x57);
/// `#2FD6B0` — connected, upload.
pub const ACCENT_UP: Color = rgb(0x2F, 0xD6, 0xB0);
/// `#2FD6B0AA` — the connected dot's glow.
pub const ACCENT_UP_GLOW: Color = rgba(0x2F, 0xD6, 0xB0, 0xAA as f32 / 255.0);
/// `#5B9BFF` — download.
pub const ACCENT_DOWN: Color = rgb(0x5B, 0x9B, 0xFF);
/// `#FF8A7E` — the Quit label.
pub const DANGER_TEXT: Color = rgb(0xFF, 0x8A, 0x7E);
/// `#FF6B5C1F` — the Quit hover fill.
pub const DANGER_HOVER: Color = rgba(0xFF, 0x6B, 0x5C, 0x1F as f32 / 255.0);
/// `#FFFFFF08` — tile fill.
pub const TILE_FILL: Color = rgba(0xFF, 0xFF, 0xFF, 0x08 as f32 / 255.0);
/// `#FFFFFF0F` — tile border and the interface chip's background.
pub const CHIP_FILL: Color = rgba(0xFF, 0xFF, 0xFF, 0x0F as f32 / 255.0);
/// `#FFFFFF12` — hairline dividers and menu hover.
pub const HAIRLINE: Color = rgba(0xFF, 0xFF, 0xFF, 0x12 as f32 / 255.0);

// ---- sizes (handoff "Screen: applet popup") ----
pub const POPUP_PADDING: u16 = 6;
pub const HEADER_GAP: u16 = 11;
pub const HEADER_COLUMN_GAP: u16 = 5;
pub const MARK_PX: u16 = 24;
pub const TITLE_SIZE: f32 = 14.0;
pub const STATUS_SIZE: f32 = 11.5;
pub const STATUS_GAP: u16 = 7;
pub const DOT_PX: f32 = 7.0;
pub const DOT_GLOW_BLUR: f32 = 8.0;
pub const CHIP_SIZE: f32 = 11.0;
pub const CHIP_RADIUS: f32 = 6.0;
pub const COUNT_SIZE: f32 = 22.0;
pub const COUNT_LABEL_SIZE: f32 = 13.0;
pub const BAND_RIGHT_SIZE: f32 = 10.5;
pub const TILE_GAP: u16 = 6;
pub const TILE_RADIUS: f32 = 10.0;
pub const TILE_COLUMN_GAP: u16 = 7;
pub const TILE_LABEL_SIZE: f32 = 10.0;
pub const TILE_TOTAL_SIZE: f32 = 16.0;
pub const TILE_RATE_SIZE: f32 = 11.0;
pub const GLYPH_PX: u16 = 10;
pub const MENU_SIZE: f32 = 13.5;
pub const MENU_HINT_SIZE: f32 = 10.5;
pub const MENU_RADIUS: f32 = 8.0;
/// The panel icon's opacity in the daemon-offline / not-installed state.
pub const DIM_OPACITY: f32 = 0.38;

/// A 1 px `#FFFFFF12` rule. The handoff's dividers, not COSMIC's.
pub fn hairline<'a, M: 'a>() -> cosmic::Element<'a, M> {
    cosmic::widget::container(
        cosmic::widget::space()
            .width(cosmic::iced::Length::Fill)
            .height(cosmic::iced::Length::Fixed(1.0)),
    )
    .class(cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(HAIRLINE)),
        ..Default::default()
    }))
    .into()
}

/// A filled, optionally glowing round dot (the header status dot).
pub fn dot_class(color: Color, glow: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |_| container::Style {
        background: Some(Background::Color(color)),
        border: cosmic::iced::Border { radius: Radius::from(DOT_PX / 2.0), ..Default::default() },
        shadow: if glow {
            Shadow { color: ACCENT_UP_GLOW, offset: Vector::ZERO, blur_radius: DOT_GLOW_BLUR }
        } else {
            Shadow::default()
        },
        ..Default::default()
    })
}

/// The interface chip behind `yutani0`.
pub fn chip_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(CHIP_FILL)),
        border: cosmic::iced::Border { radius: Radius::from(CHIP_RADIUS), ..Default::default() },
        ..Default::default()
    })
}

/// A traffic tile: `#FFFFFF08` on a 1 px `#FFFFFF0F` border, radius 10.
pub fn tile_class() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|_| container::Style {
        background: Some(Background::Color(TILE_FILL)),
        border: cosmic::iced::Border { color: CHIP_FILL, width: 1.0, radius: Radius::from(TILE_RADIUS) },
        ..Default::default()
    })
}

fn row_style(text: Color, fill: Option<Color>) -> button::Style {
    button::Style {
        background: fill.map(Background::Color),
        border_radius: Radius::from(MENU_RADIUS),
        text_color: Some(text),
        icon_color: Some(text),
        ..Default::default()
    }
}

/// A menu row: transparent, radius 8, `hover` fill on hover and press, and
/// 40 % text when disabled.
pub fn menu_row_class(text: Color, hover: Color) -> cosmic::theme::Button {
    let dim = Color { a: 0.4, ..text };
    cosmic::theme::Button::Custom {
        active: Box::new(move |_focused, _theme| row_style(text, None)),
        disabled: Box::new(move |_theme| row_style(dim, None)),
        hovered: Box::new(move |_focused, _theme| row_style(text, Some(hover))),
        pressed: Box::new(move |_focused, _theme| row_style(text, Some(hover))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::parse_color;

    /// Every colour const must equal the handoff's hex literal, parsed by
    /// the same code the config uses.
    #[test]
    fn colours_match_the_handoff_tokens() {
        let expect = |c: Color, hex: &str| {
            let [r, g, b, a] = parse_color(hex).expect(hex);
            assert!((c.r - r).abs() < 0.002 && (c.g - g).abs() < 0.002 && (c.b - b).abs() < 0.002 && (c.a - a).abs() < 0.002,
                "{hex}: got {c:?}");
        };
        expect(TEXT_PRIMARY, "#F2F4F7");
        expect(TEXT_ON_SURFACE, "#E6E8EC");
        expect(TEXT_SECONDARY, "#C2C7CF");
        expect(TEXT_MUTED, "#8A8F98");
        expect(TEXT_FAINT, "#7D838C");
        expect(SEPARATOR, "#4A4F57");
        expect(ACCENT_UP, "#2FD6B0");
        expect(ACCENT_UP_GLOW, "#2FD6B0AA");
        expect(ACCENT_DOWN, "#5B9BFF");
        expect(DANGER_TEXT, "#FF8A7E");
        expect(DANGER_HOVER, "#FF6B5C1F");
        expect(TILE_FILL, "#FFFFFF08");
        expect(CHIP_FILL, "#FFFFFF0F");
        expect(HAIRLINE, "#FFFFFF12");
    }

    #[test]
    fn sizes_match_the_handoff() {
        assert_eq!(POPUP_PADDING, 6);
        assert_eq!(MARK_PX, 24);
        assert_eq!((TITLE_SIZE, STATUS_SIZE, CHIP_SIZE), (14.0, 11.5, 11.0));
        assert_eq!((COUNT_SIZE, COUNT_LABEL_SIZE, BAND_RIGHT_SIZE), (22.0, 13.0, 10.5));
        assert_eq!((TILE_LABEL_SIZE, TILE_TOTAL_SIZE, TILE_RATE_SIZE), (10.0, 16.0, 11.0));
        assert_eq!((MENU_SIZE, MENU_HINT_SIZE, MENU_RADIUS), (13.5, 10.5, 8.0));
        assert_eq!(DIM_OPACITY, 0.38);
    }
}
```

Finally, `src/main.rs`: replace the three module declarations that moved into the library. The head of the file becomes:

```rust
mod backend;
mod cli;
mod doctor;
mod shortcuts;
mod ui;

// `ipc`, `model` and `tunnel` now live in the library (so `yutani-applet`
// can share them). Re-binding them at the binary's crate root keeps every
// `crate::ipc::…` / `crate::model::…` / `crate::tunnel::…` path in
// `src/ui/`, `src/cli.rs`, `src/shortcuts.rs` and `src/adopt.rs` working
// unchanged: a private `use` is visible to this module and its descendants.
use yutani::{ipc, model, tunnel};
```

(Plan A also added `mod adopt;` and `mod launch;` — keep those in the `mod` list, and delete plan A's `mod tunnel;` line, which this `use` replaces.)

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo build -q 2>&1 | tail -5          # no output, no warnings
cargo test -q 2>&1 | grep 'test result'
```
Expected: `test result: ok. 115 passed` (107 + 8: three in `assets`, three in `applet`, two in `applet::theme`). If plan A finished on a different total, the delta is what matters: **+8**.

Also confirm the library really is the only home of the moved modules:
```bash
grep -n "^mod ipc\|^mod model\|^mod tunnel" src/main.rs   # no output
```

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock assets src/lib.rs src/assets.rs src/applet src/main.rs
git commit -m "feat(applet): lib/bin split, handoff icon assets and the design-token table

The crate gains src/lib.rs exposing ipc, model, tunnel, assets and the new
pure applet model, so a second binary can share them; libcosmic gains the
applet feature. Colours and sizes from the design handoff live in exactly
one place, src/applet/theme.rs.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 2: The pure applet model — formatting, rates, icon state, and the IPC client

**Files:**
- Create: `src/applet/format.rs`, `src/applet/rate.rs`, `src/applet/icon.rs`, `src/applet/client.rs`
- Test: the `mod tests` block in each of those four files

**Interfaces:**
- Consumes: `crate::tunnel::status::{Status, TunnelStatus}` (plan A, Task 2), `crate::ipc::{MAX_LINE, Request, Response, socket_path}` with `Response::OkData(String)` (plan A, Task 4), `crate::applet::PENDING_S`.
- Produces:
  ```rust
  // src/applet/format.rs
  pub fn bytes(n: u64) -> String;                 // "0 KB" | "222 KB" | "413.1 MB" | "2.79 GB"
  pub fn rate(bytes_per_s: f64) -> String;        // "0 KB/s" | "222 KB/s" | "2.5 MB/s"
  pub fn handshake(age_s: Option<u64>) -> String; // "hs 21s ago" | "hs —"
  pub fn accounts_label(n: usize) -> &'static str;// "Account connected" | "Accounts connected"
  // src/applet/rate.rs
  #[derive(Clone, Copy, Debug, Default, PartialEq)] pub struct Rates { pub rx: f64, pub tx: f64 }
  #[derive(Clone, Copy, Debug, Default)] pub struct Sampler { /* private */ }
  impl Sampler { pub fn push(&mut self, rx: u64, tx: u64, t_ms: u64) -> Rates; pub fn reset(&mut self); }
  // src/applet/icon.rs
  pub const HANDSHAKE_STALE_S: u64 = 180;
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum IconState { Plain, Sync, Attention, Dim }
  pub fn icon_state(tunnel: Option<&TunnelStatus>, pending: bool) -> IconState;
  // src/applet/client.rs
  #[derive(Clone, Debug, PartialEq, Eq)] pub enum IpcError { Offline, Failed(String) }
  pub async fn send_to(path: &Path, request: &Request) -> Result<Option<String>, IpcError>;
  pub async fn send(request: Request) -> Result<Option<String>, IpcError>;
  pub async fn status_from(path: PathBuf) -> Result<Status, IpcError>;
  pub async fn status() -> Result<Status, IpcError>;
  ```

- [ ] **Step 1: Write the failing tests for `format`, `rate` and `icon`**

Create `src/applet/format.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_use_the_handoff_ladder() {
        assert_eq!(bytes(0), "0 KB");
        assert_eq!(bytes(999), "0 KB");
        assert_eq!(bytes(222_000), "222 KB");
        assert_eq!(bytes(999_999), "999 KB");
        assert_eq!(bytes(1_000_000), "1.0 MB");
        assert_eq!(bytes(413_100_000), "413.1 MB");
        assert_eq!(bytes(999_999_999), "1000.0 MB");
        assert_eq!(bytes(1_000_000_000), "1.00 GB");
        assert_eq!(bytes(2_790_000_000), "2.79 GB");
    }

    #[test]
    fn rates_use_the_handoff_ladder() {
        assert_eq!(rate(0.0), "0 KB/s");
        assert_eq!(rate(-5.0), "0 KB/s");
        assert_eq!(rate(222_000.0), "222 KB/s");
        assert_eq!(rate(999_999.0), "999 KB/s");
        assert_eq!(rate(1_000_000.0), "1.0 MB/s");
        assert_eq!(rate(2_500_000.0), "2.5 MB/s");
    }

    #[test]
    fn handshake_age_reads_as_the_handoff_writes_it() {
        assert_eq!(handshake(Some(0)), "hs 0s ago");
        assert_eq!(handshake(Some(21)), "hs 21s ago");
        assert_eq!(handshake(Some(112)), "hs 112s ago");
        assert_eq!(handshake(None), "hs —");
    }

    #[test]
    fn the_accounts_label_is_singular_for_one() {
        assert_eq!(accounts_label(0), "Accounts connected");
        assert_eq!(accounts_label(1), "Account connected");
        assert_eq!(accounts_label(2), "Accounts connected");
    }
}
```

Create `src/applet/rate.rs` with only:

```rust
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
```

Create `src/applet/icon.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::TunnelStatus;

    fn tunnel(installed: bool, connected: bool, handshake_age_s: Option<u64>) -> TunnelStatus {
        TunnelStatus {
            installed,
            connected,
            iface: "yutani0".into(),
            location: "London".into(),
            address: connected.then(|| "10.2.0.2".to_string()),
            endpoint: connected.then(|| "198.51.100.10:51820".to_string()),
            handshake_age_s,
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }

    #[test]
    fn no_daemon_and_no_tunnel_are_both_dim() {
        assert_eq!(icon_state(None, false), IconState::Dim);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), false), IconState::Dim);
        // Even a pending action cannot brighten a missing daemon.
        assert_eq!(icon_state(None, true), IconState::Dim);
    }

    #[test]
    fn a_pending_action_shows_the_sync_icon() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), true), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), true), IconState::Sync);
    }

    #[test]
    fn a_deliberate_disconnect_and_a_fresh_handshake_are_both_plain() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(0))), false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(179))), false), IconState::Plain);
    }

    #[test]
    fn up_without_a_handshake_syncs_and_a_stale_handshake_demands_attention() {
        assert_eq!(icon_state(Some(&tunnel(true, true, None)), false), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(180))), false), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(6_000))), false), IconState::Attention);
    }

    #[test]
    fn each_state_names_one_of_the_installed_icons() {
        assert_eq!(IconState::Plain.icon_name(), "y-symbolic");
        assert_eq!(IconState::Sync.icon_name(), "y-sync-symbolic");
        assert_eq!(IconState::Attention.icon_name(), "y-attention-symbolic");
        assert_eq!(IconState::Dim.icon_name(), "y-symbolic");
        assert_eq!(IconState::Plain.opacity(), 1.0);
        assert_eq!(IconState::Dim.opacity(), 0.38);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod client;`, `pub mod format;`, `pub mod icon;` and `pub mod rate;` to the module list at the top of `src/applet/mod.rs` (keeping it alphabetical: `client, format, icon, rate, theme`).

Run: `cargo test -q applet:: 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function bytes in this scope`, `cannot find struct Sampler in this scope`, `cannot find function icon_state in this scope`, `failed to resolve: could not find client in applet`.

- [ ] **Step 3: Write the implementations for `format`, `rate` and `icon`**

Prepend to `src/applet/format.rs`:

```rust
//! Number and label formatting, exactly as the design handoff specifies:
//! decimal SI (matching `wg`'s human output style), ≥1e9 → `X.XX GB`,
//! ≥1e6 → `X.X MB`, else `N KB`; rates ≥1e6 → `X.X MB/s`, else `N KB/s`.
//! Sub-unit values truncate rather than round, so 999 999 B reads
//! `999 KB` and never the nonsensical `1000 KB`.

/// A cumulative byte counter.
pub fn bytes(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2} GB", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else {
        format!("{} KB", n / 1_000)
    }
}

/// A transfer rate in bytes per second. Negative (impossible) input and
/// everything under 1 kB/s read `0 KB/s`.
pub fn rate(bytes_per_s: f64) -> String {
    let b = if bytes_per_s.is_finite() && bytes_per_s > 0.0 { bytes_per_s } else { 0.0 };
    if b >= 1_000_000.0 {
        format!("{:.1} MB/s", b / 1_000_000.0)
    } else {
        format!("{} KB/s", (b / 1_000.0) as u64)
    }
}

/// Seconds since the last handshake, or the em-dash when there is none.
pub fn handshake(age_s: Option<u64>) -> String {
    match age_s {
        Some(age) => format!("hs {age}s ago"),
        None => "hs —".to_string(),
    }
}

/// The handoff's copy, singular only for exactly one account.
pub fn accounts_label(n: usize) -> &'static str {
    if n == 1 { "Account connected" } else { "Accounts connected" }
}
```

Prepend to `src/applet/rate.rs`:

```rust
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
```

Prepend to `src/applet/icon.rs`:

```rust
//! Which panel icon the tunnel's state calls for (spec §3).

use crate::tunnel::status::TunnelStatus;

/// A tunnel that is up but has not handshaked for this long is in trouble.
pub const HANDSHAKE_STALE_S: u64 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// The plain mark: connected and healthy, or deliberately disconnected.
    Plain,
    /// Ring badge: an action is in flight, or the tunnel is up and still
    /// handshaking.
    Sync,
    /// Exclamation badge: up but no handshake for `HANDSHAKE_STALE_S`.
    Attention,
    /// The plain mark at 38 %: no daemon, or no tunnel installed.
    Dim,
}

impl IconState {
    /// The icon-theme name `yutani applet install` writes (the applet draws
    /// the same bytes from `crate::assets`).
    pub fn icon_name(self) -> &'static str {
        match self {
            IconState::Sync => "y-sync-symbolic",
            IconState::Attention => "y-attention-symbolic",
            IconState::Plain | IconState::Dim => "y-symbolic",
        }
    }

    pub fn opacity(self) -> f32 {
        if self == IconState::Dim { super::theme::DIM_OPACITY } else { 1.0 }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            IconState::Sync => crate::assets::Y_SYNC_SYMBOLIC,
            IconState::Attention => crate::assets::Y_ATTENTION_SYMBOLIC,
            IconState::Plain | IconState::Dim => crate::assets::Y_SYMBOLIC,
        }
    }
}

/// `tunnel` is `None` when the daemon did not answer. `pending` is true for
/// [`super::PENDING_S`] after a `tunnel connect|disconnect` was sent.
pub fn icon_state(tunnel: Option<&TunnelStatus>, pending: bool) -> IconState {
    let Some(t) = tunnel else { return IconState::Dim };
    if !t.installed {
        return IconState::Dim;
    }
    if pending {
        return IconState::Sync;
    }
    match (t.connected, t.handshake_age_s) {
        (false, _) => IconState::Plain,
        (true, None) => IconState::Sync,
        (true, Some(age)) if age >= HANDSHAKE_STALE_S => IconState::Attention,
        (true, Some(_)) => IconState::Plain,
    }
}
```

- [ ] **Step 4: Write the failing test for the IPC client**

Create `src/applet/client.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    /// A current-thread runtime; `#[tokio::test]` would need the `macros`
    /// feature (and a new lock entry), which this crate does not have.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap().block_on(f)
    }

    fn socket(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-client-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("yutani.sock")
    }

    /// Accept exactly one connection, read the request line, answer `reply`.
    /// Returns the request line the client sent.
    async fn one_shot(path: &std::path::Path, reply: &'static str) -> tokio::task::JoinHandle<String> {
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            write.write_all(reply.as_bytes()).await.unwrap();
            write.shutdown().await.unwrap();
            line
        })
    }

    fn sample_status() -> Status {
        Status {
            clients: vec![ClientStatus { name: "KestrelVance".into(), active: true }],
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected: true,
                iface: "yutani0".into(),
                location: "London".into(),
                address: Some("10.2.0.2".into()),
                endpoint: Some("198.51.100.10:51820".into()),
                handshake_age_s: Some(21),
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
        }
    }

    #[test]
    fn a_missing_socket_means_the_daemon_is_offline() {
        let path = socket("absent");
        let _ = std::fs::remove_file(&path);
        let err = block_on(status_from(path)).unwrap_err();
        assert_eq!(err, IpcError::Offline);
    }

    #[test]
    fn status_round_trips_through_the_line_protocol() {
        let path = socket("ok");
        let json = serde_json::to_string(&sample_status()).unwrap();
        let reply: &'static str = Box::leak(format!("ok {json}\n").into_boxed_str());
        let got = block_on(async {
            let server = one_shot(&path, reply).await;
            let status = status_from(path.clone()).await;
            (server.await.unwrap(), status)
        });
        assert_eq!(got.0, "status\n");
        let status = got.1.unwrap();
        assert_eq!(status.clients[0].name, "KestrelVance");
        assert!(status.clients[0].active);
        assert_eq!(status.tunnel.handshake_age_s, Some(21));
        assert_eq!(status.tunnel.tx_bytes, 2_790_000_000);
    }

    #[test]
    fn an_err_reply_becomes_a_failure_and_a_plain_ok_carries_no_data() {
        let path = socket("err");
        let got = block_on(async {
            let server = one_shot(&path, "err no client 3 (2 known)\n").await;
            let r = send_to(&path, &crate::ipc::Request::Focus(3)).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "focus 3\n");
        assert_eq!(got.1.unwrap_err(), IpcError::Failed("no client 3 (2 known)".into()));

        let path = socket("bare");
        let got = block_on(async {
            let server = one_shot(&path, "ok\n").await;
            let r = send_to(&path, &crate::ipc::Request::Hide).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "hide\n");
        assert_eq!(got.1.unwrap(), None);
    }

    #[test]
    fn a_reply_that_is_not_json_is_a_failure_not_a_panic() {
        let path = socket("garbage");
        let got = block_on(async {
            let server = one_shot(&path, "ok not json at all\n").await;
            let r = status_from(path.clone()).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "status\n");
        assert!(matches!(got.1.unwrap_err(), IpcError::Failed(m) if m.starts_with("status:")));
    }
}
```

- [ ] **Step 5: Run the client tests to verify they fail**

Run: `cargo test -q applet::client 2>&1 | tail -10`
Expected: FAIL — `cannot find function status_from in this scope`, `cannot find function send_to in this scope`, `failed to resolve: use of undeclared type IpcError`.

- [ ] **Step 6: Write the IPC client**

Prepend to `src/applet/client.rs`:

```rust
//! The applet's side of the IPC socket: connect, write one request line,
//! read one reply line, close. A connect that finds nothing listening is
//! the daemon-offline signal, not an error to show.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::{MAX_LINE, Request, Response, socket_path};
use crate::tunnel::status::Status;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcError {
    /// Nothing is listening on the socket: the daemon is not running.
    Offline,
    /// The daemon answered `err <msg>`, or the exchange itself failed.
    Failed(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Offline => f.write_str("yutani is not running"),
            IpcError::Failed(msg) => f.write_str(msg),
        }
    }
}

/// One request, one reply. `Ok(None)` is a bare `ok`; `Ok(Some(data))` is
/// `ok <data>` (plan A's `Response::OkData`).
pub async fn send_to(path: &Path, request: &Request) -> Result<Option<String>, IpcError> {
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(err)
            if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) =>
        {
            return Err(IpcError::Offline);
        }
        Err(err) => return Err(IpcError::Failed(format!("connect: {err}"))),
    };
    let (read, mut write) = stream.into_split();
    write
        .write_all(request.to_line().as_bytes())
        .await
        .map_err(|err| IpcError::Failed(format!("send: {err}")))?;
    let mut line = String::new();
    BufReader::new(read.take(MAX_LINE as u64))
        .read_line(&mut line)
        .await
        .map_err(|err| IpcError::Failed(format!("reply: {err}")))?;
    match Response::parse(&line) {
        Response::Ok => Ok(None),
        Response::OkData(data) => Ok(Some(data)),
        Response::Err(msg) => Err(IpcError::Failed(msg)),
    }
}

/// `send_to` against `$XDG_RUNTIME_DIR/yutani.sock`.
pub async fn send(request: Request) -> Result<Option<String>, IpcError> {
    send_to(&socket_path(), &request).await
}

/// The `status` request, parsed. Takes an owned path so the future is
/// `'static` and can be handed to `Task::future`.
pub async fn status_from(path: PathBuf) -> Result<Status, IpcError> {
    let data = send_to(&path, &Request::Status)
        .await?
        .ok_or_else(|| IpcError::Failed("status: daemon replied ok with no data".to_string()))?;
    serde_json::from_str(&data).map_err(|err| IpcError::Failed(format!("status: {err}")))
}

pub async fn status() -> Result<Status, IpcError> {
    status_from(socket_path()).await
}
```

- [ ] **Step 7: Run all the tests**

```bash
cargo build -q 2>&1 | tail -5              # silent
cargo test -q 2>&1 | grep 'test result'
```
Expected: `132 passed` — **+17** on Task 1's total (4 in `format`, 4 in `rate`, 5 in `icon`, 4 in `client`). The delta is what matters.

- [ ] **Step 8: Commit**

```bash
git add src/applet/format.rs src/applet/rate.rs src/applet/icon.rs src/applet/client.rs
git commit -m "feat(applet): formatting, rate sampling, icon states and the IPC client

Pure model, fully unit-tested: the handoff's byte/rate ladder, deltas with
counter-reset handling, the four panel icon states, and a tokio client for
the daemon socket whose connect failure is the offline signal.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 3: The applet binary — panel button, popup lifecycle, polling, header / accounts band / traffic tiles

**Files:**
- Create: `src/applet/display.rs`, `src/bin/yutani-applet/main.rs`, `src/bin/yutani-applet/app.rs`, `src/bin/yutani-applet/view.rs`
- Modify: `src/applet/mod.rs` (add `pub mod display;`)
- Test: the `mod tests` block in `src/applet/display.rs`; the binary is verified by `cargo build --bin yutani-applet`

**Interfaces:**
- Consumes: `crate::tunnel::status::{Status, TunnelStatus}`; `crate::tunnel::IFACE`; `applet::{poll_interval, PENDING_S}`; `applet::client::{IpcError, status}`; `applet::format::{bytes, rate, handshake, accounts_label}`; `applet::icon::{IconState, icon_state, HANDSHAKE_STALE_S}`; `applet::rate::{Rates, Sampler}`; `applet::theme::*`; `crate::assets::{Y_SYMBOLIC, PIN, ARROW_UP, ARROW_DOWN}`.
- Produces:
  ```rust
  // src/applet/display.rs
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Display {
      pub online: bool, pub connected: bool, pub status_text: &'static str,
      pub location: String, pub iface: String,
      pub accounts: usize, pub accounts_label: &'static str,
      pub address: String, pub handshake: String,
      pub up_total: String, pub up_rate: String,
      pub down_total: String, pub down_rate: String,
      pub hidden: bool, pub installed: bool,
  }
  pub fn display(status: Option<&crate::tunnel::status::Status>, rates: crate::applet::rate::Rates) -> Display;
  // src/bin/yutani-applet/app.rs
  pub struct Applet { pub core: Core, pub popup: Option<Id>, pub status: Option<Status>,
                      pub rates: Rates, pub sampler: Sampler, pub started: Instant,
                      pub pending_until: Option<Instant>, pub accounts_open: bool,
                      pub note: Option<(String, u64)> }
  impl Applet { pub fn pending(&self) -> bool; pub fn now_ms(&self) -> u64; pub fn display(&self) -> Display; }
  pub enum Msg { Tick, Status(Result<Status, IpcError>), Press(Action), Done(Action, Result<(), String>),
                 ToggleAccounts, Surface(cosmic::surface::Action<Msg>), PopupClosed(Id) }
  // src/bin/yutani-applet/view.rs
  pub fn panel_button(state: &Applet) -> cosmic::Element<'_, Msg>;
  pub fn popup(state: &Applet) -> cosmic::Element<'_, Msg>;
  ```

**libcosmic facts this task relies on** (all verified against the pinned rev `a401af8…`, `src/applet/mod.rs` + `examples/applet/`):
- `pub fn cosmic::applet::run<App: Application>(flags: App::Flags) -> iced::Result` — takes flags only; window settings come from `Context::default()`, which parses `COSMIC_PANEL_SIZE`/`_ANCHOR`/`_SPACING`/`_BACKGROUND`/`_OUTPUT`/`_NAME`/`_PADDING_OVERLAP` (RON values) from the environment. It has **no** single-instance logic; do not call `run_single_instance`.
- `core.applet` is a `cosmic::applet::Context`. `anchor` and `size` are **public fields, not methods**. There is **no** `Context::get_popup` and **no** `Context::destroy_popup` at this rev. What exists: `suggested_size(&self, is_symbolic: bool) -> (u16, u16)`, `suggested_padding(&self, is_symbolic: bool) -> (u16, u16)`, `is_horizontal(&self) -> bool`, `icon_button(&self, &'a str)`, `icon_button_from_handle(&self, widget::icon::Handle)`, `button_from_element(&self, impl Into<Element<'a, M>>, use_symbolic_size: bool)`, `popup_container(&self, impl Into<Element<'a, M>>) -> Autosize<…>`, `get_popup_settings(&self, parent: window::Id, id: window::Id, size: Option<(u32,u32)>, width_padding: Option<i32>, height_padding: Option<i32>) -> SctkPopupSettings`, `theme()`, `text()`.
- Popups are opened through `cosmic::surface::action::app_popup::<App>(live_settings, settings, view)` and closed with `cosmic::surface::action::destroy_popup(id)`, both wrapped in a `Msg::Surface(..)` variant that `update` re-emits as `cosmic::task::message(cosmic::Action::Surface(a))`. The popup's contents come from `app_popup`'s boxed view closure — **not** from `view_window`.
- `cosmic::widget::Button::on_press_with_rectangle(impl Fn(Vector, Rectangle) -> Message + 'a)` gives the button's own bounds for the popup anchor.
- `Application::style(&self) -> Option<cosmic::iced::theme::Style>` should return `Some(cosmic::applet::style())`, which sets `icon_color` to the panel foreground — that, plus `Handle::symbolic(true)`, is what tints the Y mark on light and dark panels.
- `cosmic::iced::time::every(Duration) -> Subscription<Instant>` is the timer; the subscription is keyed on the duration, so returning 1 s vs 5 s restarts it — which is exactly the cadence switch we want.
- `popup_container` hard-codes `min_width(360.0).max_width(360.0).max_height(1000.0)` and paints the popup's own surface, blur, radius and shadow.

- [ ] **Step 1: Write the failing test for `Display`**

Create `src/applet/display.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::applet::rate::Rates;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    fn status(connected: bool, handshake_age_s: Option<u64>, clients: usize) -> Status {
        Status {
            clients: (0..clients)
                .map(|i| ClientStatus { name: format!("Pilot{i}"), active: i == 0 })
                .collect(),
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: Some("10.2.0.2".into()),
                endpoint: Some("198.51.100.10:51820".into()),
                handshake_age_s,
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
        }
    }

    #[test]
    fn a_healthy_tunnel_shows_everything_the_handoff_asks_for() {
        let s = status(true, Some(21), 3);
        let d = display(Some(&s), Rates { rx: 222_000.0, tx: 41_000.0 });
        assert!(d.online && d.connected && d.installed);
        assert_eq!(d.status_text, "Connected");
        assert_eq!(d.location, "London");
        assert_eq!(d.iface, "yutani0");
        assert_eq!((d.accounts, d.accounts_label), (3, "Accounts connected"));
        assert_eq!(d.address, "10.2.0.2");
        assert_eq!(d.handshake, "hs 21s ago");
        assert_eq!((d.up_total.as_str(), d.up_rate.as_str()), ("2.79 GB", "41 KB/s"));
        assert_eq!((d.down_total.as_str(), d.down_rate.as_str()), ("413.1 MB", "222 KB/s"));
        assert!(!d.hidden);
    }

    #[test]
    fn one_account_is_singular() {
        let s = status(true, Some(1), 1);
        assert_eq!(display(Some(&s), Rates::default()).accounts_label, "Account connected");
    }

    #[test]
    fn disconnecting_freezes_the_totals_and_zeroes_everything_live() {
        let s = status(false, None, 2);
        let d = display(Some(&s), Rates { rx: 999_000.0, tx: 999_000.0 });
        assert!(d.online && !d.connected);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!(d.address, "—");
        assert_eq!(d.handshake, "hs —");
        // Counters, not gauges: the totals stay where they stopped.
        assert_eq!((d.up_total.as_str(), d.down_total.as_str()), ("2.79 GB", "413.1 MB"));
        assert_eq!((d.up_rate.as_str(), d.down_rate.as_str()), ("0 KB/s", "0 KB/s"));
    }

    #[test]
    fn a_stale_handshake_reads_as_disconnected_even_though_the_link_is_up() {
        let s = status(true, Some(180), 1);
        let d = display(Some(&s), Rates::default());
        assert!(!d.connected);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!(d.handshake, "hs 180s ago");
        // The link really is up, so the address still shows.
        assert_eq!(d.address, "10.2.0.2");
    }

    #[test]
    fn the_offline_state_shows_dashes_and_zeroes() {
        let d = display(None, Rates { rx: 5.0, tx: 5.0 });
        assert!(!d.online && !d.connected && !d.installed);
        assert_eq!(d.status_text, "Disconnected");
        assert_eq!((d.location.as_str(), d.iface.as_str()), ("—", "yutani0"));
        assert_eq!((d.accounts, d.accounts_label), (0, "Accounts connected"));
        assert_eq!((d.address.as_str(), d.handshake.as_str()), ("—", "hs —"));
        assert_eq!((d.up_total.as_str(), d.down_total.as_str()), ("0 KB", "0 KB"));
        assert_eq!((d.up_rate.as_str(), d.down_rate.as_str()), ("0 KB/s", "0 KB/s"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod display;` to the module list at the top of `src/applet/mod.rs`.

Run: `cargo test -q applet::display 2>&1 | tail -10`
Expected: FAIL — `cannot find function display in this scope`, `cannot find struct Display in this scope`.

- [ ] **Step 3: Write `Display`**

Prepend to `src/applet/display.rs`:

```rust
//! Everything the popup's read-only area shows, as finished strings. The
//! view renders this and decides nothing; the rules live here where they
//! are tested. Spec §4.1–§4.3.

use crate::applet::format;
use crate::applet::icon::HANDSHAKE_STALE_S;
use crate::applet::rate::Rates;
use crate::tunnel::status::Status;

const DASH: &str = "—";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// The daemon answered.
    pub online: bool,
    /// The tunnel is up *and* handshaked within the last
    /// [`HANDSHAKE_STALE_S`] seconds — the handoff's "Connected" state,
    /// which drives the dot, its glow, the colours and the menu label.
    pub connected: bool,
    pub status_text: &'static str,
    pub location: String,
    pub iface: String,
    pub accounts: usize,
    pub accounts_label: &'static str,
    pub address: String,
    pub handshake: String,
    pub up_total: String,
    pub up_rate: String,
    pub down_total: String,
    pub down_rate: String,
    /// Thumbnails are hidden (drives the Show/Hide row).
    pub hidden: bool,
    /// A tunnel conf has been installed (a false disables Connect).
    pub installed: bool,
}

pub fn display(status: Option<&Status>, rates: Rates) -> Display {
    let Some(s) = status else {
        return Display {
            online: false,
            connected: false,
            status_text: "Disconnected",
            location: DASH.to_string(),
            iface: crate::tunnel::IFACE.to_string(),
            accounts: 0,
            accounts_label: format::accounts_label(0),
            address: DASH.to_string(),
            handshake: format::handshake(None),
            up_total: format::bytes(0),
            up_rate: format::rate(0.0),
            down_total: format::bytes(0),
            down_rate: format::rate(0.0),
            hidden: false,
            installed: false,
        };
    };
    let t = &s.tunnel;
    // "Connected" is link-up *and* a fresh handshake (spec §4.1); a link
    // that is up but stale reads Disconnected while the panel icon shows
    // the attention badge.
    let connected = t.connected && t.handshake_age_s.is_some_and(|age| age < HANDSHAKE_STALE_S);
    let live = |r: f64| if t.connected { format::rate(r) } else { format::rate(0.0) };
    Display {
        online: true,
        connected,
        status_text: if connected { "Connected" } else { "Disconnected" },
        location: t.location.clone(),
        iface: t.iface.clone(),
        accounts: s.clients.len(),
        accounts_label: format::accounts_label(s.clients.len()),
        address: match (t.connected, t.address.as_deref()) {
            (true, Some(addr)) => addr.to_string(),
            _ => DASH.to_string(),
        },
        handshake: format::handshake(if t.connected { t.handshake_age_s } else { None }),
        up_total: format::bytes(t.tx_bytes),
        up_rate: live(rates.tx),
        down_total: format::bytes(t.rx_bytes),
        down_rate: live(rates.rx),
        hidden: s.hidden,
        installed: t.installed,
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -q applet::display 2>&1 | tail -3`
Expected: `test result: ok. 5 passed`.

- [ ] **Step 5: Write the applet binary**

Create `src/bin/yutani-applet/main.rs`:

```rust
//! `yutani-applet`: the COSMIC panel applet. cosmic-panel spawns one
//! process per panel slot (see `yutani applet install`), so there is no
//! single-instance handling here — that belongs to the daemon.

mod app;
mod view;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,yutani=info,cosmic::theme=off,cosmic::app=error")
            }),
        )
        .with_writer(std::io::stderr)
        .init();
    cosmic::applet::run::<app::Applet>(())
}
```

Create `src/bin/yutani-applet/app.rs`:

```rust
//! The applet's `cosmic::Application`: poll `status`, keep the last reply,
//! render it, send actions back. No domain state of its own.

use std::time::{Duration, Instant};

use cosmic::app::{Core, Task};
use cosmic::iced::window::Id;
use cosmic::iced::{Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};

use yutani::applet::client::{self, IpcError};
use yutani::applet::display::{Display, display};
use yutani::applet::rate::{Rates, Sampler};
use yutani::applet::{Action, PENDING_S, note_visible, poll_interval};
use yutani::tunnel::status::Status;

use crate::view;

pub struct Applet {
    pub core: Core,
    /// The popup's surface id while it is open.
    pub popup: Option<Id>,
    /// The daemon's last `status` reply; `None` is the offline state.
    pub status: Option<Status>,
    pub rates: Rates,
    pub sampler: Sampler,
    /// Monotonic base for the rate sampler and the note timer.
    pub started: Instant,
    /// A `tunnel connect|disconnect` is in flight (icon shows sync).
    pub pending_until: Option<Instant>,
    pub accounts_open: bool,
    /// `(message, set_at_ms)` — an `err …` reply, shown for 3 s.
    pub note: Option<(String, u64)>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// The poll timer fired.
    Tick,
    /// A `status` request came back.
    Status(Result<Status, IpcError>),
    /// A menu row was pressed.
    Press(Action),
    /// A pressed action finished.
    Done(Action, Result<(), String>),
    ToggleAccounts,
    /// Popup create/destroy, handled by libcosmic.
    Surface(cosmic::surface::Action<Msg>),
    PopupClosed(Id),
}

impl Applet {
    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// True while a connect/disconnect is still settling (spec §3).
    pub fn pending(&self) -> bool {
        self.pending_until.is_some_and(|until| Instant::now() < until)
    }

    pub fn display(&self) -> Display {
        display(self.status.as_ref(), self.rates)
    }

    fn poll(&self) -> Task<Msg> {
        cosmic::task::future(async { Msg::Status(client::status().await) })
    }

    fn note(&mut self, message: String) {
        let at = self.now_ms();
        self.note = Some((message, at));
    }

    /// Open `~/.config/yutani/config.ron`, creating it with defaults first
    /// (spec §4.4). Until plan 5's settings window exists this is the
    /// Preferences… item.
    fn open_preferences() -> Result<(), String> {
        let path = yutani::model::config::config_path();
        if !path.exists()
            && let Err(err) = yutani::model::config::Config::default().save_to(&path)
        {
            return Err(format!("{err:#}"));
        }
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map(|_| ())
            .map_err(|_| format!("open {} manually", path.display()))
    }
}

impl cosmic::Application for Applet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Msg;
    /// Must match the basename of the `.desktop` file `yutani applet
    /// install` writes, which is how cosmic-panel identifies the applet.
    const APP_ID: &'static str = "com.yutani.Applet";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Msg>) {
        let applet = Applet {
            core,
            popup: None,
            status: None,
            rates: Rates::default(),
            sampler: Sampler::default(),
            started: Instant::now(),
            pending_until: None,
            accounts_open: false,
            note: None,
        };
        let first = cosmic::task::future(async { Msg::Status(client::status().await) });
        (applet, first)
    }

    fn on_close_requested(&self, id: Id) -> Option<Msg> {
        Some(Msg::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Msg> {
        cosmic::iced::time::every(poll_interval(self.popup.is_some())).map(|_| Msg::Tick)
    }

    fn update(&mut self, message: Msg) -> Task<Msg> {
        match message {
            Msg::Tick => {
                if let Some((_, at)) = self.note
                    && !note_visible(at, self.now_ms())
                {
                    self.note = None;
                }
                self.poll()
            }
            Msg::Status(Ok(status)) => {
                let live = status.tunnel.connected;
                if live {
                    let now = self.now_ms();
                    self.rates = self.sampler.push(status.tunnel.rx_bytes, status.tunnel.tx_bytes, now);
                } else {
                    // Counters freeze and rates read zero while down; the
                    // next connect must not show one huge catch-up spike.
                    self.sampler.reset();
                    self.rates = Rates::default();
                }
                if self.pending_until.is_some_and(|until| Instant::now() >= until) {
                    self.pending_until = None;
                }
                self.status = Some(status);
                Task::none()
            }
            Msg::Status(Err(IpcError::Offline)) => {
                self.status = None;
                self.sampler.reset();
                self.rates = Rates::default();
                self.pending_until = None;
                Task::none()
            }
            Msg::Status(Err(IpcError::Failed(msg))) => {
                // A failed poll after a success keeps the last totals
                // (spec §7); only the note changes.
                self.note(msg);
                Task::none()
            }
            Msg::Press(Action::Preferences) => {
                if let Err(msg) = Self::open_preferences() {
                    self.note(msg);
                }
                Task::none()
            }
            Msg::Press(Action::StartDaemon) => {
                match std::process::Command::new(yutani::applet::daemon_exe()).spawn() {
                    Ok(_) => self.poll(),
                    Err(err) => {
                        self.note(format!("cannot start yutani: {err}"));
                        Task::none()
                    }
                }
            }
            Msg::Press(action) => {
                let Some(request) = action.request() else { return Task::none() };
                if matches!(action, Action::Connect | Action::Disconnect) {
                    self.pending_until = Some(Instant::now() + Duration::from_secs(PENDING_S));
                }
                cosmic::task::future(async move {
                    let result = client::send(request).await.map(|_| ()).map_err(|err| err.to_string());
                    Msg::Done(action, result)
                })
            }
            Msg::Done(_, Ok(())) => self.poll(),
            Msg::Done(action, Err(msg)) => {
                if matches!(action, Action::Connect | Action::Disconnect) {
                    self.pending_until = None;
                }
                self.note(msg);
                self.poll()
            }
            Msg::ToggleAccounts => {
                self.accounts_open = !self.accounts_open;
                Task::none()
            }
            Msg::Surface(action) => cosmic::task::message(cosmic::Action::Surface(action)),
            Msg::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.accounts_open = false;
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> cosmic::Element<'_, Msg> {
        view::panel_button(self)
    }

    /// The popup's contents come from `app_popup`'s view closure, so no
    /// other surface is ever rendered.
    fn view_window(&self, _id: Id) -> cosmic::Element<'_, Msg> {
        cosmic::widget::text("").into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

/// Re-exported so `view` can build the popup-opening message.
pub fn open_popup_message(bounds: Rectangle, offset: cosmic::iced::Vector) -> Msg {
    Msg::Surface(app_popup::<Applet>(
        |_| Default::default(),
        move |state: &mut Applet| {
            let new_id = Id::unique();
            state.popup = Some(new_id);
            let mut settings = state.core.applet.get_popup_settings(
                state.core.main_window_id().unwrap(),
                new_id,
                None,
                None,
                None,
            );
            settings.positioner.anchor_rect = Rectangle {
                x: (bounds.x - offset.x) as i32,
                y: (bounds.y - offset.y) as i32,
                width: bounds.width as i32,
                height: bounds.height as i32,
            };
            settings
        },
        Some(Box::new(move |state: &Applet| {
            cosmic::Element::from(state.core.applet.popup_container(view::popup(state)))
                .map(cosmic::Action::App)
        })),
    ))
}

/// Close the open popup.
pub fn close_popup_message(id: Id) -> Msg {
    Msg::Surface(destroy_popup(id))
}
```

`Action::StartDaemon` needs to know which binary to spawn. Add this helper to `src/applet/mod.rs`, below `poll_interval`, and its test to that file's `mod tests`:

```rust
/// The daemon binary: the `yutani` next to this applet if it is there
/// (a cargo target dir, a prefix bin dir), else whatever `yutani` `PATH`
/// finds.
pub fn daemon_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("yutani")))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("yutani"))
}
```

```rust
    #[test]
    fn the_daemon_is_a_sibling_binary_or_just_a_name() {
        let exe = daemon_exe();
        assert_eq!(exe.file_name().unwrap(), "yutani");
        // Either an absolute sibling that exists, or the bare name for PATH.
        assert!(exe.is_absolute() && exe.is_file() || exe == std::path::PathBuf::from("yutani"));
    }
```

Create `src/bin/yutani-applet/view.rs` (panel button + the popup's read-only area; Task 4 appends the menu):

```rust
//! Rendering. Every colour and size comes from `yutani::applet::theme`;
//! every string comes from `yutani::applet::display::Display`.

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, Column, Row};
use cosmic::{Element, theme as cosmic_theme};

use yutani::applet::display::Display;
use yutani::applet::icon::icon_state;
use yutani::applet::theme;
use yutani::assets;

use crate::app::{Applet, Msg, close_popup_message, open_popup_message};

/// UI-font text in one of the handoff's colours.
fn ui<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, color: cosmic::iced::Color) -> Element<'a, Msg> {
    widget::text(content).size(size).class(cosmic_theme::Text::Color(color)).into()
}

/// Monospace, tabular text — every number, id and rate (handoff: critical,
/// live counters must not jitter).
fn mono<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a, size: f32, color: cosmic::iced::Color) -> Element<'a, Msg> {
    widget::text::monotext(content).size(size).class(cosmic_theme::Text::Color(color)).into()
}

fn glyph<'a>(bytes: &'static [u8], w: u16, h: u16) -> Element<'a, Msg> {
    widget::icon(widget::icon::from_svg_bytes(bytes))
        .width(Length::Fixed(f32::from(w)))
        .height(Length::Fixed(f32::from(h)))
        .into()
}

/// The Y mark in the panel: the state's icon, tinted by the panel theme
/// (`symbolic(true)` + `applet::style()`'s `icon_color`), dimmed to 38 %
/// when there is no daemon or no tunnel.
pub fn panel_button(state: &Applet) -> Element<'_, Msg> {
    let icon = icon_state(state.status.as_ref().map(|s| &s.tunnel), state.pending());
    let (w, h) = state.core.applet.suggested_size(true);
    let mark = widget::icon(widget::icon::from_svg_bytes(icon.bytes()).symbolic(true))
        .width(Length::Fixed(f32::from(w)))
        .height(Length::Fixed(f32::from(h)))
        .opacity(icon.opacity());
    let open = state.popup;
    state
        .core
        .applet
        .button_from_element(mark, true)
        .on_press_with_rectangle(move |offset, bounds| match open {
            Some(id) => close_popup_message(id),
            None => open_popup_message(bounds, offset),
        })
        .into()
}

/// Header: Y mark, "WireGuard", the status dot + text + location, and the
/// interface chip.
///
/// Every string is cloned rather than borrowed: `Display` is built inside
/// `popup`, so a borrowed `Element` could not outlive it. Do not "optimise"
/// these clones away — they are what makes the returned element `'a`-free.
fn header<'a>(d: &Display) -> Element<'a, Msg> {
    let (dot_color, text_color) = if d.connected {
        (theme::ACCENT_UP, theme::ACCENT_UP)
    } else {
        (theme::TEXT_MUTED, theme::TEXT_MUTED)
    };
    let dot = widget::container(
        widget::space().width(Length::Fixed(theme::DOT_PX)).height(Length::Fixed(theme::DOT_PX)),
    )
    .class(theme::dot_class(dot_color, d.connected));

    let status_row = Row::new()
        .spacing(theme::STATUS_GAP)
        .align_y(Alignment::Center)
        .push(dot)
        .push(mono(d.status_text, theme::STATUS_SIZE, text_color))
        .push(mono("·", theme::STATUS_SIZE, theme::SEPARATOR))
        .push(glyph(assets::PIN, 10, 12))
        .push(mono(d.location.clone(), theme::STATUS_SIZE, theme::TEXT_SECONDARY));

    let titles = Column::new()
        .spacing(theme::HEADER_COLUMN_GAP)
        .push(ui("WireGuard", theme::TITLE_SIZE, theme::TEXT_PRIMARY))
        .push(status_row);

    let chip = widget::container(mono(d.iface.clone(), theme::CHIP_SIZE, theme::TEXT_FAINT))
        .padding([4, 8])
        .class(theme::chip_class());

    Row::new()
        .spacing(theme::HEADER_GAP)
        .align_y(Alignment::Center)
        .padding([10, 10, 2, 10])
        .push(
            widget::icon(widget::icon::from_svg_bytes(assets::Y_SYMBOLIC).symbolic(true))
                .width(Length::Fixed(f32::from(theme::MARK_PX)))
                .height(Length::Fixed(f32::from(theme::MARK_PX))),
        )
        .push(titles)
        .push(widget::space().width(Length::Fill))
        .push(chip)
        .into()
}

/// Accounts band: the count and its label on the left, the tunnel IP and
/// the handshake age on the right.
fn accounts_band<'a>(d: &Display) -> Element<'a, Msg> {
    let left = Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(mono(d.accounts.to_string(), theme::COUNT_SIZE, theme::TEXT_PRIMARY))
        .push(ui(d.accounts_label, theme::COUNT_LABEL_SIZE, theme::TEXT_SECONDARY));
    let right = Column::new()
        .spacing(theme::HEADER_COLUMN_GAP)
        .align_x(Alignment::End)
        .push(mono(d.address.clone(), theme::BAND_RIGHT_SIZE, theme::TEXT_FAINT))
        .push(mono(d.handshake.clone(), theme::BAND_RIGHT_SIZE, theme::TEXT_FAINT));
    Row::new()
        .padding([4, 12, 6, 12])
        .align_y(Alignment::Center)
        .push(left)
        .push(widget::space().width(Length::Fill))
        .push(right)
        .into()
}

/// One traffic tile. No hover state — these are display only.
fn tile<'a>(
    label: &'static str,
    arrow: &'static [u8],
    accent: cosmic::iced::Color,
    total: String,
    rate: String,
) -> Element<'a, Msg> {
    let label_row = Row::new()
        .spacing(6)
        .align_y(Alignment::Center)
        .push(glyph(arrow, theme::GLYPH_PX, theme::GLYPH_PX))
        .push(ui(label, theme::TILE_LABEL_SIZE, theme::TEXT_FAINT));
    widget::container(
        Column::new()
            .spacing(theme::TILE_COLUMN_GAP)
            .push(label_row)
            .push(mono(total, theme::TILE_TOTAL_SIZE, theme::TEXT_PRIMARY))
            .push(mono(rate, theme::TILE_RATE_SIZE, accent)),
    )
    .padding([11, 13])
    .width(Length::FillPortion(1))
    .class(theme::tile_class())
    .into()
}

fn tiles<'a>(d: &Display) -> Element<'a, Msg> {
    Row::new()
        .spacing(theme::TILE_GAP)
        .padding([2, 10])
        .push(tile("UPLOAD", assets::ARROW_UP, theme::ACCENT_UP, d.up_total.clone(), d.up_rate.clone()))
        .push(tile(
            "DOWNLOAD",
            assets::ARROW_DOWN,
            theme::ACCENT_DOWN,
            d.down_total.clone(),
            d.down_rate.clone(),
        ))
        .into()
}

/// The popup's contents. libcosmic's `popup_container` supplies the
/// surface, blur, radius and shadow around this.
pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let d = state.display();
    Column::new()
        .spacing(theme::POPUP_PADDING)
        .padding(theme::POPUP_PADDING)
        .push(header(&d))
        .push(widget::container(theme::hairline::<Msg>()).padding([6, 10, 2, 10]))
        .push(accounts_band(&d))
        .push(widget::container(theme::hairline::<Msg>()).padding([2, 10, 4, 10]))
        .push(tiles(&d))
        .into()
}
```

- [ ] **Step 6: Build and test**

```bash
cargo build --bin yutani-applet 2>&1 | tail -20     # no warnings, no errors
cargo build -q 2>&1 | tail -5                        # the daemon still builds clean
cargo test -q 2>&1 | grep 'test result'
```
Expected: the applet binary links; `cargo test` prints **+6** on Task 2's total (5 in `applet::display`, 1 in `applet::tests::the_daemon_is_a_sibling_binary_or_just_a_name`) → `138 passed`.

Smoke it without a panel (the applet will fail to find a Wayland panel and exit; what matters is that it starts and does not panic on the missing daemon):
```bash
./target/debug/yutani-applet --help 2>&1 | head -3 || true
```
(`cosmic::applet::run` takes no arguments, so this only proves the binary executes; the real check is Task 6.)

- [ ] **Step 7: Commit**

```bash
git add src/applet/display.rs src/applet/mod.rs src/bin
git commit -m "feat(applet): panel button, popup, and the header/accounts/traffic area

The applet polls the daemon's status every 1 s with the popup open and
every 5 s closed, turns the reply into a tested Display, and renders the
handoff's header, accounts band and traffic tiles inside libcosmic's
standard popup container.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 4: The popup menu — tunnel, Accounts…, Preferences…, thumbnails, Quit, and the offline state

**Files:**
- Create: `src/applet/menu.rs`
- Modify: `src/applet/mod.rs` (add `pub mod menu;`), `src/bin/yutani-applet/view.rs`
- Test: the `mod tests` block in `src/applet/menu.rs`

**Interfaces:**
- Consumes: `crate::applet::Action`, `crate::tunnel::status::Status`, `crate::applet::{note_visible}`, `crate::applet::theme::*`, and from Task 3 `crate::bin` `view::{ui, mono}` helpers and `Applet`/`Msg`.
- Produces:
  ```rust
  // src/applet/menu.rs
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum RowKind { Normal, Danger, Account { active: bool } }
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct MenuRow {
      pub label: String,
      pub hint: Option<String>,
      pub trailing: Option<String>,
      pub action: Option<crate::applet::Action>,
      pub kind: RowKind,
      pub toggles_accounts: bool,
  }
  pub fn rows(status: Option<&crate::tunnel::status::Status>, accounts_open: bool) -> Vec<MenuRow>;
  // src/bin/yutani-applet/view.rs
  fn menu_row(row: &MenuRow) -> Element<'_, Msg>;
  // `popup` gains the menu, the note and Quit.
  ```

- [ ] **Step 1: Write the failing test for the menu**

Create `src/applet/menu.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::applet::Action;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    fn status(installed: bool, connected: bool, hidden: bool, names: &[&str]) -> Status {
        Status {
            clients: names
                .iter()
                .enumerate()
                .map(|(i, n)| ClientStatus { name: (*n).to_string(), active: i == 1 })
                .collect(),
            hidden,
            tunnel: TunnelStatus {
                installed,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: None,
                endpoint: None,
                handshake_age_s: None,
                rx_bytes: 0,
                tx_bytes: 0,
            },
        }
    }

    fn labels(rows: &[MenuRow]) -> Vec<&str> {
        rows.iter().map(|r| r.label.as_str()).collect()
    }

    #[test]
    fn without_a_daemon_the_only_thing_to_do_is_start_it() {
        let rows = rows(None, false);
        assert_eq!(labels(&rows), vec!["Start Yutani"]);
        assert_eq!(rows[0].action, Some(Action::StartDaemon));
        assert_eq!(rows[0].kind, RowKind::Normal);
        assert!(!rows[0].toggles_accounts);
        // The expansion state cannot resurrect account rows while offline.
        assert_eq!(rows(None, true).len(), 1);
    }

    #[test]
    fn a_connected_tunnel_offers_disconnect_with_the_truthful_hint() {
        let s = status(true, true, false, &["A", "B"]);
        let rows = rows(Some(&s), false);
        assert_eq!(
            labels(&rows),
            vec!["Disconnect tunnel", "Accounts…", "Preferences…", "Hide thumbnails", "Quit"]
        );
        assert_eq!(rows[0].action, Some(Action::Disconnect));
        assert_eq!(rows[0].hint.as_deref(), Some("systemd"));
        assert_eq!(rows[1].trailing.as_deref(), Some("2"));
        assert!(rows[1].toggles_accounts && rows[1].action.is_none());
        assert_eq!(rows[2].action, Some(Action::Preferences));
        assert_eq!(rows[3].action, Some(Action::HideThumbs));
        assert_eq!(rows[4].action, Some(Action::Quit));
        assert_eq!(rows[4].kind, RowKind::Danger);
    }

    #[test]
    fn a_disconnected_tunnel_offers_connect_and_an_uninstalled_one_is_disabled() {
        let s = status(true, false, false, &[]);
        let rows = rows(Some(&s), false);
        assert_eq!(rows[0].label, "Connect tunnel");
        assert_eq!(rows[0].action, Some(Action::Connect));
        assert_eq!(rows[0].hint.as_deref(), Some("systemd"));

        let s = status(false, false, false, &[]);
        let rows = rows(Some(&s), false);
        assert_eq!(rows[0].label, "Connect tunnel");
        assert_eq!(rows[0].action, None, "an uninstalled tunnel cannot be connected");
        assert_eq!(rows[0].hint.as_deref(), Some("not installed"));
    }

    #[test]
    fn expanding_accounts_adds_one_focus_row_per_client_in_layout_order() {
        let s = status(true, true, false, &["KestrelVance", "TrilliumTWO", "TrilliumTHREE"]);
        assert_eq!(rows(Some(&s), false).len(), 5);
        let rows = rows(Some(&s), true);
        assert_eq!(
            labels(&rows),
            vec![
                "Disconnect tunnel",
                "Accounts…",
                "KestrelVance",
                "TrilliumTWO",
                "TrilliumTHREE",
                "Preferences…",
                "Hide thumbnails",
                "Quit",
            ]
        );
        // `focus <n>` is 1-based over the daemon's layout order.
        assert_eq!(rows[2].action, Some(Action::Focus(1)));
        assert_eq!(rows[3].action, Some(Action::Focus(2)));
        assert_eq!(rows[4].action, Some(Action::Focus(3)));
        assert_eq!(rows[2].kind, RowKind::Account { active: false });
        assert_eq!(rows[3].kind, RowKind::Account { active: true });
        assert!(rows.iter().all(|r| r.label != "Accounts…" || r.toggles_accounts));
    }

    #[test]
    fn the_thumbnails_row_follows_the_daemons_hidden_flag() {
        let s = status(true, true, true, &["A"]);
        let rows = rows(Some(&s), false);
        assert_eq!(rows[3].label, "Show thumbnails");
        assert_eq!(rows[3].action, Some(Action::ShowThumbs));
        assert_eq!(rows[1].trailing.as_deref(), Some("1"));
    }

    #[test]
    fn quit_is_always_last_and_the_only_danger_row() {
        for accounts_open in [false, true] {
            let s = status(true, true, false, &["A", "B"]);
            let rows = rows(Some(&s), accounts_open);
            assert_eq!(rows.last().unwrap().label, "Quit");
            assert_eq!(rows.iter().filter(|r| r.kind == RowKind::Danger).count(), 1);
        }
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -q applet::menu 2>&1 | tail -10`
Expected: FAIL — `cannot find function rows in this scope`, `cannot find struct MenuRow in this scope`, `cannot find type RowKind in this scope`.

- [ ] **Step 3: Write the menu model**

Prepend to `src/applet/menu.rs`:

```rust
//! The popup's menu, as data. Spec §4.4–§4.5: the tunnel row, Accounts…
//! (expanding in place to one row per client), Preferences…, the
//! thumbnails row that replaces the old tray's Show/Hide, and Quit — or,
//! with no daemon, a single "Start Yutani".

use crate::applet::Action;
use crate::tunnel::status::Status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Normal,
    /// Quit: `#FF8A7E` text on a `#FF6B5C1F` hover.
    Danger,
    /// A client row under an expanded Accounts…; the dot shows focus.
    Account { active: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: String,
    /// Trailing mono hint, e.g. `systemd` or `not installed`.
    pub hint: Option<String>,
    /// Trailing mono value, e.g. the account count.
    pub trailing: Option<String>,
    /// `None` renders the row disabled.
    pub action: Option<Action>,
    pub kind: RowKind,
    /// The Accounts… header expands/collapses instead of acting.
    pub toggles_accounts: bool,
}

impl MenuRow {
    fn new(label: impl Into<String>, action: Option<Action>) -> Self {
        MenuRow {
            label: label.into(),
            hint: None,
            trailing: None,
            action,
            kind: RowKind::Normal,
            toggles_accounts: false,
        }
    }
}

/// `status` is `None` in the offline state.
pub fn rows(status: Option<&Status>, accounts_open: bool) -> Vec<MenuRow> {
    let Some(s) = status else {
        return vec![MenuRow::new("Start Yutani", Some(Action::StartDaemon))];
    };
    let mut out = Vec::new();

    // The handoff's hint is "wg-quick"; we drive systemd, so we say so.
    let mut tunnel = if s.tunnel.connected {
        MenuRow::new("Disconnect tunnel", Some(Action::Disconnect))
    } else {
        MenuRow::new("Connect tunnel", Some(Action::Connect))
    };
    if s.tunnel.installed {
        tunnel.hint = Some("systemd".to_string());
    } else {
        tunnel.action = None;
        tunnel.hint = Some("not installed".to_string());
    }
    out.push(tunnel);

    let mut accounts = MenuRow::new("Accounts…", None);
    accounts.trailing = Some(s.clients.len().to_string());
    accounts.toggles_accounts = true;
    out.push(accounts);
    if accounts_open {
        for (i, client) in s.clients.iter().enumerate() {
            let mut row = MenuRow::new(client.name.clone(), Some(Action::Focus(i + 1)));
            row.kind = RowKind::Account { active: client.active };
            out.push(row);
        }
    }

    out.push(MenuRow::new("Preferences…", Some(Action::Preferences)));
    out.push(if s.hidden {
        MenuRow::new("Show thumbnails", Some(Action::ShowThumbs))
    } else {
        MenuRow::new("Hide thumbnails", Some(Action::HideThumbs))
    });

    let mut quit = MenuRow::new("Quit", Some(Action::Quit));
    quit.kind = RowKind::Danger;
    out.push(quit);
    out
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -q applet::menu 2>&1 | tail -3`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 5: Render the menu, the note and Quit**

In `src/bin/yutani-applet/view.rs`, add to the imports:

```rust
use yutani::applet::menu::{MenuRow, RowKind};
use yutani::applet::note_visible;
```
(`yutani::applet::theme` is already imported from Task 3.) Add `pub mod menu;` to `src/applet/mod.rs`.

Append these two functions (above `popup`):

```rust
/// One menu row. Disabled rows carry no message, so libcosmic renders them
/// with the class's `disabled` style. The row is taken by value: its
/// strings become the element's, so nothing borrows `popup`'s local list.
fn menu_row<'a>(row: MenuRow) -> Element<'a, Msg> {
    let (text_color, hover) = match row.kind {
        RowKind::Danger => (theme::DANGER_TEXT, theme::DANGER_HOVER),
        _ => (theme::TEXT_ON_SURFACE, theme::HAIRLINE),
    };
    let mut content = Row::new().spacing(8).align_y(Alignment::Center);
    if let RowKind::Account { active } = row.kind {
        let color = if active { theme::ACCENT_UP } else { theme::TEXT_MUTED };
        content = content.push(
            widget::container(
                widget::space()
                    .width(Length::Fixed(theme::DOT_PX))
                    .height(Length::Fixed(theme::DOT_PX)),
            )
            .class(theme::dot_class(color, false)),
        );
    }
    content = content
        .push(ui(row.label, theme::MENU_SIZE, text_color))
        .push(widget::space().width(Length::Fill));
    if let Some(hint) = row.hint {
        content = content.push(mono(hint, theme::MENU_HINT_SIZE, theme::TEXT_FAINT));
    }
    if let Some(trailing) = row.trailing {
        content = content.push(mono(trailing, theme::MENU_HINT_SIZE, theme::TEXT_FAINT));
    }
    // Account rows are indented under their header.
    let padding: [u16; 4] =
        if matches!(row.kind, RowKind::Account { .. }) { [9, 10, 9, 22] } else { [9, 10, 9, 10] };
    let message = if row.toggles_accounts { Some(Msg::ToggleAccounts) } else { row.action.map(Msg::Press) };
    widget::button::custom(content)
        .width(Length::Fill)
        .padding(padding)
        .class(theme::menu_row_class(text_color, hover))
        .on_press_maybe(message)
        .into()
}

/// The last `err …` reply, for three seconds (spec §7).
fn note<'a>(state: &Applet) -> Option<Element<'a, Msg>> {
    let (message, at) = state.note.as_ref()?;
    note_visible(*at, state.now_ms()).then(|| {
        widget::container(mono(message.clone(), theme::MENU_HINT_SIZE, theme::DANGER_TEXT))
            .padding([0, 10, 4, 10])
            .into()
    })
}
```

Replace the body of `popup` with:

```rust
/// The popup's contents. libcosmic's `popup_container` supplies the
/// surface, blur, radius and shadow around this.
pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let d = state.display();
    let mut column = Column::new().spacing(theme::POPUP_PADDING).padding(theme::POPUP_PADDING);
    if d.online {
        column = column
            .push(header(&d))
            .push(widget::container(theme::hairline::<Msg>()).padding([6, 10, 2, 10]))
            .push(accounts_band(&d))
            .push(widget::container(theme::hairline::<Msg>()).padding([2, 10, 4, 10]))
            .push(tiles(&d))
            .push(widget::container(theme::hairline::<Msg>()).padding([2, 10, 2, 10]));
    } else {
        // Daemon offline: the header still identifies the applet, then
        // straight to the single "Start Yutani" row.
        column = column
            .push(header(&d))
            .push(widget::container(theme::hairline::<Msg>()).padding([6, 10, 2, 10]));
    }

    let rows = yutani::applet::menu::rows(state.status.as_ref(), state.accounts_open);
    let mut group = Column::new().spacing(1);
    for row in rows {
        // The handoff puts a hairline between the menu group and Quit.
        if row.kind == RowKind::Danger {
            group = group.push(widget::container(theme::hairline::<Msg>()).padding([2, 4, 2, 4]));
        }
        group = group.push(menu_row(row));
    }
    column = column.push(widget::container(group).padding([0, 4, 0, 4]));
    if let Some(note) = note(state) {
        column = column.push(note);
    }
    column.into()
}
```

- [ ] **Step 6: Build and test**

```bash
cargo build --bin yutani-applet 2>&1 | tail -20     # clean
cargo build -q 2>&1 | tail -5
cargo test -q 2>&1 | grep 'test result'
```
Expected: **+6** on Task 3's total → `144 passed`.

- [ ] **Step 7: Commit**

```bash
git add src/applet/menu.rs src/bin/yutani-applet/view.rs
git commit -m "feat(applet): popup menu, in-place account list, error notes and the offline state

Menu rows are data (tested): Connect/Disconnect with the truthful systemd
hint and a disabled 'not installed' state, Accounts… expanding to one
focus row per client in layout order, Preferences…, the thumbnails row
that replaces the old tray, and Quit — or a single Start Yutani when the
daemon is not running.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 5: `yutani applet install|uninstall`, and the removal of the ksni tray

**Files:**
- Create: `src/applet/install.rs`
- Modify: `src/applet/mod.rs` (add `pub mod install;`), `src/main.rs`, `src/ui/mod.rs`, `Cargo.toml`, `docs/superpowers/specs/2026-09-11-yutani-design.md`
- Delete: `src/ui/tray.rs`
- Test: the `mod tests` block in `src/applet/install.rs`

**Interfaces:**
- Consumes: `crate::assets::ICONS`.
- Produces:
  ```rust
  // src/applet/install.rs
  pub const DESKTOP_ID: &str = "com.yutani.Applet.desktop";
  pub fn icon_dir() -> std::path::PathBuf;      // ~/.local/share/icons/hicolor/symbolic/apps
  pub fn desktop_path() -> std::path::PathBuf;  // ~/.local/share/applications/com.yutani.Applet.desktop
  pub fn applet_exe() -> std::path::PathBuf;    // the yutani-applet next to this binary, else the bare name
  pub fn desktop_entry(exec: &str) -> String;
  pub fn install_to(icon_dir: &Path, desktop: &Path, exec: &str) -> anyhow::Result<usize>;
  pub fn uninstall_from(icon_dir: &Path, desktop: &Path) -> anyhow::Result<usize>;
  pub fn install() -> anyhow::Result<()>;
  pub fn uninstall() -> anyhow::Result<()>;
  ```
- Removed: `crate::ui::tray` (the whole module), `ui::Msg::Tray`, the `ksni` dependency.

- [ ] **Step 1: Write the failing test for install/uninstall**

Create `src/applet/install.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("yutani-applet-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        (root.join("icons"), root.join("applications").join(DESKTOP_ID))
    }

    #[test]
    fn the_desktop_entry_is_what_cosmic_panel_looks_for() {
        let text = desktop_entry("/usr/local/bin/yutani-applet");
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.ends_with('\n'));
        for line in [
            "Name=Yutani",
            "Type=Application",
            "Exec=/usr/local/bin/yutani-applet",
            "Icon=y-symbolic",
            "Terminal=false",
            "Categories=COSMIC;",
            "NoDisplay=true",
            "X-CosmicApplet=true",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
        }
    }

    #[test]
    fn install_writes_every_icon_and_the_desktop_file_and_is_idempotent() {
        let (icons, desktop) = dirs("install");
        assert_eq!(install_to(&icons, &desktop, "/opt/yutani-applet").unwrap(), 6);
        for (name, bytes) in crate::assets::ICONS {
            assert_eq!(std::fs::read(icons.join(name)).unwrap(), bytes);
        }
        assert!(std::fs::read_to_string(&desktop).unwrap().contains("Exec=/opt/yutani-applet"));
        // Running it again rewrites the same six files, not more.
        assert_eq!(install_to(&icons, &desktop, "/opt/yutani-applet").unwrap(), 6);
        assert_eq!(std::fs::read_dir(&icons).unwrap().count(), 5);
    }

    #[test]
    fn uninstall_removes_exactly_our_files() {
        let (icons, desktop) = dirs("uninstall");
        install_to(&icons, &desktop, "/opt/yutani-applet").unwrap();
        std::fs::write(icons.join("someone-elses-symbolic.svg"), b"<svg/>").unwrap();
        assert_eq!(uninstall_from(&icons, &desktop).unwrap(), 6);
        assert!(!desktop.exists());
        assert!(icons.join("someone-elses-symbolic.svg").exists(), "foreign icons stay");
        assert_eq!(std::fs::read_dir(&icons).unwrap().count(), 1);
    }

    #[test]
    fn uninstall_on_a_clean_system_removes_nothing_and_does_not_fail() {
        let (icons, desktop) = dirs("clean");
        assert_eq!(uninstall_from(&icons, &desktop).unwrap(), 0);
    }

    #[test]
    fn the_real_paths_are_the_xdg_user_ones() {
        assert!(icon_dir().ends_with("icons/hicolor/symbolic/apps"));
        assert!(desktop_path().ends_with("applications/com.yutani.Applet.desktop"));
        assert_eq!(applet_exe().file_name().unwrap(), "yutani-applet");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -q applet::install 2>&1 | tail -10`
Expected: FAIL — `cannot find function desktop_entry in this scope`, `cannot find function install_to in this scope`, `cannot find value DESKTOP_ID in this scope`.

- [ ] **Step 3: Write the installer**

Prepend to `src/applet/install.rs`:

```rust
//! `yutani applet install|uninstall`: the icon-theme copies and the
//! `.desktop` file cosmic-panel needs to list the applet (spec §3).
//! Everything is under `~/.local/share`, so no privileges are involved.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::assets::ICONS;

/// Must match `Applet::APP_ID` in the applet binary — cosmic-panel keys
/// applets on the desktop-entry id.
pub const DESKTOP_ID: &str = "com.yutani.Applet.desktop";

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Where symbolic app icons go for the current user.
pub fn icon_dir() -> PathBuf {
    data_dir().join("icons").join("hicolor").join("symbolic").join("apps")
}

pub fn desktop_path() -> PathBuf {
    data_dir().join("applications").join(DESKTOP_ID)
}

/// The applet binary: the one next to the running `yutani` when there is
/// one (cargo target dir, or a prefix bin dir), else the bare name so a
/// `PATH` lookup decides.
pub fn applet_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("yutani-applet")))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| PathBuf::from("yutani-applet"))
}

/// The desktop entry, shaped like COSMIC's own applets
/// (`/usr/share/applications/com.system76.CosmicApplet*.desktop`).
pub fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Name=Yutani\n\
         Comment=EVE Online clients and the WireGuard tunnel\n\
         Type=Application\n\
         Exec={exec}\n\
         Terminal=false\n\
         Categories=COSMIC;\n\
         Keywords=COSMIC;Applet;EVE;WireGuard;VPN;Yutani;\n\
         Icon=y-symbolic\n\
         StartupNotify=true\n\
         NoDisplay=true\n\
         X-CosmicApplet=true\n\
         X-OverflowPriority=50\n"
    )
}

/// Write the five icons and the desktop file; returns how many files were
/// written. Overwrites, so running it twice is a no-op with the same count.
pub fn install_to(icon_dir: &Path, desktop: &Path, exec: &str) -> anyhow::Result<usize> {
    std::fs::create_dir_all(icon_dir).with_context(|| format!("create {}", icon_dir.display()))?;
    for (name, bytes) in ICONS {
        let path = icon_dir.join(name);
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    }
    if let Some(parent) = desktop.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(desktop, desktop_entry(exec))
        .with_context(|| format!("write {}", desktop.display()))?;
    Ok(ICONS.len() + 1)
}

/// Remove exactly the files `install_to` wrote; returns how many existed.
pub fn uninstall_from(icon_dir: &Path, desktop: &Path) -> anyhow::Result<usize> {
    let mut removed = 0;
    for (name, _) in ICONS {
        let path = icon_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }
    match std::fs::remove_file(desktop) {
        Ok(()) => removed += 1,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("remove {}", desktop.display())),
    }
    Ok(removed)
}

/// Refresh the icon cache if the tool is there; never fatal.
fn update_icon_cache(icon_dir: &Path) {
    let theme_root = icon_dir.ancestors().nth(2); // …/icons/hicolor
    if let Some(root) = theme_root {
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .arg("-q")
            .arg("-t")
            .arg("-f")
            .arg(root)
            .status();
    }
}

pub fn install() -> anyhow::Result<()> {
    let icons = icon_dir();
    let desktop = desktop_path();
    let exe = applet_exe();
    let count = install_to(&icons, &desktop, &exe.to_string_lossy())?;
    update_icon_cache(&icons);
    println!("installed {count} files ({} and {})", icons.display(), desktop.display());
    println!("one manual step is left:");
    println!("  Settings → Desktop → Panel → Applets → add \"Yutani\"");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let icons = icon_dir();
    let desktop = desktop_path();
    let count = uninstall_from(&icons, &desktop)?;
    update_icon_cache(&icons);
    println!("removed {count} files");
    println!("remove the applet from the panel in Settings → Desktop → Panel → Applets");
    Ok(())
}
```

- [ ] **Step 4: Wire the subcommand and run the tests**

In `src/main.rs`, add to `enum Command` (after `Shortcuts`):

```rust
    /// Install or remove the COSMIC panel applet (icons and .desktop file)
    Applet {
        #[command(subcommand)]
        action: AppletAction,
    },
```

and after `enum ShortcutsAction`:

```rust
#[derive(Subcommand)]
enum AppletAction {
    /// Copy the icons and write the applet's .desktop file (idempotent)
    Install,
    /// Remove the icons and the .desktop file
    Uninstall,
}
```

and to the `match cli.command` arms (after the two `Shortcuts` arms):

```rust
        Some(Command::Applet { action: AppletAction::Install }) => {
            yutani::applet::install::install().map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Applet { action: AppletAction::Uninstall }) => {
            yutani::applet::install::uninstall().map(|()| ExitCode::SUCCESS)
        }
```

```bash
cargo test -q applet::install 2>&1 | tail -3        # test result: ok. 5 passed
cargo run -q -- applet install                      # writes 6 files, prints the manual step
ls ~/.local/share/icons/hicolor/symbolic/apps/y-*.svg ~/.local/share/applications/com.yutani.Applet.desktop
cargo run -q -- applet uninstall                    # removed 6 files
```

- [ ] **Step 5: Remove the ksni tray**

```bash
git rm src/ui/tray.rs
```

In `Cargo.toml`, delete the line:
```toml
ksni = { version = "0.3", default-features = false, features = ["blocking", "tokio"] }
```

In `src/ui/mod.rs`:
- delete `pub mod tray;` from the module list;
- delete the `Tray(tray::TrayEvent),` variant from `enum Msg`;
- delete the three `Msg::Tray(...)` arms from `update` (visibility toggling already goes through `set_hidden`, and `Request::Quit` already does the `ipc::remove_socket(); cosmic::iced::exit()` the tray's Quit did);
- delete `tray::subscription().map(Msg::Tray),` from the `subs` vector;
- change the doc comment on `set_hidden` from `/// The one place `hidden` changes (tray and IPC both come through here).` to `/// The one place `hidden` changes (every IPC request comes through here).`

Then:
```bash
cargo build -q 2>&1 | tail -5                       # silent; no unused-import warnings
cargo test -q 2>&1 | grep 'test result'
git diff Cargo.lock | grep '^[-+]name ='
```
Expected: build clean; tests **+5** on Task 4's total → `149 passed`; the lock diff shows exactly `-name = "ksni"` and `-name = "pastey"` and nothing else.

- [ ] **Step 6: Point the main spec at the applet spec**

In `docs/superpowers/specs/2026-09-11-yutani-design.md`, replace the `### Tray` section

```
### Tray

StatusNotifierItem via `ksni`: Show/Hide, Layouts submenu (apply), Settings,
Quit.
```

with

```
### Panel applet

Not a StatusNotifierItem: a COSMIC panel applet (`yutani-applet`, a second
binary of this crate) showing the Y mark and, on click, tunnel status,
connected accounts, traffic and the menu (Connect/Disconnect tunnel,
Accounts…, Preferences…, Show/Hide thumbnails, Quit). Specified in full in
`2026-09-12-yutani-applet-design.md`; the `ksni` path was removed, not kept
as a fallback (decided 2026-09-12).
```

and, four paragraphs above, change `libcosmic window, opened from the tray or `yutani settings`.` to `libcosmic window, opened from the applet or `yutani settings`.`

- [ ] **Step 7: Commit**

```bash
git add -A Cargo.toml Cargo.lock src/applet/install.rs src/main.rs src/ui docs/superpowers/specs/2026-09-11-yutani-design.md
git commit -m "feat(applet): yutani applet install|uninstall; drop the ksni tray

Installs the five handoff icons into the user's hicolor symbolic theme and
writes com.yutani.Applet.desktop (X-CosmicApplet=true, NoDisplay) so
cosmic-panel lists the applet, then prints the one manual step. The
StatusNotifierItem, its ksni dependency and Msg::Tray are gone; the main
spec now points at the applet spec.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

### Task 6: Hands-on acceptance (Daniel) and the spec status note

Everything up to here is verifiable without a panel. This task needs a COSMIC session, the tunnel from plan A installed, and EVE running.

- [ ] **Step 1: Install and add the applet**

```bash
cargo build --release 2>&1 | tail -3
./target/release/yutani applet install     # 6 files + the manual step
```
Then: *Settings → Desktop → Panel → Applets → add "Yutani"*. The Y mark appears in the panel. With no daemon running it is dimmed (38 %); clicking it opens the popup whose only menu row is **Start Yutani**. Press it — the daemon starts and the popup fills in within a second.

- [ ] **Step 2: Walk the states**

- **Icons:** with the tunnel disconnected the mark is plain; press *Connect tunnel* → it goes to the ring (sync) badge for up to 10 s, then plain once the handshake lands. Stop the peer (or `sudo ip link set yutani0 down`) and wait 3 minutes → the exclamation badge, and the header reads `Disconnected` while the address still shows.
- **Popup contents:** header shows `WireGuard`, a glowing `#2FD6B0` dot, `Connected · 📍 London`, and the `yutani0` chip; the accounts band shows the client count with the correct singular/plural, the tunnel IP and `hs Ns ago` counting up and resetting on each handshake; the tiles move while EVE is in game and the numbers do not jitter (tabular mono).
- **Disconnect:** dot and text go grey, the glow disappears, IP and handshake become `—`, both rates read `0 KB/s`, **the totals stay frozen**, and the row becomes *Connect tunnel*.
- **Accounts…:** shows the count; clicking expands one row per character in layout order with the active one dotted in `#2FD6B0`; clicking a row focuses that client; clicking the header again collapses it.
- **Thumbnails:** *Hide thumbnails* / *Show thumbnails* toggles the dock, and the label follows.
- **Preferences…:** opens `~/.config/yutani/config.ron` (creating it if absent).
- **Quit:** the daemon exits, the applet stays in the panel and switches to the dimmed icon and the single **Start Yutani** row.
- **Light theme:** switch COSMIC to light — the Y mark tints dark and stays legible at 16 px; the popup still reads correctly.
- **No tunnel installed** (on a machine where plan A's install has not run): *Connect tunnel* is disabled with the `not installed` hint.

- [ ] **Step 2b: Check the logs and the resource cost**

```bash
journalctl --user -n 50 | grep -i yutani        # nothing alarming
ps -o pid,rss,comm -C yutani-applet             # RSS should be modest and stable
```
With the popup closed the applet should wake once every 5 s; confirm the CPU cost is negligible over a minute of observation.

- [ ] **Step 3: Spec status**

Append to `docs/superpowers/specs/2026-09-12-yutani-applet-design.md`:

> **Status after plan B (date):** implemented. Acceptance results: which icon states were observed, how the popup looked against the handoff, anything deviating (popup width, the sync badge, letter-spacing — see the plan's Self-review), and anything left for plan 5.

```bash
git add docs/superpowers/specs/2026-09-12-yutani-applet-design.md
git commit -m "docs: applet acceptance

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9"
```

---

## Self-review

### Spec coverage

| Applet spec section | Where it lands |
|---|---|
| §1 Goal; non-goals (offline state, ksni removed not kept) | Tasks 3 (offline `Display`), 4 (offline menu), 5 (ksni removal) |
| §2 Architecture — second binary, `src/lib.rs` with `ipc` + status types + `model::config` | Task 1 |
| §2 — libcosmic applet API, popup as a shell-owned surface | Task 3 (`applet::run`, `get_popup_settings`, `app_popup`, `popup_container`) |
| §2 — polling 1 s open / 5 s closed; rates as deltas; totals frozen while down | Task 1 (`poll_interval`), 2 (`rate::Sampler`), 3 (`Display`, `Msg::Status` handling) |
| §2 — actions map 1:1 to IPC; poll immediately after | Task 1 (`Action::request`), 3 (`Msg::Done` → `poll`) |
| §3 Panel button, five icon states, `yutani applet install`, `.desktop` | Tasks 1 (assets), 2 (`icon_state`), 3 (button), 5 (install) |
| §4.1 Header — mark, "WireGuard", dot + glow, status text, `·`, pin, location, iface chip | Tasks 1 (tokens, pin glyph), 3 (`header`, `Display`) |
| §4.2 Accounts band — count, singular/plural, address or `—`, `hs Ns ago` / `hs —` | Tasks 2 (`format`), 3 (`Display`, `accounts_band`) |
| §4.3 Traffic tiles — totals, rates, accents, arrows, tabular numerals | Tasks 1 (arrow glyphs, tokens), 2 (`format::bytes`/`rate`), 3 (`tiles`, `monotext`) |
| §4.4 Menu — tunnel row + `systemd` hint + `not installed` disabled; Accounts… expansion → `focus n`; Preferences…; thumbnails row | Task 4 (`menu::rows`, `menu_row`), 3 (`Action` dispatch, `open_preferences`) |
| §4.5 Quit → offline state with a single `Start Yutani` | Task 4 (`rows(None, _)`), 3 (`Action::StartDaemon`, `daemon_exe`) |
| §5 IPC additions consumed; `serde_json`; offline detection on connect failure | Task 2 (`client`, `IpcError::Offline`) |
| §6 Removal of the ksni tray; main spec pointer | Task 5 |
| §7 Errors — socket errors never crash; `err …` note for 3 s; `xdg-open` missing note | Tasks 1 (`note_visible`), 2 (`IpcError`), 3 (`Msg::Status(Err)`, `open_preferences`), 4 (`note`) |
| §8 Testing — formatting table, `Status` JSON round-trip, icon-state selection, rate with counter reset | Task 2 (all four; the JSON round-trip is exercised end-to-end in `client`'s tests, on top of plan A's own `status_json_round_trips`) |
| §8 Hands-on | Task 6 |
| §9 Out of scope (graphs, per-client traffic, multiple locations, notifications) | Not implemented anywhere — confirmed absent |

### Placeholder scan

No "TBD", no "add error handling", no "write tests for the above", no "similar to Task N". Every code block is complete and self-contained; every type, function and constant named in a task is defined either in that task, in an earlier task of this plan, or in plan A (`ipc::{MAX_LINE, Request, Response, socket_path}`, `ipc::Request::{Status, TunnelConnect, TunnelDisconnect, Focus, Show, Hide, Quit}`, `ipc::Response::{Ok, OkData, Err}`, `tunnel::IFACE`, `tunnel::status::{Status, ClientStatus, TunnelStatus}`) or the standard library / a pinned dependency whose signature is quoted.

### Type consistency

- `Status` / `ClientStatus` / `TunnelStatus` are plan A's, used with plan A's exact field names (`installed, connected, iface, location, address, endpoint, handshake_age_s, rx_bytes, tx_bytes`; `clients, hidden, tunnel`; `name, active`) in Tasks 2, 3, 4 and every test fixture. Nothing redefines them.
- `ipc::Response::OkData(String)` is consumed in exactly one place (`client::send_to`), matching plan A Task 4's `to_line`/`parse`.
- `applet::Action` is produced in `menu.rs` (Task 4), consumed in `app.rs` (Task 3) and mapped in `mod.rs` (Task 1) — one definition, `Copy`, so `Option<Action>` stays `Copy` for `row.action.map(Msg::Press)`.
- `rate::Rates { rx, tx }` is `rx = download, tx = upload` everywhere; `Display` maps `tx → up_*` and `rx → down_*` once, in `display()`, and the view never re-decides.
- `Display` field names are identical in `display.rs`, its tests and `view.rs`.
- `IconState` is only produced by `icon_state` and only consumed by `panel_button`.
- `IpcError` is `Clone + Debug + PartialEq + Eq`, as `Msg::Status(Result<Status, IpcError>)` requires (`Msg: Clone + Debug + Send`).
- Module declarations: Task 1 adds `theme`; Task 2 adds `client, format, icon, rate`; Task 3 adds `display`; Task 4 adds `menu`; Task 5 adds `install`. No task declares a module whose file does not yet exist.
- Test totals: 107 (after plan A) → 115 → 132 → 138 → 144 → 149. Deltas +8, +17, +6, +6, +5.

### Ambiguities resolved (and how)

1. **Popup width 336 px → 360 px.** `Context::popup_container` hard-codes `min_width(360.0).max_width(360.0)` at the pinned rev, and both the spec (§2) and the handoff say to use the standard popup container because it supplies the surface, blur, radius and shadow. 336 was the web prototype's outer width. Every *internal* padding, gap, radius, size and colour is the handoff's, unchanged.
2. **Spinner rotation (1.1 s linear) is not animated.** libcosmic's `Icon` does expose `.rotation(..)`, but animating it needs a per-frame redraw subscription in a panel process for a state that lasts ≤ 10 s. The sync icon is shown statically. (`IconState::Sync` and the ring asset are in place, so adding a rotation later is a one-line change.)
3. **`y-sync-symbolic.svg`'s badge is re-drawn as one even-odd path** with identical geometry, because a symbolic tint is a single colour filter over the whole SVG and would otherwise fill the ring's hole. The bundle's other four icons keep their exact drawing.
4. **The C2PA `<metadata>` blob is stripped** from all five icons (≈7.8 KB each) so the repo diff is readable. Drawing bytes are unchanged; a test asserts no `c2pa` remains.
5. **Letter-spacing `.12em` on the uppercase tile labels is dropped** — iced's `Text` has no letter-spacing setting at this rev. Everything else about those labels (10 px, uppercase, `#7D838C`, the 10×10 arrow) is as specified.
6. **The panel button's 26×26 / radius 7 / `#FFFFFF1A` hover / `#FFFFFF1F` active come from libcosmic**, via `button_from_element` + `Button::AppletIcon` + `Context::suggested_size`, rather than being hardcoded — that is what makes the button track the panel's configured size and look right on a light theme. The handoff's numbers are COSMIC's own values for a small panel.
7. **The popup's surface colours (`#1A1D21F5`), its `0 22px 52px #00000099` shadow and its 14 px radius are not implemented by us** — `popup_container` paints them, exactly as the handoff instructs. Only the tokens used *inside* the popup are in `theme.rs`.
8. **Desktop id `com.yutani.Applet`** as the spec names it, and `Applet::APP_ID` is set to match (cosmic-panel keys applets on the desktop-entry id). The daemon keeps its own `io.github.yutani`; they are separate processes and separate surfaces.
9. **"Connected" requires a handshake younger than 180 s** (spec §4.1, taken literally): a tunnel that has just come up reads `Disconnected` in the header for the second or two before its first handshake, while the panel icon shows the sync badge. This is the spec's rule, not an accident; if it feels wrong in Task 6, the one-line change is in `display()`.
10. **"unit failed" has no representation in `TunnelStatus`** (plan A did not add one), so `IconState::Attention` is driven only by handshake staleness, and a connected tunnel with no handshake yet is `Sync` rather than `Attention`. A future `failed: bool` on `TunnelStatus` would slot into `icon_state` alone.
11. **The tunnel hint reads `systemd`, not the handoff's `wg-quick`** — the spec explicitly calls for the truthful hint.
12. **The offline popup keeps the header** (with `—` for location and address, `0 KB` totals) above the single `Start Yutani` row. The spec only fixes the menu for that state; showing the chrome keeps the popup from collapsing to a single button.
13. **Fonts:** COSMIC's UI font via `cosmic::widget::text` and its monospace via `cosmic::widget::text::monotext` (`cosmic::font::mono()`), as the handoff permits ("substitute the codebase's own UI font… keep a monospace for all numbers"). Space Grotesk / JetBrains Mono are not shipped.
14. **`bytes()` and `rate()` truncate below their next unit** (`999_999 B` → `999 KB`, not `1000 KB`). The handoff gives the ladder but not the rounding.
15. **Seven new `Cargo.lock` entries** are unavoidable (see Global Constraints); they are named, justified, and the `[patch]` workaround is documented as rejected by cargo.

Nothing is left unresolved.
