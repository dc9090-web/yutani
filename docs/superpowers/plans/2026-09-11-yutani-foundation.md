# Yutani Foundation & Live-Thumbnail Proof — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A running `yutani` binary that shows one live, zero-copy thumbnail per EVE Online client on COSMIC, with an active-client border, character name, and click-to-focus — plus `yutani doctor`.

**Architecture:** One libcosmic app with no main window. A backend thread runs a second event queue on iced's Wayland connection (calloop), tracks toplevels via `cosmic::cctk`, captures each EVE client into a 2-buffer gbm dmabuf swapchain via `ext_image_copy_capture`, and streams `SubsurfaceBuffer`s to the UI. The UI creates one overlay layer surface per client and renders the frame with libcosmic's `Subsurface` widget. This is plan 1 of 3 (spec §3–§6 core path; dragging, dock, settings, tray, hotkeys, IPC and layouts come in plans 2 and 3).

**Tech Stack:** Rust 2024 (rustup stable ≥ 1.93), libcosmic (git, pinned rev), `cosmic::cctk` (cosmic-client-toolkit re-export), calloop 0.14 + calloop-wayland-source 0.4, gbm 0.18, futures-timer, clap 4, serde + ron, tracing.

**Spec:** `docs/superpowers/specs/2026-09-11-yutani-design.md`

**Reference code (read-only, in `reference/`, gitignored):**
- `reference/cosmic-workspaces/src/backend/wayland/` — the backend we mirror (GPL-3.0-only; Yutani is GPL-3.0-only too).
- `reference/libcosmic/iced/winit/src/platform_specific/wayland/subsurface_widget.rs` — `Subsurface`, `SubsurfaceBuffer`, `Dmabuf`, `Plane`, `Shmbuf`, `BufferSource`.
- `reference/cosmic-protocols/client-toolkit/src/` — `toplevel_info`, `toplevel_management`, `screencopy`.

## Global Constraints

- Platform: cosmic-comp ≥ 1.7 on Wayland; protocols listed in spec §2 are required at startup (`ext_foreign_toplevel_list_v1`, `zcosmic_toplevel_info_v1`, `zcosmic_toplevel_manager_v1`, `ext_image_copy_capture_manager_v1`, `ext_foreign_toplevel_image_capture_source_manager_v1`, `zwlr_layer_shell_v1`, `zwp_linux_dmabuf_v1`).
- libcosmic pinned to git rev `a401af8b1c54a8abd393b8c5b7c8809402f83850` (the revision in `reference/libcosmic`). All Wayland types come from `cosmic::cctk::{wayland_client, wayland_protocols, sctk}` — never add a direct `wayland-client`/`wayland-protocols`/`smithay-client-toolkit` dependency.
- Default detection key: `app_ids: ["steam_app_8500"]`; title `EVE Launcher` is never a client; `EVE - <Name>` → logged in as `<Name>`; any other title with a matching app_id → logging in (spec §4).
- Config file: `~/.config/yutani/config.ron`; unparseable config → warn and use defaults, never overwrite (spec §9, §10).
- Layer surfaces: layer Overlay, anchor top-left, exclusive zone 0, keyboard interactivity None, namespace `yutani` (spec §6).
- Yutani never sends input to EVE; it only calls `activate` / `set_minimized` (spec §1).
- Logging via `tracing`, `RUST_LOG` honoured (spec §10).
- License: GPL-3.0-only (backend is modelled on cosmic-workspaces).
- Commit after every task with a `feat:`/`test:`/`chore:` prefix.

## File Structure

| Path | Responsibility |
| --- | --- |
| `Cargo.toml` | crate metadata and pinned dependencies |
| `LICENSE` | GPL-3.0-only text |
| `src/main.rs` | clap CLI; dispatches to `ui::run()` or `doctor::run()` |
| `src/doctor.rs` | lists Wayland globals, checks required protocols (pure `evaluate` + I/O `run`) |
| `src/model/mod.rs` | re-exports |
| `src/model/client.rs` | `Login`, `classify()` — pure EVE window classification |
| `src/model/config.rs` | `Config` struct, defaults, RON load/save, `parse_color` |
| `src/backend/mod.rs` | `Event`, `Cmd`, `ClientInfo`, `CaptureImage`, `subscription()`, `AppData`, thread + calloop loop, `handle_cmd` |
| `src/backend/toplevels.rs` | `ToplevelInfoHandler` / `ToplevelManagerHandler` impls → `Event::Client*`, start/stop capture |
| `src/backend/gbm_devices.rs` | cache of opened gbm devices by dev id |
| `src/backend/dmabuf.rs` | `DmabufHandler` impl (stores feedback) |
| `src/backend/buffer.rs` | `Buffer` (gbm dmabuf or shm fallback) + `wl_buffer` dispatch |
| `src/backend/capture.rs` | `Capture`, `ScreencopySession`, `ScreencopyHandler` impl, FPS throttle |
| `src/ui/mod.rs` | `App: cosmic::Application`, `Msg`, subscriptions, layer-surface lifecycle |
| `src/ui/thumbnail.rs` | thumbnail widget: bordered container + `Subsurface` + name label |

---

### Task 1: Scaffold the crate and `yutani doctor`

**Files:**
- Create: `Cargo.toml`, `LICENSE`, `.gitignore` (append), `src/main.rs`, `src/doctor.rs`

**Interfaces:**
- Produces: `doctor::run() -> anyhow::Result<std::process::ExitCode>`; `doctor::evaluate(globals: &[(String, u32)]) -> Vec<Check>`; `pub struct Check { pub interface: &'static str, pub required: bool, pub found: Option<u32> }`.

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "yutani"
version = "0.1.0"
edition = "2024"
rust-version = "1.93"
license = "GPL-3.0-only"
description = "Live thumbnails and client switching for EVE Online on COSMIC"

[dependencies]
libcosmic = { git = "https://github.com/pop-os/libcosmic", rev = "a401af8b1c54a8abd393b8c5b7c8809402f83850", default-features = false, features = [
    "tokio",
    "wayland",
    "multi-window",
    "winit",
    "wgpu",
    "single-instance",
] }
calloop = { version = "0.14.4", features = ["executor"] }
calloop-wayland-source = "0.4.1"
gbm = "0.18.0"
futures-timer = "3"
anyhow = "1"
clap = { version = "4", features = ["derive"] }
dirs = "6"
ron = "0.10"
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
```

- [ ] **Step 2: Add the licence and gitignore entries**

Run:
```bash
cd ~/Yutani
curl -sL https://www.gnu.org/licenses/gpl-3.0.txt -o LICENSE
head -3 LICENSE
printf 'target/\n' >> .gitignore
```
Expected: first line of `LICENSE` is `                    GNU GENERAL PUBLIC LICENSE`.

- [ ] **Step 3: Write `src/main.rs`**

```rust
mod doctor;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "yutani", version, about = "Live thumbnails for EVE Online on COSMIC")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Check that the compositor supports everything Yutani needs
    Doctor,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Doctor) => doctor::run(),
        None => {
            eprintln!("yutani: the app is not built yet; try `yutani doctor`");
            Ok(ExitCode::from(1))
        }
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("yutani: {err:#}");
            ExitCode::from(1)
        }
    }
}
```

- [ ] **Step 4: Write the failing test for `evaluate` in `src/doctor.rs`**

```rust
//! `yutani doctor`: report which Wayland protocols the compositor offers.

use cosmic::cctk::wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub interface: &'static str,
    pub required: bool,
    pub found: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_interface_present_is_found_with_version() {
        let globals = vec![("zwlr_layer_shell_v1".to_string(), 5)];
        let checks = evaluate(&globals);
        let layer = checks
            .iter()
            .find(|c| c.interface == "zwlr_layer_shell_v1")
            .unwrap();
        assert!(layer.required);
        assert_eq!(layer.found, Some(5));
    }

    #[test]
    fn missing_interface_has_no_version() {
        let checks = evaluate(&[]);
        assert!(checks.iter().all(|c| c.found.is_none()));
        assert!(checks.iter().any(|c| c.interface == "zcosmic_overlap_notify_v1" && !c.required));
    }

    #[test]
    fn all_required_present_means_ok() {
        let globals: Vec<(String, u32)> = REQUIRED.iter().map(|i| (i.to_string(), 1)).collect();
        assert!(all_required_present(&evaluate(&globals)));
        assert!(!all_required_present(&evaluate(&globals[1..])));
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cd ~/Yutani && cargo test doctor 2>&1 | tail -5`
Expected: compile error — `evaluate`, `REQUIRED`, `all_required_present` not found. (The first build fetches and compiles libcosmic; allow 5–10 minutes.)

- [ ] **Step 6: Implement `evaluate`, `all_required_present`, and `run`**

Insert above the `#[cfg(test)]` block in `src/doctor.rs`:

```rust
pub const REQUIRED: &[&str] = &[
    "ext_foreign_toplevel_list_v1",
    "zcosmic_toplevel_info_v1",
    "zcosmic_toplevel_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "zwlr_layer_shell_v1",
    "zwp_linux_dmabuf_v1",
];

pub const OPTIONAL: &[&str] = &["zcosmic_overlap_notify_v1"];

/// Pure: match the advertised globals against what Yutani needs.
pub fn evaluate(globals: &[(String, u32)]) -> Vec<Check> {
    let lookup = |name: &str| {
        globals
            .iter()
            .find(|(iface, _)| iface == name)
            .map(|(_, version)| *version)
    };
    REQUIRED
        .iter()
        .map(|iface| Check { interface: iface, required: true, found: lookup(iface) })
        .chain(
            OPTIONAL
                .iter()
                .map(|iface| Check { interface: iface, required: false, found: lookup(iface) }),
        )
        .collect()
}

pub fn all_required_present(checks: &[Check]) -> bool {
    checks.iter().filter(|c| c.required).all(|c| c.found.is_some())
}

struct Doctor;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Doctor {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

pub fn run() -> anyhow::Result<ExitCode> {
    let conn = Connection::connect_to_env()?;
    let (globals, _queue) = registry_queue_init::<Doctor>(&conn)?;
    let advertised: Vec<(String, u32)> = globals
        .contents()
        .clone_list()
        .into_iter()
        .map(|g| (g.interface, g.version))
        .collect();

    let checks = evaluate(&advertised);
    println!("{:<56} {:<9} {}", "interface", "needed", "found");
    for c in &checks {
        let needed = if c.required { "required" } else { "optional" };
        let found = match c.found {
            Some(v) => format!("v{v}"),
            None => "MISSING".to_string(),
        };
        println!("{:<56} {:<9} {}", c.interface, needed, found);
    }

    if all_required_present(&checks) {
        println!("\nOK: cosmic-comp advertises everything Yutani needs.");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("\nMISSING required protocols — Yutani cannot run on this compositor.");
        Ok(ExitCode::from(2))
    }
}
```

- [ ] **Step 7: Run the tests and the command**

Run: `cd ~/Yutani && cargo test doctor 2>&1 | tail -5`
Expected: `test result: ok. 3 passed`

Run: `cargo run -q -- doctor`
Expected: a table with every required row showing a version and the final line `OK: cosmic-comp advertises everything Yutani needs.`; exit code 0 (`echo $?`).

- [ ] **Step 8: Commit**

```bash
cd ~/Yutani && git add Cargo.toml Cargo.lock LICENSE .gitignore src/main.rs src/doctor.rs
git commit -m "feat: scaffold crate and add yutani doctor"
```

---

### Task 2: EVE window classification (`model::client`)

**Files:**
- Create: `src/model/mod.rs`, `src/model/client.rs`
- Modify: `src/main.rs` (add `mod model;`)

**Interfaces:**
- Produces: `model::client::Login { LoggingIn, LoggedIn(String) }`; `model::client::classify(app_ids: &[String], app_id: &str, title: &str) -> Option<Login>`; `Login::label(&self) -> &str`.

- [ ] **Step 1: Write the failing tests in `src/model/client.rs`**

```rust
//! Pure classification of compositor toplevels into EVE clients.

/// Login state derived from the window title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    /// Login screen, character select, or a transient title.
    LoggingIn,
    /// In game as this character.
    LoggedIn(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        vec!["steam_app_8500".to_string()]
    }

    #[test]
    fn logged_in_title_yields_character_name() {
        assert_eq!(
            classify(&ids(), "steam_app_8500", "EVE - Aria Vex"),
            Some(Login::LoggedIn("Aria Vex".into()))
        );
    }

    #[test]
    fn bare_eve_title_is_logging_in() {
        assert_eq!(classify(&ids(), "steam_app_8500", "EVE"), Some(Login::LoggingIn));
    }

    #[test]
    fn launcher_is_not_a_client() {
        assert_eq!(classify(&ids(), "steam_app_8500", "EVE Launcher"), None);
    }

    #[test]
    fn other_app_id_is_not_a_client() {
        assert_eq!(classify(&ids(), "firefox", "EVE - Aria Vex"), None);
    }

    #[test]
    fn unknown_title_with_matching_app_id_is_logging_in() {
        assert_eq!(classify(&ids(), "steam_app_8500", "Wine crash"), Some(Login::LoggingIn));
    }

    #[test]
    fn empty_name_after_dash_is_logging_in() {
        assert_eq!(classify(&ids(), "steam_app_8500", "EVE -  "), Some(Login::LoggingIn));
    }

    #[test]
    fn label_is_name_or_placeholder() {
        assert_eq!(Login::LoggedIn("Kel".into()).label(), "Kel");
        assert_eq!(Login::LoggingIn.label(), "Logging in…");
    }
}
```

And `src/model/mod.rs`:

```rust
pub mod client;
```

Add `mod model;` under `mod doctor;` in `src/main.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/Yutani && cargo test model::client 2>&1 | tail -5`
Expected: compile error — `classify` and `label` not found.

- [ ] **Step 3: Implement `classify` and `label`**

Insert between the `Login` enum and the tests in `src/model/client.rs`:

```rust
const LAUNCHER_TITLE: &str = "EVE Launcher";
const CLIENT_PREFIX: &str = "EVE - ";

impl Login {
    pub fn label(&self) -> &str {
        match self {
            Login::LoggedIn(name) => name,
            Login::LoggingIn => "Logging in…",
        }
    }
}

/// Decide whether a toplevel is an EVE client and, if so, its login state.
///
/// `None` means "not a client" (wrong app_id, or the launcher).
pub fn classify(app_ids: &[String], app_id: &str, title: &str) -> Option<Login> {
    if !app_ids.iter().any(|id| id == app_id) {
        return None;
    }
    if title == LAUNCHER_TITLE {
        return None;
    }
    if let Some(name) = title.strip_prefix(CLIENT_PREFIX) {
        let name = name.trim();
        if !name.is_empty() {
            return Some(Login::LoggedIn(name.to_string()));
        }
    }
    Some(Login::LoggingIn)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ~/Yutani && cargo test model::client 2>&1 | tail -5`
Expected: `test result: ok. 7 passed`

- [ ] **Step 5: Commit**

```bash
cd ~/Yutani && git add src/main.rs src/model
git commit -m "feat: classify toplevels into EVE clients by app_id and title"
```

---

### Task 3: Config file (`model::config`)

**Files:**
- Create: `src/model/config.rs`
- Modify: `src/model/mod.rs`

**Interfaces:**
- Produces: `model::config::Config { app_ids: Vec<String>, thumb_width: u32, opacity: f32, fps: u32, active_border: String, inactive_border: String, border_px: u32, show_names: bool }` with `Default`, `Config::load() -> Config`, `Config::load_from(path: &Path) -> Config`, `Config::save(&self) -> anyhow::Result<()>`, `Config::save_to(&self, path: &Path) -> anyhow::Result<()>`, `config_path() -> PathBuf`, `parse_color(hex: &str) -> Option<[f32; 4]>`.

- [ ] **Step 1: Write the failing tests in `src/model/config.rs`**

```rust
//! User configuration: `~/.config/yutani/config.ron`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Wayland app_ids that are EVE clients.
    pub app_ids: Vec<String>,
    /// Thumbnail width in logical pixels; height follows the window's aspect.
    pub thumb_width: u32,
    /// 0.0–1.0
    pub opacity: f32,
    /// Max capture rate: 10, 15, 30 or 60.
    pub fps: u32,
    /// "#rrggbb" or "#rrggbbaa"
    pub active_border: String,
    pub inactive_border: String,
    pub border_px: u32,
    pub show_names: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.app_ids, vec!["steam_app_8500".to_string()]);
        assert_eq!(c.thumb_width, 320);
        assert_eq!(c.opacity, 0.9);
        assert_eq!(c.fps, 30);
        assert_eq!(c.active_border, "#ff8800");
        assert_eq!(c.inactive_border, "#404040");
        assert_eq!(c.border_px, 2);
        assert!(c.show_names);
    }

    #[test]
    fn round_trips_through_ron() {
        let dir = std::env::temp_dir().join(format!("yutani-test-{}", std::process::id()));
        let path = dir.join("config.ron");
        let mut c = Config::default();
        c.thumb_width = 200;
        c.app_ids.push("firefox".into());
        c.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(Config::load_from(Path::new("/nonexistent/yutani.ron")), Config::default());
    }

    #[test]
    fn bad_file_gives_defaults_and_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("yutani-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        std::fs::write(&path, "(this is not ron").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(this is not ron");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let dir = std::env::temp_dir().join(format!("yutani-partial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        std::fs::write(&path, "(fps: 60)").unwrap();
        let c = Config::load_from(&path);
        assert_eq!(c.fps, 60);
        assert_eq!(c.thumb_width, 320);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#ff8800"), Some([1.0, 136.0 / 255.0, 0.0, 1.0]));
        assert_eq!(parse_color("#00000080"), Some([0.0, 0.0, 0.0, 128.0 / 255.0]));
        assert_eq!(parse_color("ff8800"), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#gg0000"), None);
    }
}
```

Add `pub mod config;` to `src/model/mod.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/Yutani && cargo test model::config 2>&1 | tail -5`
Expected: compile error — `Default` not implemented for `Config`, `save_to`/`load_from`/`parse_color` not found.

- [ ] **Step 3: Implement defaults, load/save, and `parse_color`**

Insert between the struct and the tests:

```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            app_ids: vec!["steam_app_8500".to_string()],
            thumb_width: 320,
            opacity: 0.9,
            fps: 30,
            active_border: "#ff8800".to_string(),
            inactive_border: "#404040".to_string(),
            border_px: 2,
            show_names: true,
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yutani")
        .join("config.ron")
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    /// Missing file → defaults. Unreadable/unparseable file → warn, defaults,
    /// and the file is never touched.
    pub fn load_from(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!("cannot read {}: {err}; using defaults", path.display());
                return Self::default();
            }
        };
        match ron::from_str(&text) {
            Ok(config) => config,
            Err(err) => {
                tracing::warn!("cannot parse {}: {err}; using defaults", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, text)?;
        Ok(())
    }
}

/// "#rrggbb" or "#rrggbbaa" → [r, g, b, a] in 0.0–1.0.
pub fn parse_color(hex: &str) -> Option<[f32; 4]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 && digits.len() != 8 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let r = byte(0)?;
    let g = byte(2)?;
    let b = byte(4)?;
    let a = if digits.len() == 8 { byte(6)? } else { 255 };
    Some([r, g, b, a].map(|v| v as f32 / 255.0))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ~/Yutani && cargo test model::config 2>&1 | tail -5`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: Commit**

```bash
cd ~/Yutani && git add src/model
git commit -m "feat: add RON config with defaults and colour parsing"
```

---

### Task 4: Backend thread with toplevel tracking, and the app shell

This task makes `yutani` (no args) start a libcosmic app with no windows that logs every EVE client appearing, changing, and disappearing, and can activate one. Capture is added in Task 5, thumbnails in Task 6.

**Files:**
- Create: `src/backend/mod.rs`, `src/backend/toplevels.rs`, `src/ui/mod.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces (backend): `backend::Handle` (= `ExtForeignToplevelHandleV1`); `backend::ClientInfo { login: Login, activated: bool, minimized: bool, outputs: Vec<WlOutput> }`; `backend::Event::{CmdSender(calloop::channel::Sender<Cmd>), ClientAdded(Handle, ClientInfo), ClientUpdated(Handle, ClientInfo), ClientRemoved(Handle)}`; `backend::Cmd::{Activate(Handle), Minimize(Handle), SetAppIds(Vec<String>), SetFps(u32)}`; `backend::subscription(conn: Connection, app_ids: Vec<String>, fps: u32) -> Subscription<Event>`; `AppData` fields `qh, registry_state, seat_state, toplevel_info_state, toplevel_manager_state, sender, app_ids, fps` and methods `send_event`, `handle_cmd`, `client_info(&ToplevelInfo) -> Option<ClientInfo>`.
- Produces (ui): `ui::run(config: Config) -> iced::Result`; `App { core, config, conn: Option<Connection>, cmd: Option<Sender<Cmd>>, clients: HashMap<Handle, Client>, outputs: Vec<Output> }`; `Msg::{Wayland(WaylandEvent), Backend(Event)}`.

- [ ] **Step 1: Write `src/backend/mod.rs`**

```rust
//! Compositor thread: a second event queue on iced's Wayland connection,
//! driven by calloop. Tracks toplevels, activates them, and (Task 5)
//! captures them. Modelled on cosmic-workspaces' backend.

use cosmic::cctk::{
    self,
    sctk::{
        registry::{ProvidesRegistryState, RegistryState},
        seat::{SeatHandler, SeatState},
    },
    toplevel_info::{ToplevelInfo, ToplevelInfoState},
    toplevel_management::ToplevelManagerState,
    wayland_client::{
        Connection, QueueHandle,
        globals::registry_queue_init,
        protocol::{wl_output::WlOutput, wl_seat},
    },
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
};
use cosmic::cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State;
use cosmic::iced::{
    self,
    futures::{FutureExt, SinkExt, channel::mpsc, executor::block_on},
};
use calloop_wayland_source::WaylandSource;
use std::{hash::Hash, thread};

use crate::model::client::{Login, classify};

mod toplevels;

pub type Handle = ExtForeignToplevelHandleV1;

#[derive(Clone, Debug, PartialEq)]
pub struct ClientInfo {
    pub login: Login,
    pub activated: bool,
    pub minimized: bool,
    pub outputs: Vec<WlOutput>,
}

#[derive(Clone, Debug)]
pub enum Event {
    /// Sent once at startup so the UI can send commands.
    CmdSender(calloop::channel::Sender<Cmd>),
    ClientAdded(Handle, ClientInfo),
    ClientUpdated(Handle, ClientInfo),
    ClientRemoved(Handle),
}

#[derive(Debug)]
pub enum Cmd {
    Activate(Handle),
    Minimize(Handle),
    SetAppIds(Vec<String>),
    SetFps(u32),
}

/// iced subscription that owns the backend thread for the app's lifetime.
pub fn subscription(conn: Connection, app_ids: Vec<String>, fps: u32) -> iced::Subscription<Event> {
    #[derive(Clone)]
    struct Key(Connection);
    impl Hash for Key {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.0.backend().display_id().hash(state);
        }
    }
    iced::Subscription::run_with(Key(conn), move |Key(conn)| {
        let conn = conn.clone();
        let app_ids = app_ids.clone();
        async move { start(conn, app_ids, fps) }.flatten_stream()
    })
}

pub struct AppData {
    pub qh: QueueHandle<Self>,
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub toplevel_info_state: ToplevelInfoState,
    pub toplevel_manager_state: Option<ToplevelManagerState>,
    pub sender: mpsc::Sender<Event>,
    pub app_ids: Vec<String>,
    pub fps: u32,
}

impl AppData {
    pub fn send_event(&mut self, event: Event) {
        let _ = block_on(self.sender.send(event));
    }

    /// `None` when the toplevel is not an EVE client.
    pub fn client_info(&self, info: &ToplevelInfo) -> Option<ClientInfo> {
        let login = classify(&self.app_ids, &info.app_id, &info.title)?;
        Some(ClientInfo {
            login,
            activated: info.state.contains(&State::Activated),
            minimized: info.state.contains(&State::Minimized),
            outputs: info.output.iter().cloned().collect(),
        })
    }

    fn cosmic_handle(
        &self,
        handle: &Handle,
    ) -> Option<cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>
    {
        self.toplevel_info_state
            .info(handle)
            .and_then(|info| info.cosmic_toplevel.clone())
    }

    pub fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Activate(handle) => {
                let Some(cosmic) = self.cosmic_handle(&handle) else { return };
                let Some(manager) = &self.toplevel_manager_state else { return };
                for seat in self.seat_state.seats() {
                    manager.manager.activate(&cosmic, &seat);
                }
            }
            Cmd::Minimize(handle) => {
                let Some(cosmic) = self.cosmic_handle(&handle) else { return };
                let Some(manager) = &self.toplevel_manager_state else { return };
                manager.manager.set_minimized(&cosmic);
            }
            Cmd::SetAppIds(app_ids) => {
                self.app_ids = app_ids;
                self.reclassify_all();
            }
            Cmd::SetFps(fps) => {
                self.fps = fps;
            }
        }
    }
}

fn start(conn: Connection, app_ids: Vec<String>, fps: u32) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(64);

    let (globals, event_queue) = registry_queue_init(&conn).expect("wayland registry");
    let qh = event_queue.handle();

    thread::Builder::new()
        .name("yutani-backend".into())
        .spawn(move || {
            let registry_state = RegistryState::new(&globals);
            let mut app_data = AppData {
                qh: qh.clone(),
                seat_state: SeatState::new(&globals, &qh),
                toplevel_info_state: ToplevelInfoState::new(&registry_state, &qh),
                toplevel_manager_state: ToplevelManagerState::try_new(&registry_state, &qh),
                registry_state,
                sender,
                app_ids,
                fps,
            };

            let (cmd_sender, cmd_channel) = calloop::channel::channel();
            app_data.send_event(Event::CmdSender(cmd_sender));

            let mut event_loop = calloop::EventLoop::try_new().expect("calloop");
            WaylandSource::new(conn, event_queue)
                .insert(event_loop.handle())
                .expect("wayland source");
            event_loop
                .handle()
                .insert_source(cmd_channel, |event, _, app_data: &mut AppData| {
                    if let calloop::channel::Event::Msg(cmd) = event {
                        app_data.handle_cmd(cmd);
                    }
                })
                .expect("cmd channel");

            loop {
                if let Err(err) = event_loop.dispatch(None, &mut app_data) {
                    tracing::error!("backend event loop failed: {err}");
                    std::process::exit(1);
                }
            }
        })
        .expect("spawn backend thread");

    receiver
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    // Deliberately no OutputState: all wl_output handles are iced's.
    cctk::sctk::registry_handlers!(SeatState);
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: cctk::sctk::seat::Capability,
    ) {
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: cctk::sctk::seat::Capability,
    ) {
    }
}

cctk::sctk::delegate_registry!(AppData);
cctk::sctk::delegate_seat!(AppData);
```

- [ ] **Step 2: Write `src/backend/toplevels.rs`**

```rust
//! Toplevel list → `Event::Client*`; toplevel manager capabilities.

use cosmic::cctk::{
    self,
    cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    wayland_client::{Connection, QueueHandle, WEnum},
};
use std::collections::HashSet;

use super::{AppData, Event, Handle};

impl AppData {
    /// Re-run classification for every known toplevel (after `SetAppIds`).
    pub fn reclassify_all(&mut self) {
        let infos: Vec<_> = self
            .toplevel_info_state
            .toplevels()
            .map(|info| (info.foreign_toplevel.clone(), info.clone()))
            .collect();
        for (handle, info) in infos {
            match self.client_info(&info) {
                Some(client) => self.send_event(Event::ClientAdded(handle, client)),
                None => self.send_event(Event::ClientRemoved(handle)),
            }
        }
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, handle: &Handle) {
        let Some(info) = self.toplevel_info_state.info(handle).cloned() else { return };
        if let Some(client) = self.client_info(&info) {
            tracing::info!(title = %info.title, "client added");
            self.send_event(Event::ClientAdded(handle.clone(), client));
        }
    }

    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, handle: &Handle) {
        let Some(info) = self.toplevel_info_state.info(handle).cloned() else { return };
        match self.client_info(&info) {
            Some(client) => {
                tracing::debug!(title = %info.title, activated = client.activated, "client updated");
                // The UI treats Updated for an unknown handle as Added.
                self.send_event(Event::ClientUpdated(handle.clone(), client));
            }
            None => self.send_event(Event::ClientRemoved(handle.clone())),
        }
    }

    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, handle: &Handle) {
        tracing::info!("toplevel closed");
        self.send_event(Event::ClientRemoved(handle.clone()));
    }
}

impl ToplevelManagerHandler for AppData {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        self.toplevel_manager_state.as_mut().expect("toplevel manager")
    }

    fn capabilities(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        capabilities: Vec<
            WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>,
        >,
    ) {
        let caps: HashSet<_> = capabilities
            .into_iter()
            .filter_map(|c| match c {
                WEnum::Value(v) => Some(v),
                WEnum::Unknown(_) => None,
            })
            .collect();
        tracing::info!(?caps, "toplevel manager capabilities");
    }
}

cctk::delegate_toplevel_info!(AppData);
cctk::delegate_toplevel_manager!(AppData);
```

- [ ] **Step 3: Write `src/ui/mod.rs`**

```rust
//! libcosmic application: no main window; one overlay layer surface per
//! EVE client (Task 6). This task only wires events and logs them.

use cosmic::cctk::wayland_client::{Connection, Proxy, protocol::wl_output::WlOutput};
use cosmic::iced::event::wayland::{Event as WaylandEvent, OutputEvent};
use cosmic::iced::{self, Subscription};
use cosmic::{Application, Element, Task};
use std::collections::HashMap;

use crate::backend::{self, ClientInfo, Cmd, Event, Handle};
use crate::model::config::Config;

pub fn run(config: Config) -> iced::Result {
    cosmic::app::run_single_instance::<App>(
        cosmic::app::Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        config,
    )
}

#[derive(Clone, Debug)]
pub struct Output {
    pub handle: WlOutput,
    pub name: String,
    pub logical_size: (i32, i32),
}

#[derive(Debug)]
pub struct Client {
    pub info: ClientInfo,
}

pub struct App {
    core: cosmic::app::Core,
    pub config: Config,
    pub conn: Option<Connection>,
    pub cmd: Option<calloop::channel::Sender<Cmd>>,
    pub clients: HashMap<Handle, Client>,
    pub outputs: Vec<Output>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Wayland(WaylandEvent),
    Backend(Event),
}

impl App {
    fn send(&self, cmd: Cmd) {
        match &self.cmd {
            Some(sender) => {
                if let Err(err) = sender.send(cmd) {
                    tracing::error!("backend command channel closed: {err}");
                }
            }
            None => tracing::warn!("backend not ready; dropping command"),
        }
    }

    fn on_output(&mut self, event: OutputEvent, output: WlOutput) {
        // Recover iced's connection from the first output we see.
        if self.conn.is_none()
            && let Some(backend) = output.backend().upgrade()
        {
            self.conn = Some(Connection::from_backend(backend));
        }
        match event {
            OutputEvent::Created(Some(info)) | OutputEvent::InfoUpdate(info) => {
                let name = info.name.clone().unwrap_or_else(|| format!("output-{}", info.id));
                let logical_size = info.logical_size.unwrap_or((0, 0));
                if let Some(existing) = self.outputs.iter_mut().find(|o| o.handle == output) {
                    existing.name = name;
                    existing.logical_size = logical_size;
                } else {
                    tracing::info!(%name, ?logical_size, "output");
                    self.outputs.push(Output { handle: output, name, logical_size });
                }
            }
            OutputEvent::Created(None) => {}
            OutputEvent::Removed => self.outputs.retain(|o| o.handle != output),
        }
    }

    fn on_backend(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::CmdSender(sender) => {
                self.cmd = Some(sender);
            }
            Event::ClientAdded(handle, info) | Event::ClientUpdated(handle, info) => {
                tracing::info!(label = info.login.label(), activated = info.activated, "client");
                self.clients.insert(handle, Client { info });
            }
            Event::ClientRemoved(handle) => {
                self.clients.remove(&handle);
            }
        }
        Task::none()
    }
}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = Config;
    type Message = Msg;
    const APP_ID: &'static str = "io.github.yutani";

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn init(core: cosmic::app::Core, config: Config) -> (Self, Task<Msg>) {
        let app = App {
            core,
            config,
            conn: None,
            cmd: None,
            clients: HashMap::new(),
            outputs: Vec::new(),
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Msg) -> Task<Msg> {
        match message {
            Msg::Wayland(WaylandEvent::Output(event, output)) => {
                self.on_output(event, output);
                Task::none()
            }
            Msg::Wayland(_) => Task::none(),
            Msg::Backend(event) => self.on_backend(event),
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let wayland = iced::event::listen_with(|event, _, _| match event {
            iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(event)) => {
                Some(Msg::Wayland(event))
            }
            _ => None,
        });
        let mut subs = vec![wayland];
        if let Some(conn) = self.conn.clone() {
            subs.push(
                backend::subscription(conn, self.config.app_ids.clone(), self.config.fps)
                    .map(Msg::Backend),
            );
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Msg> {
        unreachable!("no main window")
    }

    fn view_window(&self, _id: iced::window::Id) -> Element<'_, Msg> {
        cosmic::widget::text("yutani").into()
    }
}
```

- [ ] **Step 4: Wire `main.rs` to run the app**

Replace the whole of `src/main.rs` with:

```rust
mod backend;
mod doctor;
mod model;
mod ui;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "yutani", version, about = "Live thumbnails for EVE Online on COSMIC")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Check that the compositor supports everything Yutani needs
    Doctor,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Doctor) => doctor::run(),
        None => {
            let config = model::config::Config::load();
            ui::run(config)
                .map(|()| ExitCode::SUCCESS)
                .map_err(anyhow::Error::from)
        }
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("yutani: {err:#}");
            ExitCode::from(1)
        }
    }
}
```

- [ ] **Step 5: Build and fix compile errors**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^(error|warning: unused)' -A5 | head -60`
Expected: builds clean. Likely first-pass fixes: import paths for `WaylandEvent`/`OutputEvent` (`cosmic::iced::event::wayland::{Event, OutputEvent}`), `let` chains need edition 2024 (already set). If `cosmic::executor::Default` is not found, use `cosmic::SingleThreadExecutor`.

- [ ] **Step 6: Manual test — see clients appear**

Create a test config that treats Firefox as an EVE client (any app works; Firefox has several windows):

```bash
mkdir -p ~/.config/yutani
cat > ~/.config/yutani/config.ron <<'EOF'
(app_ids: ["firefox", "steam_app_8500"])
EOF
cd ~/Yutani && RUST_LOG=info timeout 8 cargo run -q 2>&1 | grep -E 'output|client|capabilities' | head -20
```
Expected: lines like `output name=DP-1`, `toplevel manager capabilities caps={Activate, …}`, and one `client label="Logging in…"` per Firefox window (and, if EVE is running, `client label="Aria Vex" …`). Open/close a Firefox window while it runs (use `timeout 30`) and confirm `toplevel closed` appears.

- [ ] **Step 7: Commit**

```bash
cd ~/Yutani && git add src
git commit -m "feat: backend thread tracks EVE clients; app shell with no main window"
```

---

### Task 5: Zero-copy capture pipeline

Adds per-client capture sessions with a 2-buffer gbm dmabuf swapchain (shm fallback), FPS throttling, and `Event::Frame`.

**Files:**
- Create: `src/backend/gbm_devices.rs`, `src/backend/dmabuf.rs`, `src/backend/buffer.rs`, `src/backend/capture.rs`
- Modify: `src/backend/mod.rs`, `src/backend/toplevels.rs`, `src/ui/mod.rs`

**Interfaces:**
- Produces: `backend::CaptureImage { buffer: SubsurfaceBuffer, width: u32, height: u32, transform: wl_output::Transform }`; `Event::Frame(Handle, CaptureImage)`; `AppData` gains `screencopy_state, dmabuf_state, dmabuf_feedback, gbm_devices, shm_state, captures: HashMap<Handle, Arc<Capture>>, thread_pool, fps: Arc<AtomicU32>`; `AppData::start_capture(&Handle)`, `AppData::stop_capture(&Handle)`, `AppData::create_buffer(&Formats) -> Buffer`.
- Consumes: `Handle`, `Event`, `AppData::send_event`, `AppData::client_info` from Task 4.

- [ ] **Step 1: Write `src/backend/gbm_devices.rs`**

```rust
//! Opened gbm devices, keyed by DRM dev id (as reported by the compositor).

use std::collections::hash_map::{self, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

#[derive(Default)]
pub struct GbmDevices {
    devices: HashMap<u64, (PathBuf, gbm::Device<fs::File>)>,
}

impl GbmDevices {
    pub fn gbm_device(&mut self, dev: u64) -> io::Result<Option<(&Path, &gbm::Device<fs::File>)>> {
        Ok(match self.devices.entry(dev) {
            hash_map::Entry::Occupied(entry) => {
                let (path, gbm) = entry.into_mut();
                Some((path, gbm))
            }
            hash_map::Entry::Vacant(entry) => match find_gbm_device(dev)? {
                Some(value) => {
                    let (path, gbm) = entry.insert(value);
                    Some((path, gbm))
                }
                None => None,
            },
        })
    }
}

fn find_gbm_device(dev: u64) -> io::Result<Option<(PathBuf, gbm::Device<fs::File>)>> {
    for entry in fs::read_dir("/dev/dri")? {
        let entry = entry?;
        if entry.metadata()?.rdev() == dev {
            let file = fs::File::options().read(true).write(true).open(entry.path())?;
            tracing::info!("opened gbm device {}", entry.path().display());
            return Ok(Some((entry.path(), gbm::Device::new(file)?)));
        }
    }
    Ok(None)
}
```

- [ ] **Step 2: Write `src/backend/dmabuf.rs`**

```rust
//! linux-dmabuf handler: we only need the default feedback (main device).

use cosmic::cctk::{
    self,
    sctk::dmabuf::{DmabufFeedback, DmabufHandler, DmabufState},
    wayland_client::{Connection, QueueHandle, protocol::wl_buffer},
    wayland_protocols::wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
    },
};

use super::AppData;

impl DmabufHandler for AppData {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_feedback(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpLinuxDmabufFeedbackV1,
        feedback: DmabufFeedback,
    ) {
        self.dmabuf_feedback = Some(feedback);
    }

    fn created(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpLinuxBufferParamsV1, _: wl_buffer::WlBuffer) {}

    fn failed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpLinuxBufferParamsV1) {}

    fn released(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_buffer::WlBuffer) {}
}

cctk::sctk::delegate_dmabuf!(AppData);
```

- [ ] **Step 3: Write `src/backend/buffer.rs`**

```rust
//! One capture buffer: a gbm dmabuf (zero-copy) or, if that fails, wl_shm.
//! `backing` is shared with the UI so the Subsurface widget reuses its
//! wl_buffer instead of re-importing the fds every frame.

use cosmic::cctk::{
    screencopy::{Formats, Rect},
    wayland_client::{
        Connection, Dispatch, QueueHandle,
        protocol::{wl_buffer, wl_shm, wl_shm_pool},
    },
    wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1,
};
use cosmic::iced::platform_specific::shell::subsurface_widget::{BufferSource, Dmabuf, Plane, Shmbuf};
use std::os::fd::AsFd;
use std::sync::Arc;

use super::AppData;

pub struct Buffer {
    pub backing: Arc<BufferSource>,
    pub buffer: wl_buffer::WlBuffer,
    pub damage: Vec<Rect>,
    pub size: (u32, u32),
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

fn full_damage((width, height): (u32, u32)) -> Vec<Rect> {
    vec![Rect { x: 0, y: 0, width: width as i32, height: height as i32 }]
}

impl AppData {
    fn create_gbm_buffer(
        &mut self,
        format: u32,
        modifiers: &[u64],
        (width, height): (u32, u32),
        drm_dev: Option<u64>,
    ) -> anyhow::Result<Option<Buffer>> {
        let Some(feedback) = self.dmabuf_feedback.as_ref() else {
            return Ok(None);
        };
        let drm_dev = drm_dev.unwrap_or(feedback.main_device());
        let Some((_path, gbm)) = self.gbm_devices.gbm_device(drm_dev)? else {
            return Ok(None);
        };

        let modifiers: Vec<gbm::Modifier> = modifiers.iter().map(|m| gbm::Modifier::from(*m)).collect();
        if modifiers.is_empty() {
            return Ok(None);
        }
        let gbm_format = gbm::Format::try_from(format)?;
        let bo = if modifiers.iter().all(|m| *m == gbm::Modifier::Invalid) {
            gbm.create_buffer_object::<()>(width, height, gbm_format, gbm::BufferObjectFlags::empty())?
        } else {
            gbm.create_buffer_object_with_modifiers::<()>(width, height, gbm_format, modifiers.iter().copied())?
        };

        let params = self.dmabuf_state.create_params(&self.qh)?;
        let modifier = bo.modifier();
        let mut planes = Vec::new();
        for i in 0..bo.plane_count() as i32 {
            let fd = bo.fd_for_plane(i)?;
            let offset = bo.offset(i);
            let stride = bo.stride_for_plane(i);
            params.add(fd.as_fd(), i as u32, offset, stride, modifier.into());
            planes.push(Plane { fd, plane_idx: i as u32, offset, stride });
        }
        let (buffer, _) = params.create_immed(
            width as i32,
            height as i32,
            format,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &self.qh,
        );

        Ok(Some(Buffer {
            backing: Arc::new(
                Dmabuf { width: width as i32, height: height as i32, planes, format, modifier: modifier.into() }.into(),
            ),
            buffer,
            damage: full_damage((width, height)),
            size: (width, height),
        }))
    }

    fn create_shm_buffer(&self, format: wl_shm::Format, (width, height): (u32, u32)) -> anyhow::Result<Buffer> {
        let stride = width as i32 * 4;
        let len = stride as usize * height as usize;
        let fd = memfd(len)?;
        let pool = self.shm_state.wl_shm().create_pool(fd.as_fd(), len as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, width as i32, height as i32, stride, format, &self.qh, ());
        pool.destroy();
        Ok(Buffer {
            backing: Arc::new(
                Shmbuf { fd, offset: 0, width: width as i32, height: height as i32, stride, format }.into(),
            ),
            buffer,
            damage: full_damage((width, height)),
            size: (width, height),
        })
    }

    /// gbm dmabuf if the compositor advertises one for ABGR8888, else shm.
    pub fn create_buffer(&mut self, formats: &Formats) -> anyhow::Result<Buffer> {
        let format = wl_shm::Format::Abgr8888;
        if let Some((_, modifiers)) = formats.dmabuf_formats.iter().find(|(f, _)| *f == u32::from(format)) {
            match self.create_gbm_buffer(u32::from(format), modifiers, formats.buffer_size, formats.dmabuf_device) {
                Ok(Some(buffer)) => return Ok(buffer),
                Ok(None) => tracing::warn!("no usable gbm device; falling back to shm"),
                Err(err) => tracing::warn!("gbm buffer failed: {err}; falling back to shm"),
            }
        }
        anyhow::ensure!(formats.shm_formats.contains(&format), "compositor offers neither dmabuf nor shm ABGR8888");
        self.create_shm_buffer(format, formats.buffer_size)
    }
}

fn memfd(len: usize) -> anyhow::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let name = c"yutani-shm";
    // SAFETY: memfd_create with a valid C string and flags; result checked below.
    let fd = unsafe { libc_memfd_create(name.as_ptr(), 1 /* MFD_CLOEXEC */) };
    anyhow::ensure!(fd >= 0, "memfd_create failed: {}", std::io::Error::last_os_error());
    // SAFETY: fd is a fresh, owned descriptor.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let file = std::fs::File::from(fd.try_clone()?);
    file.set_len(len as u64)?;
    Ok(fd)
}

unsafe extern "C" {
    #[link_name = "memfd_create"]
    fn libc_memfd_create(name: *const std::ffi::c_char, flags: std::ffi::c_uint) -> std::ffi::c_int;
}

impl Dispatch<wl_buffer::WlBuffer, ()> for AppData {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for AppData {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
```

- [ ] **Step 4: Write `src/backend/capture.rs`**

```rust
//! Per-client capture session: 2-buffer swapchain, damage-driven, throttled
//! to `fps`. The front buffer is shipped to the UI as a `SubsurfaceBuffer`;
//! the next capture into it waits for the compositor's release.

use cosmic::cctk::{
    self,
    screencopy::{
        CaptureFrame, CaptureOptions, CaptureSession, CaptureSource, FailureReason, Formats, Frame,
        ScreencopyFrameData, ScreencopyFrameDataExt, ScreencopyHandler, ScreencopySessionData,
        ScreencopySessionDataExt, ScreencopyState,
    },
    wayland_client::{Connection, QueueHandle, WEnum},
};
use cosmic::iced::platform_specific::shell::subsurface_widget::{SubsurfaceBuffer, SubsurfaceBufferRelease};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::buffer::Buffer;
use super::{AppData, CaptureImage, Event, Handle};

const BUFFER_COUNT: usize = 2;

pub struct Capture {
    pub handle: Handle,
    pub session: Mutex<Option<ScreencopySession>>,
}

impl Capture {
    pub fn new(handle: Handle) -> Arc<Self> {
        Arc::new(Capture { handle, session: Mutex::new(None) })
    }

    pub fn for_session(session: &CaptureSession) -> Option<Arc<Self>> {
        session.data::<SessionData>()?.capture.upgrade()
    }

    pub fn start(self: &Arc<Self>, screencopy: &ScreencopyState, qh: &QueueHandle<AppData>) {
        let mut session = self.session.lock().unwrap();
        if session.is_none() {
            *session = ScreencopySession::new(self, screencopy, qh);
        }
    }

    pub fn stop(&self) {
        self.session.lock().unwrap().take();
    }
}

pub struct ScreencopySession {
    formats: Option<Formats>,
    /// [front, back]; rotated on every ready frame.
    buffers: Option<[Buffer; BUFFER_COUNT]>,
    session: CaptureSession,
    release: Option<SubsurfaceBufferRelease>,
    last_submit: Instant,
}

impl ScreencopySession {
    fn new(capture: &Arc<Capture>, screencopy: &ScreencopyState, qh: &QueueHandle<AppData>) -> Option<Self> {
        let udata = SessionData { session_data: Default::default(), capture: Arc::downgrade(capture) };
        let source = CaptureSource::Toplevel(capture.handle.clone());
        match screencopy.capturer().create_session(&source, CaptureOptions::empty(), qh, udata) {
            Ok(session) => Some(Self { formats: None, buffers: None, session, release: None, last_submit: Instant::now() }),
            Err(err) => {
                tracing::error!("cannot create capture session: {err:?}");
                None
            }
        }
    }

    fn submit(&mut self, capture: &Arc<Capture>, conn: &Connection, qh: &QueueHandle<AppData>) {
        let Some(back) = self.buffers.as_ref().map(|b| &b[1]) else { return };
        self.session.capture(
            &back.buffer,
            &back.damage,
            qh,
            FrameData { frame_data: Default::default(), capture: Arc::downgrade(capture) },
        );
        self.last_submit = Instant::now();
        let _ = conn.flush();
    }
}

pub struct SessionData {
    session_data: ScreencopySessionData,
    capture: Weak<Capture>,
}

impl ScreencopySessionDataExt for SessionData {
    fn screencopy_session_data(&self) -> &ScreencopySessionData {
        &self.session_data
    }
}

struct FrameData {
    frame_data: ScreencopyFrameData,
    capture: Weak<Capture>,
}

impl ScreencopyFrameDataExt for FrameData {
    fn screencopy_frame_data(&self) -> &ScreencopyFrameData {
        &self.frame_data
    }
}

impl AppData {
    pub fn start_capture(&mut self, handle: &Handle) {
        let capture = self.captures.entry(handle.clone()).or_insert_with(|| Capture::new(handle.clone())).clone();
        capture.start(&self.screencopy_state, &self.qh);
    }

    pub fn stop_capture(&mut self, handle: &Handle) {
        if let Some(capture) = self.captures.remove(handle) {
            capture.stop();
        }
    }

    fn allocate(&mut self, formats: &Formats) -> Option<[Buffer; BUFFER_COUNT]> {
        let a = self.create_buffer(formats);
        let b = self.create_buffer(formats);
        match (a, b) {
            (Ok(a), Ok(b)) => Some([a, b]),
            (Err(err), _) | (_, Err(err)) => {
                tracing::error!("cannot allocate capture buffers: {err}");
                None
            }
        }
    }
}

fn frame_interval(fps: &AtomicU32) -> Duration {
    Duration::from_secs_f64(1.0 / fps.load(Ordering::Relaxed).max(1) as f64)
}

impl ScreencopyHandler for AppData {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }

    fn init_done(&mut self, conn: &Connection, qh: &QueueHandle<Self>, session: &CaptureSession, formats: &Formats) {
        let Some(capture) = Capture::for_session(session) else { return };
        let buffers = self.allocate(formats);
        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        let resized = state.formats.as_ref().is_some_and(|f| f.buffer_size != formats.buffer_size);
        state.formats = Some(formats.clone());
        if state.buffers.is_none() || resized {
            state.buffers = buffers;
            state.release = None;
            state.submit(&capture, conn, qh);
        }
    }

    fn ready(&mut self, conn: &Connection, qh: &QueueHandle<Self>, capture_frame: &CaptureFrame, frame: Frame) {
        let Some(capture) = capture_frame.data::<FrameData>().and_then(|d| d.capture.upgrade()) else { return };
        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        let Some(buffers) = state.buffers.as_mut() else { return };

        // Back buffer now holds the newest frame: make it the front.
        buffers.rotate_left(1);
        buffers[0].damage.clear();
        for buffer in &mut buffers[1..] {
            buffer.damage.extend_from_slice(&frame.damage);
        }

        let front = &buffers[0];
        let (subsurface_buffer, release) = SubsurfaceBuffer::new(front.backing.clone());
        let image = CaptureImage {
            buffer: subsurface_buffer,
            width: front.size.0,
            height: front.size.1,
            transform: match frame.transform {
                WEnum::Value(t) => t,
                WEnum::Unknown(_) => cctk::wayland_client::protocol::wl_output::Transform::Normal,
            },
        };
        let previous_release = state.release.replace(release);

        // Next capture: after the previous front buffer is released by the
        // compositor and at least one frame interval since the last submit.
        let wait = frame_interval(&self.fps).saturating_sub(state.last_submit.elapsed());
        let capture_for_task = capture.clone();
        let conn = conn.clone();
        let qh = qh.clone();
        self.thread_pool.spawn_ok(async move {
            if let Some(release) = previous_release {
                release.await;
            }
            if !wait.is_zero() {
                futures_timer::Delay::new(wait).await;
            }
            let mut guard = capture_for_task.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                state.submit(&capture_for_task, &conn, &qh);
            }
        });

        drop(guard);
        self.send_event(Event::Frame(capture.handle.clone(), image));
    }

    fn failed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, capture_frame: &CaptureFrame, reason: WEnum<FailureReason>) {
        let Some(capture) = capture_frame.data::<FrameData>().and_then(|d| d.capture.upgrade()) else { return };
        match reason {
            WEnum::Value(FailureReason::BufferConstraints) => {
                tracing::info!("buffer constraints changed; reallocating");
                let formats = capture.session.lock().unwrap().as_ref().and_then(|s| s.formats.clone());
                let Some(formats) = formats else { return };
                let buffers = self.allocate(&formats);
                let mut guard = capture.session.lock().unwrap();
                if let Some(state) = guard.as_mut() {
                    state.buffers = buffers;
                    state.release = None;
                    state.submit(&capture, conn, qh);
                }
            }
            WEnum::Value(FailureReason::Stopped) => {
                tracing::info!("capture stopped by compositor");
                capture.stop();
            }
            other => {
                tracing::warn!("capture failed: {other:?}; retrying in 500ms");
                let capture_for_task = capture.clone();
                let conn = conn.clone();
                let qh = qh.clone();
                self.thread_pool.spawn_ok(async move {
                    futures_timer::Delay::new(Duration::from_millis(500)).await;
                    let mut guard = capture_for_task.session.lock().unwrap();
                    if let Some(state) = guard.as_mut() {
                        state.submit(&capture_for_task, &conn, &qh);
                    }
                });
            }
        }
    }

    fn stopped(&mut self, _: &Connection, _: &QueueHandle<Self>, session: &CaptureSession) {
        if let Some(capture) = Capture::for_session(session) {
            capture.stop();
        }
    }
}

cctk::delegate_screencopy!(AppData);
```

- [ ] **Step 5: Extend `src/backend/mod.rs`**

Add these imports at the top (alongside the existing ones):

```rust
use cosmic::cctk::{
    screencopy::ScreencopyState,
    sctk::{
        dmabuf::{DmabufFeedback, DmabufState},
        shm::{Shm, ShmHandler},
    },
    wayland_client::protocol::wl_output,
};
use cosmic::iced::futures::executor::ThreadPool;
use cosmic::iced::platform_specific::shell::subsurface_widget::SubsurfaceBuffer;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
```

Add the new modules after `mod toplevels;`:

```rust
mod buffer;
mod capture;
mod dmabuf;
mod gbm_devices;
```

Add `CaptureImage` and the `Frame` event variant:

```rust
#[derive(Clone, Debug)]
pub struct CaptureImage {
    pub buffer: SubsurfaceBuffer,
    pub width: u32,
    pub height: u32,
    pub transform: wl_output::Transform,
}
```

In `pub enum Event`, add `Frame(Handle, CaptureImage),` after `ClientRemoved(Handle),`.

Replace the `AppData` struct with:

```rust
pub struct AppData {
    pub qh: QueueHandle<Self>,
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub toplevel_info_state: ToplevelInfoState,
    pub toplevel_manager_state: Option<ToplevelManagerState>,
    pub screencopy_state: ScreencopyState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_feedback: Option<DmabufFeedback>,
    pub gbm_devices: gbm_devices::GbmDevices,
    pub shm_state: Shm,
    pub captures: HashMap<Handle, Arc<capture::Capture>>,
    pub thread_pool: ThreadPool,
    pub sender: mpsc::Sender<Event>,
    pub app_ids: Vec<String>,
    pub fps: Arc<AtomicU32>,
}
```

In `handle_cmd`, replace the `Cmd::SetFps` arm with:

```rust
            Cmd::SetFps(fps) => {
                self.fps.store(fps, Ordering::Relaxed);
            }
```

In `start`, replace the `AppData { … }` construction with:

```rust
            let dmabuf_state = DmabufState::new(&globals, &qh);
            if let Err(err) = dmabuf_state.get_default_feedback(&qh) {
                tracing::warn!("dmabuf feedback unsupported; shm only: {err}");
            }
            let registry_state = RegistryState::new(&globals);
            let mut app_data = AppData {
                qh: qh.clone(),
                seat_state: SeatState::new(&globals, &qh),
                toplevel_info_state: ToplevelInfoState::new(&registry_state, &qh),
                toplevel_manager_state: ToplevelManagerState::try_new(&registry_state, &qh),
                screencopy_state: ScreencopyState::new(&globals, &qh),
                dmabuf_state,
                dmabuf_feedback: None,
                gbm_devices: gbm_devices::GbmDevices::default(),
                shm_state: Shm::bind(&globals, &qh).expect("wl_shm"),
                captures: HashMap::new(),
                thread_pool: ThreadPool::builder().pool_size(1).create().expect("thread pool"),
                registry_state,
                sender,
                app_ids,
                fps: Arc::new(AtomicU32::new(fps)),
            };
```

Append at the bottom of the file:

```rust
impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

cctk::sctk::delegate_shm!(AppData);
```

- [ ] **Step 6: Start/stop capture from `src/backend/toplevels.rs`**

In `new_toplevel`, after `self.send_event(Event::ClientAdded(...))`, add `self.start_capture(handle);`.

In `update_toplevel`, change the match to:

```rust
        match self.client_info(&info) {
            Some(client) => {
                tracing::debug!(title = %info.title, activated = client.activated, "client updated");
                self.send_event(Event::ClientUpdated(handle.clone(), client));
                self.start_capture(handle);
            }
            None => {
                self.stop_capture(handle);
                self.send_event(Event::ClientRemoved(handle.clone()));
            }
        }
```

In `toplevel_closed`, add `self.stop_capture(handle);` before the `send_event`.

In `reclassify_all`, add `self.start_capture(&handle);` in the `Some` arm and `self.stop_capture(&handle);` in the `None` arm (before each `send_event`; you will need to bind `handle` by reference since it is moved into the event — clone it: `Event::ClientAdded(handle.clone(), client)`).

- [ ] **Step 7: Log frames in the UI for now**

In `src/ui/mod.rs`, add to `on_backend`'s match:

```rust
            Event::Frame(handle, image) => {
                if let Some(client) = self.clients.get(&handle) {
                    tracing::debug!(label = client.info.login.label(), w = image.width, h = image.height, "frame");
                }
            }
```

- [ ] **Step 8: Build and run**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A8 | head -60`
Expected: clean build. If `libc_memfd_create` linking fails, replace the `memfd` helper with `rustix::fs::memfd_create` by adding `rustix = { version = "1", features = ["fs"] }` to `Cargo.toml` — cosmic-workspaces uses that route.

Run: `cd ~/Yutani && RUST_LOG=yutani=debug timeout 6 cargo run -q 2>&1 | grep -E 'gbm|frame|falling back|failed' | head -20`
Expected: `opened gbm device /dev/dri/…` once, then a stream of `frame label="Logging in…" w=… h=…` lines (one per Firefox window per change, throttled to ≤ 30/s each). No `falling back to shm` line — if it appears, the GPU path failed; note it but continue (shm still works, just with CPU cost).

- [ ] **Step 9: Commit**

```bash
cd ~/Yutani && git add src Cargo.toml Cargo.lock
git commit -m "feat: zero-copy capture pipeline with gbm swapchain and fps throttle"
```

---

### Task 6: Floating thumbnails with click-to-focus

One overlay layer surface per client showing the live frame, a coloured border (active vs inactive), the character name, and left-click → activate.

**Files:**
- Create: `src/ui/thumbnail.rs`
- Modify: `src/ui/mod.rs`

**Interfaces:**
- Produces: `ui::thumbnail::view<'a>(client: &'a Client, config: &Config, on_press: Msg) -> Element<'a, Msg>`; `ui::thumbnail::size(config: &Config, image: Option<&CaptureImage>) -> (u32, u32)`; `Client` gains `image: Option<CaptureImage>`, `surface: Option<SurfaceId>`, `position: (i32, i32)`; `Msg::Activate(Handle)`.

- [ ] **Step 1: Write the failing test for `size` in `src/ui/thumbnail.rs`**

```rust
//! The thumbnail widget shared by floating (and, later, dock) modes.

use cosmic::iced::{Border, ContentFit, Length};
use cosmic::iced::platform_specific::shell::subsurface_widget::Subsurface;
use cosmic::widget;
use cosmic::{Element, theme};

use super::{Client, Msg};
use crate::backend::CaptureImage;
use crate::model::config::{Config, parse_color};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_uses_default_aspect_before_first_frame() {
        let config = Config { thumb_width: 320, border_px: 2, ..Config::default() };
        assert_eq!(size(&config, None), (324, 184));
    }

    #[test]
    fn size_follows_captured_aspect() {
        let config = Config { thumb_width: 320, border_px: 2, ..Config::default() };
        assert_eq!(size_for(&config, 2560, 1440), (324, 184));
        assert_eq!(size_for(&config, 1000, 1000), (324, 324));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ~/Yutani && cargo test ui::thumbnail 2>&1 | tail -5`
Expected: compile error — `size`, `size_for` not found (and `ui::thumbnail` module missing until Step 4 registers it; add `pub mod thumbnail;` to `src/ui/mod.rs` now so the error is only about the functions).

- [ ] **Step 3: Implement `size`, `size_for`, and `view`**

Insert above the tests:

```rust
/// Layer-surface size (including border) for a thumbnail.
pub fn size(config: &Config, image: Option<&CaptureImage>) -> (u32, u32) {
    match image {
        Some(img) if img.width > 0 && img.height > 0 => size_for(config, img.width, img.height),
        _ => size_for(config, 16, 9),
    }
}

pub fn size_for(config: &Config, src_w: u32, src_h: u32) -> (u32, u32) {
    let inner_w = config.thumb_width.max(1);
    let inner_h = ((inner_w as u64 * src_h as u64) / src_w.max(1) as u64) as u32;
    (inner_w + 2 * config.border_px, inner_h.max(1) + 2 * config.border_px)
}

pub fn view<'a>(client: &'a Client, config: &Config, on_press: Msg) -> Element<'a, Msg> {
    let border_hex = if client.info.activated { &config.active_border } else { &config.inactive_border };
    let [r, g, b, a] = parse_color(border_hex).unwrap_or([1.0, 0.5, 0.0, 1.0]);
    let border_color = cosmic::iced::Color { r, g, b, a };
    let border_px = config.border_px as f32;

    let image: Element<'a, Msg> = match &client.image {
        Some(img) => Subsurface::new(img.buffer.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(ContentFit::Contain)
            .alpha(config.opacity)
            .transform(img.transform)
            .into(),
        None => widget::container(widget::text("waiting for frame…").size(12))
            .center(Length::Fill)
            .into(),
    };

    let mut layers: Vec<Element<'a, Msg>> = vec![image];
    if config.show_names {
        let label = widget::container(widget::text(client.info.login.label()).size(13))
            .padding([2, 6])
            .class(theme::Container::custom(|_| widget::container::Style {
                background: Some(cosmic::iced::Background::Color(cosmic::iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.6 })),
                border: Border { radius: 4.0.into(), ..Default::default() },
                ..Default::default()
            }));
        layers.push(
            widget::container(label)
                .align_bottom(Length::Fill)
                .align_left(Length::Fill)
                .padding(4)
                .into(),
        );
    }

    let stack = cosmic::iced::widget::stack(layers).width(Length::Fill).height(Length::Fill);

    let framed = widget::container(stack)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(move |_| widget::container::Style {
            border: Border { color: border_color, width: border_px, radius: 4.0.into() },
            ..Default::default()
        }));

    widget::mouse_area(framed).on_press(on_press).into()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ~/Yutani && cargo test ui::thumbnail 2>&1 | tail -5`
Expected: `test result: ok. 2 passed`

- [ ] **Step 5: Create and manage layer surfaces in `src/ui/mod.rs`**

Add imports:

```rust
use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{destroy_layer_surface, get_layer_surface, set_size};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{IcedMargin, IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::window::Id as SurfaceId;
use crate::backend::CaptureImage;

pub mod thumbnail;
```

Replace `pub struct Client` with:

```rust
#[derive(Debug)]
pub struct Client {
    pub info: ClientInfo,
    pub image: Option<CaptureImage>,
    pub surface: Option<SurfaceId>,
    /// Top-left position on its output, logical pixels.
    pub position: (i32, i32),
}
```

Add `Activate(Handle),` to `pub enum Msg`.

Add these methods to `impl App`:

```rust
    /// Output to show a client on: the one it is on, else the first known.
    fn output_for(&self, info: &ClientInfo) -> Option<WlOutput> {
        info.outputs
            .iter()
            .find(|o| self.outputs.iter().any(|known| known.handle == **o))
            .cloned()
            .or_else(|| self.outputs.first().map(|o| o.handle.clone()))
    }

    /// Next free slot: a row along the top, left to right.
    fn next_position(&self) -> (i32, i32) {
        let (w, _) = thumbnail::size(&self.config, None);
        let used = self.clients.values().filter(|c| c.surface.is_some()).count() as i32;
        (40 + used * (w as i32 + 16), 40)
    }

    fn create_surface(&mut self, handle: &Handle) -> Task<Msg> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        if client.surface.is_some() {
            return Task::none();
        }
        let Some(output) = self.output_for(&client.info) else {
            tracing::warn!("no outputs yet; deferring surface");
            return Task::none();
        };
        let position = self.next_position();
        let (width, height) = thumbnail::size(&self.config, client.image.as_ref());
        let id = SurfaceId::unique();
        let client = self.clients.get_mut(handle).unwrap();
        client.surface = Some(id);
        client.position = position;
        get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: IcedOutput::Output(output),
            namespace: "yutani".into(),
            margin: IcedMargin { top: position.1, left: position.0, ..Default::default() },
            size: Some((Some(width), Some(height))),
            exclusive_zone: 0,
            ..Default::default()
        })
    }

    fn destroy_surface(&mut self, handle: &Handle) -> Task<Msg> {
        match self.clients.get_mut(handle).and_then(|c| c.surface.take()) {
            Some(id) => destroy_layer_surface(id),
            None => Task::none(),
        }
    }
```

Replace `on_backend` with:

```rust
    fn on_backend(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::CmdSender(sender) => {
                self.cmd = Some(sender);
                Task::none()
            }
            Event::ClientAdded(handle, info) | Event::ClientUpdated(handle, info) => {
                let entry = self.clients.entry(handle.clone()).or_insert_with(|| Client {
                    info: info.clone(),
                    image: None,
                    surface: None,
                    position: (0, 0),
                });
                entry.info = info;
                self.create_surface(&handle)
            }
            Event::ClientRemoved(handle) => {
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                task
            }
            Event::Frame(handle, image) => {
                let Some(client) = self.clients.get_mut(&handle) else { return Task::none() };
                let old = thumbnail::size(&self.config, client.image.as_ref());
                let new = thumbnail::size(&self.config, Some(&image));
                client.image = Some(image);
                match client.surface {
                    Some(id) if old != new => set_size(id, Some(new.0), Some(new.1)),
                    Some(_) => Task::none(),
                    None => self.create_surface(&handle),
                }
            }
        }
    }
```

In `update`, add the arm:

```rust
            Msg::Activate(handle) => {
                self.send(Cmd::Activate(handle));
                Task::none()
            }
```

Also, outputs can arrive after clients (both come from subscriptions); when an output is first added in `on_output`, create surfaces for any client still lacking one. Change `update`'s `Msg::Wayland(WaylandEvent::Output(...))` arm to:

```rust
            Msg::Wayland(WaylandEvent::Output(event, output)) => {
                let had_outputs = !self.outputs.is_empty();
                self.on_output(event, output);
                if !had_outputs && !self.outputs.is_empty() {
                    let pending: Vec<Handle> = self
                        .clients
                        .iter()
                        .filter(|(_, c)| c.surface.is_none())
                        .map(|(h, _)| h.clone())
                        .collect();
                    return Task::batch(pending.iter().map(|h| self.create_surface(h)));
                }
                Task::none()
            }
```

Replace `view_window` with:

```rust
    fn view_window(&self, id: SurfaceId) -> Element<'_, Msg> {
        match self.clients.iter().find(|(_, c)| c.surface == Some(id)) {
            Some((handle, client)) => thumbnail::view(client, &self.config, Msg::Activate(handle.clone())),
            None => cosmic::widget::text("").into(),
        }
    }
```

- [ ] **Step 6: Build, then test against Firefox windows**

Run: `cd ~/Yutani && cargo build 2>&1 | grep -E '^error' -A8 | head -60`
Expected: clean build. Common fixes: `stack` lives at `cosmic::iced::widget::stack` (or `cosmic::iced_widget::stack`); `container.center(Length::Fill)` may need to be `.center_x(Length::Fill).center_y(Length::Fill)`; `align_bottom`/`align_left` may be `.align_y(Vertical::Bottom).align_x(Horizontal::Left)`.

Run: `cd ~/Yutani && RUST_LOG=yutani=info cargo run -q`
Expected, with `app_ids: ["firefox", "steam_app_8500"]` still in the config:
1. One bordered thumbnail per Firefox window appears in a row at the top-left of your primary output, live-updating as you scroll a page.
2. The focused window's thumbnail has an orange border; the others dark grey.
3. Clicking a thumbnail focuses that Firefox window (border colours swap).
4. Closing a Firefox window removes its thumbnail; opening one adds a thumbnail.
5. `top` shows `yutani` near 0 % CPU while windows are idle and only a few % while one is scrolling.
Stop with Ctrl-C.

- [ ] **Step 7: Test against EVE (acceptance checklist §2a items 1, 4)**

Restore the default config and launch EVE via Steam (GE-Proton, `PROTON_ENABLE_WAYLAND=1 %command%`):

```bash
cat > ~/.config/yutani/config.ron <<'EOF'
(app_ids: ["steam_app_8500"])
EOF
cd ~/Yutani && RUST_LOG=yutani=info cargo run -q
```
Expected: no thumbnail for the launcher; a thumbnail labelled "Logging in…" once the client window opens; the label changes to the character name after login; clicking it focuses the game; the thumbnail stays visible when the game goes fullscreen. If any of these fails, record exactly which and the log output — that is the finding this plan exists to surface.

- [ ] **Step 8: Commit**

```bash
cd ~/Yutani && git add src
git commit -m "feat: floating live thumbnails with active border, name, and click-to-focus"
```

---

## Self-review

**Spec coverage (plan 1 scope):** §2a checklist items 1 and 4 tested in Task 6 Step 7; items 2, 3, 5 need the spec's later features (hotkeys/order, layouts) and belong to plans 2–3, except "two clients from one launcher" which Task 6 already supports (one surface per handle) and item 3 (XWayland) which is the same code path and should be re-run at the end of plan 3. §3 architecture — Task 4. §4 classification — Task 2 (incl. lenient rule). §5 capture — Task 5 (2-buffer swapchain, fps throttle, BufferConstraints reallocation, Stopped, retry on Unknown). §6 thumbnail widget, floating surfaces, left-click activate — Task 6; drag/pin/zoom/Ctrl-click/visibility/dock/settings/tray — plan 2. §7–§9 — plan 3 (config file itself is Task 3). §10 logging — Task 1; missing-protocol exit — `yutani doctor` (the app's own startup check is added in plan 3 with the IPC work). §11 — model tests in Tasks 2, 3, 6.

**Placeholder scan:** none; every code step is complete. Steps that say "likely fixes" list concrete alternatives, not open questions.

**Type consistency:** `Handle`, `ClientInfo`, `Event`, `Cmd`, `CaptureImage` are defined in Task 4/5 `backend/mod.rs` and used with the same names in Tasks 5–6. `Login::label()` (Task 2) is used in Tasks 4–6. `Config` fields (Task 3) match those read in Task 6. `thumbnail::size` returns `(u32, u32)` and `set_size` takes `Option<u32>` — consistent.
