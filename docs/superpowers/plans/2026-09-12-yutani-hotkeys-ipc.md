# Hotkeys, IPC & CLI Implementation Plan (plan 4 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `yutani focus <n> | next | prev | show | hide | toggle | quit` work from a shell and from COSMIC keyboard shortcuts, with `yutani shortcuts install|uninstall` writing the COSMIC custom-shortcuts file.

**Architecture:** A Unix socket server (`$XDG_RUNTIME_DIR/yutani.sock`, tokio, one newline-delimited request per connection) runs as an iced subscription inside the app; requests become `Msg::Ipc` and are answered through a oneshot. The same binary is the CLI client (`src/cli.rs`) — it connects, writes one line, prints the reply and exits. Focus order is a pure function (`rules::focus_order`/`rules::step`). Shortcut install merges our `Spawn(..)` entries into cosmic-settings' `custom` RON map without touching foreign entries; cosmic-comp reloads that file on change.

**Tech Stack:** Rust 2024, libcosmic (pinned rev `a401af8`, tokio runtime), `tokio` 1 (`net`, `io-util`, `sync`), `ron` 0.10 (`RawValue`), `clap` 4, `dirs` 6.

**Spec:** `docs/superpowers/specs/2026-09-11-yutani-design.md` §3 (roles), §7 (hotkeys), §8 (IPC), §9 (`shortcuts` config).

## Global Constraints

- libcosmic pinned to rev `a401af8b1c54a8abd393b8c5b7c8809402f83850`; do not bump.
- New direct deps only: `tokio = { version = "1", features = ["net", "io-util", "sync"] }` (already in `Cargo.lock` via libcosmic — no new `[[package]]` entries).
- IPC socket: `$XDG_RUNTIME_DIR/yutani.sock` (fallback `/tmp/yutani-<uid>.sock` if the variable is unset), mode `0600`, removed on exit. Protocol: one command per connection, newline-terminated; reply `ok\n` or `err <message>\n`.
- Commands (spec §8): `focus <n>`, `next`, `prev`, `show`, `hide`, `toggle`, `layout <name>`, `settings`, `quit`. `layout`/`settings` reply `err not supported yet (settings and layouts arrive in plan 5)` in this plan.
- CLI exit codes: `ok` → 0; `err …` → prints `yutani: <message>` to stderr, exit 1; socket absent → `yutani is not running`, exit 1.
- `focus <n>` is 1-based over **all known clients** in layout order: dock mode → `rules::dock_order` (label, case-insensitive, stable); floating → sorted by (output name, y, x). `next`/`prev` cycle from the currently activated client, wrapping; with no activated client `next` → first, `prev` → last.
- Shortcuts file: `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom` — a RON map `{ (modifiers: [Ctrl, Alt], key: "1"): Spawn("<cmd>"), … }` (`Binding` fields: `modifiers: Vec<Super|Ctrl|Alt|Shift>`, optional `key: String` keysym name, optional `keycode: u32`, optional `description: String`; cosmic-settings-daemon deserialises with `deny_unknown_fields`, so never emit other fields). Install/uninstall touch **only** entries whose action is `Spawn("<path>yutani …")` (first shell word's basename is `yutani`); everything else is preserved byte-for-byte as RON text. Commands use the absolute path of the running executable (cosmic-comp runs `Spawn` through `/bin/sh -c`).
- Default `shortcuts` config: `( focus_prefix: [Ctrl, Alt], next: "Right", prev: "Left" )` — bindings `focus_prefix + "1".."9"` → `focus 1..9`, `focus_prefix + next` → `next`, `focus_prefix + prev` → `prev`.
- Second `yutani` launch while one is running prints `yutani is already running` and exits 1 (checked via the socket before `run_single_instance`).
- Every commit: `cargo build -q` warning-free for new code (pre-existing: `Config::save/save_to`) and `cargo test -q` green.
- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH
  ```

---

## File Structure

| File | Responsibility |
|---|---|
| `src/ipc.rs` (new) | Wire protocol shared by server and client: `socket_path`, `Request` parse/format, `Response` parse/format. Pure, tested. |
| `src/cli.rs` (new) | Client side: connect, send one request, print reply, exit code; `is_running()`. |
| `src/ui/ipc.rs` (new) | Server side: tokio `UnixListener` as an iced subscription yielding `IpcEvent { request, reply }`; stale-socket handling; `remove_socket()`. |
| `src/ui/rules.rs` | `focus_order`, `step` (pure, tested). |
| `src/ui/mod.rs` | `Msg::Ipc`, `handle_request`, socket removal on quit. |
| `src/shortcuts.rs` (new) | COSMIC shortcuts file: `Binding`, `desired`, `merge`, `strip`, `install`, `uninstall`. Pure merge logic tested. |
| `src/model/config.rs` | `ShortcutsConfig`, `Modifier`. |
| `src/main.rs` | clap subcommands; already-running check. |
| `docs/superpowers/specs/2026-09-11-yutani-design.md` | status notes §7/§8. |

---

### Task 1: IPC protocol (`src/ipc.rs`) and CLI client (`src/cli.rs`, `src/main.rs`)

**Files:**
- Create: `src/ipc.rs`, `src/cli.rs`
- Modify: `src/main.rs`, `Cargo.toml`

**Interfaces (produced):**
```rust
// src/ipc.rs
pub fn socket_path() -> PathBuf;
pub enum Request { Focus(usize), Next, Prev, Show, Hide, Toggle, Layout(String), Settings, Quit }
impl Request { pub fn parse(line: &str) -> Result<Request, String>; pub fn to_line(&self) -> String; }
pub enum Response { Ok, Err(String) }
impl Response { pub fn parse(line: &str) -> Response; pub fn to_line(&self) -> String; }
pub const MAX_LINE: usize = 1024;
// src/cli.rs
pub fn send(request: &Request) -> ExitCode;
pub fn is_running() -> bool;
```

- [ ] **Step 1: Dependency**

`Cargo.toml` `[dependencies]`, after `notify = "8"`:
```toml
tokio = { version = "1", features = ["net", "io-util", "sync"] }
```
`cargo build -q` must succeed with no new `[[package]]` in `Cargo.lock` (`git diff --stat Cargo.lock` shows only the `yutani` dependency list).

- [ ] **Step 2: Write `src/ipc.rs` with its tests**

```rust
//! IPC wire protocol between `yutani` (the app, server) and `yutani <cmd>`
//! (the CLI, client). Newline-delimited text, one request per connection.

use std::path::PathBuf;

/// Longest request line the server will read (bytes, including `\n`).
pub const MAX_LINE: usize = 1024;

/// `$XDG_RUNTIME_DIR/yutani.sock`, or `/tmp/yutani-<uid>.sock` without the
/// variable (a bare TTY session).
pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("yutani.sock"),
        _ => PathBuf::from(format!("/tmp/yutani-{}.sock", uid())),
    }
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc_getuid() }
}
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// 1-based index in layout order.
    Focus(usize),
    Next,
    Prev,
    Show,
    Hide,
    Toggle,
    Layout(String),
    Settings,
    Quit,
}

impl Request {
    /// Parse one request line (trailing newline optional). Errors are the
    /// text the server sends back after `err `.
    pub fn parse(line: &str) -> Result<Request, String> {
        let line = line.trim_end_matches(['\n', '\r']);
        let mut words = line.splitn(2, ' ');
        let cmd = words.next().unwrap_or("");
        let arg = words.next().map(str::trim).filter(|a| !a.is_empty());
        match (cmd, arg) {
            ("focus", Some(n)) => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(Request::Focus(n)),
                _ => Err(format!("focus needs a client number from 1, got {n:?}")),
            },
            ("focus", None) => Err("focus needs a client number".into()),
            ("next", None) => Ok(Request::Next),
            ("prev", None) => Ok(Request::Prev),
            ("show", None) => Ok(Request::Show),
            ("hide", None) => Ok(Request::Hide),
            ("toggle", None) => Ok(Request::Toggle),
            ("layout", Some(name)) => Ok(Request::Layout(name.to_string())),
            ("layout", None) => Err("layout needs a name".into()),
            ("settings", None) => Ok(Request::Settings),
            ("quit", None) => Ok(Request::Quit),
            ("", _) => Err("empty request".into()),
            (cmd, Some(_)) if matches!(cmd, "next" | "prev" | "show" | "hide" | "toggle" | "settings" | "quit") => {
                Err(format!("{cmd} takes no argument"))
            }
            (cmd, _) => Err(format!("unknown command {cmd:?}")),
        }
    }

    pub fn to_line(&self) -> String {
        match self {
            Request::Focus(n) => format!("focus {n}\n"),
            Request::Next => "next\n".into(),
            Request::Prev => "prev\n".into(),
            Request::Show => "show\n".into(),
            Request::Hide => "hide\n".into(),
            Request::Toggle => "toggle\n".into(),
            Request::Layout(name) => format!("layout {name}\n"),
            Request::Settings => "settings\n".into(),
            Request::Quit => "quit\n".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Ok,
    Err(String),
}

impl Response {
    pub fn parse(line: &str) -> Response {
        let line = line.trim_end_matches(['\n', '\r']);
        match line.strip_prefix("err") {
            Some(rest) if line == "err" || rest.starts_with(' ') => Response::Err(rest.trim_start().to_string()),
            _ if line == "ok" => Response::Ok,
            _ => Response::Err(format!("malformed reply {line:?}")),
        }
    }

    pub fn to_line(&self) -> String {
        match self {
            Response::Ok => "ok\n".into(),
            Response::Err(msg) => format!("err {}\n", msg.replace('\n', " ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        assert_eq!(Request::parse("focus 3\n"), Ok(Request::Focus(3)));
        assert_eq!(Request::parse("next"), Ok(Request::Next));
        assert_eq!(Request::parse("prev\r\n"), Ok(Request::Prev));
        assert_eq!(Request::parse("show"), Ok(Request::Show));
        assert_eq!(Request::parse("hide"), Ok(Request::Hide));
        assert_eq!(Request::parse("toggle"), Ok(Request::Toggle));
        assert_eq!(Request::parse("layout pvp fleet"), Ok(Request::Layout("pvp fleet".into())));
        assert_eq!(Request::parse("settings"), Ok(Request::Settings));
        assert_eq!(Request::parse("quit"), Ok(Request::Quit));
    }

    #[test]
    fn rejects_bad_requests_with_a_reason() {
        assert!(Request::parse("focus 0").unwrap_err().contains("from 1"));
        assert!(Request::parse("focus x").unwrap_err().contains("client number"));
        assert!(Request::parse("focus").unwrap_err().contains("client number"));
        assert_eq!(Request::parse("layout").unwrap_err(), "layout needs a name");
        assert_eq!(Request::parse("next now").unwrap_err(), "next takes no argument");
        assert_eq!(Request::parse("dance").unwrap_err(), "unknown command \"dance\"");
        assert_eq!(Request::parse("").unwrap_err(), "empty request");
    }

    #[test]
    fn request_lines_round_trip() {
        for r in [
            Request::Focus(9),
            Request::Next,
            Request::Prev,
            Request::Show,
            Request::Hide,
            Request::Toggle,
            Request::Layout("a b".into()),
            Request::Settings,
            Request::Quit,
        ] {
            let line = r.to_line();
            assert!(line.ends_with('\n'));
            assert_eq!(Request::parse(&line), Ok(r));
        }
    }

    #[test]
    fn responses_round_trip_and_tolerate_garbage() {
        assert_eq!(Response::parse("ok\n"), Response::Ok);
        assert_eq!(Response::parse("err no client 4"), Response::Err("no client 4".into()));
        assert_eq!(Response::parse("err"), Response::Err(String::new()));
        assert_eq!(Response::Err("a\nb".into()).to_line(), "err a b\n");
        assert!(matches!(Response::parse("banana"), Response::Err(m) if m.contains("malformed")));
        assert!(matches!(Response::parse("error x"), Response::Err(m) if m.contains("malformed")));
    }

    #[test]
    fn socket_path_follows_runtime_dir() {
        // Only the shape is asserted; the env var is process-global, so it
        // is not mutated here.
        let p = socket_path();
        assert!(p.to_string_lossy().ends_with(".sock"));
        assert!(p.is_absolute());
    }
}
```

- [ ] **Step 3: Run the tests**

Add `mod ipc;` to `src/main.rs` (after `mod doctor;`). Run: `cargo test -q ipc:: 2>&1 | tail -3` → `5 passed`.

- [ ] **Step 4: Write `src/cli.rs`**

```rust
//! `yutani <command>`: send one request over the IPC socket and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::Duration;

use crate::ipc::{Request, Response, socket_path};

const TIMEOUT: Duration = Duration::from_secs(3);

/// True when an app instance is listening on the socket.
pub fn is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Send `request`; print the reply; map it to an exit code.
pub fn send(request: &Request) -> ExitCode {
    let path = socket_path();
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            eprintln!("yutani is not running");
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("yutani: cannot connect to {}: {err}", path.display());
            return ExitCode::from(1);
        }
    };
    if let Err(err) = stream.set_read_timeout(Some(TIMEOUT)).and(stream.set_write_timeout(Some(TIMEOUT))) {
        eprintln!("yutani: socket setup failed: {err}");
        return ExitCode::from(1);
    }
    let mut writer = &stream;
    if let Err(err) = writer.write_all(request.to_line().as_bytes()) {
        eprintln!("yutani: send failed: {err}");
        return ExitCode::from(1);
    }
    let mut line = String::new();
    if let Err(err) = BufReader::new(&stream).read_line(&mut line) {
        eprintln!("yutani: no reply: {err}");
        return ExitCode::from(1);
    }
    match Response::parse(&line) {
        Response::Ok => ExitCode::SUCCESS,
        Response::Err(msg) => {
            eprintln!("yutani: {msg}");
            ExitCode::from(1)
        }
    }
}
```

- [ ] **Step 5: Subcommands in `src/main.rs`**

Replace the `Command` enum and the `match cli.command` with:

```rust
#[derive(Subcommand)]
enum Command {
    /// Check that the compositor supports everything Yutani needs
    Doctor,
    /// Focus the n-th client in layout order (1-based)
    Focus {
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        n: u32,
    },
    /// Focus the next client in layout order
    Next,
    /// Focus the previous client in layout order
    Prev,
    /// Show all thumbnails
    Show,
    /// Hide all thumbnails
    Hide,
    /// Hide the thumbnails if shown, show them if hidden
    Toggle,
    /// Ask the running instance to exit
    Quit,
}
```

and

```rust
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Doctor) => doctor::run(),
        Some(Command::Focus { n }) => Ok(cli::send(&ipc::Request::Focus(n as usize))),
        Some(Command::Next) => Ok(cli::send(&ipc::Request::Next)),
        Some(Command::Prev) => Ok(cli::send(&ipc::Request::Prev)),
        Some(Command::Show) => Ok(cli::send(&ipc::Request::Show)),
        Some(Command::Hide) => Ok(cli::send(&ipc::Request::Hide)),
        Some(Command::Toggle) => Ok(cli::send(&ipc::Request::Toggle)),
        Some(Command::Quit) => Ok(cli::send(&ipc::Request::Quit)),
        None => {
            if cli::is_running() {
                eprintln!("yutani is already running");
                Ok(ExitCode::from(1))
            } else {
                let config = model::config::Config::load();
                let outcome = ui::run(config).map(|()| ExitCode::SUCCESS).map_err(anyhow::Error::from);
                // Belt and braces: the app removes the socket on `quit`, but a
                // panic or SIGTERM path may not get there.
                let _ = std::fs::remove_file(ipc::socket_path());
                outcome
            }
        }
    };
```

Add `mod cli;` next to `mod ipc;`.

- [ ] **Step 6: Build, test, try the client with no server**

Run: `cargo build -q 2>&1 | grep -E '^(warning|error)' -A4; cargo test -q 2>&1 | grep 'test result'; ./target/debug/yutani focus 1; echo "exit=$?"; ./target/debug/yutani focus 0; echo "exit=$?"`
Expected: build clean (the only warning is `Config::save/save_to`); `62 passed`; `yutani is not running` / `exit=1`; clap error `1 is not in 1..` style for `focus 0` with exit 2.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/ipc.rs src/cli.rs src/main.rs
git commit -m "feat(ipc): wire protocol and CLI client (focus/next/prev/show/hide/toggle/quit); 'already running' check

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 2: Focus order (`src/ui/rules.rs`)

**Files:**
- Modify: `src/ui/rules.rs`

**Interfaces (produced):**
```rust
pub struct FocusItem<H> { pub handle: H, pub label: String, pub output: String, pub position: (i32, i32) }
pub fn focus_order<H: Clone>(mode: Mode, items: Vec<FocusItem<H>>) -> Vec<H>;
pub fn step<H: Clone + PartialEq>(order: &[H], active: Option<&H>, forward: bool) -> Option<H>;
```

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `src/ui/rules.rs`)

```rust
    fn item(h: u32, label: &str, output: &str, x: i32, y: i32) -> FocusItem<u32> {
        FocusItem { handle: h, label: label.into(), output: output.into(), position: (x, y) }
    }

    #[test]
    fn dock_focus_order_is_by_label_case_insensitive_and_stable() {
        let items = vec![item(1, "kel", "DP-1", 0, 0), item(2, "Aria", "DP-1", 0, 0), item(3, "Kel", "DP-2", 0, 0)];
        assert_eq!(focus_order(Mode::Dock, items), vec![2, 1, 3]);
    }

    #[test]
    fn floating_focus_order_is_output_then_row_then_column() {
        let items = vec![
            item(1, "z", "DP-2", 10, 10),
            item(2, "y", "DP-1", 500, 40),
            item(3, "x", "DP-1", 40, 40),
            item(4, "w", "DP-1", 40, 400),
        ];
        assert_eq!(focus_order(Mode::Floating, items), vec![3, 2, 4, 1]);
    }

    #[test]
    fn step_wraps_and_handles_no_active() {
        let order = [10, 20, 30];
        assert_eq!(step(&order, Some(&20), true), Some(30));
        assert_eq!(step(&order, Some(&30), true), Some(10));
        assert_eq!(step(&order, Some(&10), false), Some(30));
        assert_eq!(step(&order, None, true), Some(10));
        assert_eq!(step(&order, None, false), Some(30));
        // Active client unknown to the order (e.g. mid-update): treat as none.
        assert_eq!(step(&order, Some(&99), true), Some(10));
        assert_eq!(step::<u32>(&[], None, true), None);
    }
```

Add `use crate::model::config::Mode;` to the test module imports if `Mode` isn't already imported at the top of `rules.rs` (check: `should_show` uses `Visibility`, so the `config` import exists — extend it).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -q rules:: 2>&1 | grep -E 'error\[|cannot find' | head -3` → `cannot find function focus_order`.

- [ ] **Step 3: Implement** (append to `src/ui/rules.rs`, above `mod tests`)

```rust
/// What `focus_order` needs to know about one client.
pub struct FocusItem<H> {
    pub handle: H,
    pub label: String,
    /// Connector name of the output the thumbnail is on.
    pub output: String,
    /// Thumbnail top-left, logical px (floating mode).
    pub position: (i32, i32),
}

/// Layout order used by `focus <n>`, `next` and `prev` (spec §7): dock
/// order by label; floating by (output, y, x). Stable for ties.
pub fn focus_order<H: Clone>(mode: Mode, mut items: Vec<FocusItem<H>>) -> Vec<H> {
    match mode {
        Mode::Dock => items.sort_by_key(|i| i.label.to_lowercase()),
        Mode::Floating => items.sort_by(|a, b| {
            (a.output.as_str(), a.position.1, a.position.0).cmp(&(b.output.as_str(), b.position.1, b.position.0))
        }),
    }
    items.into_iter().map(|i| i.handle).collect()
}

/// Next (or previous) handle in `order` after `active`, wrapping. With no
/// active client — or one not in `order` — `next` is the first, `prev` the
/// last.
pub fn step<H: Clone + PartialEq>(order: &[H], active: Option<&H>, forward: bool) -> Option<H> {
    if order.is_empty() {
        return None;
    }
    let idx = active.and_then(|a| order.iter().position(|h| h == a));
    let next = match (idx, forward) {
        (Some(i), true) => (i + 1) % order.len(),
        (Some(i), false) => (i + order.len() - 1) % order.len(),
        (None, true) => 0,
        (None, false) => order.len() - 1,
    };
    Some(order[next].clone())
}
```

Add `Mode` to the existing `use crate::model::config::{…}` line at the top of `rules.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test -q rules:: 2>&1 | tail -2` → all `rules::` tests pass (previous count + 3). Full: `cargo test -q 2>&1 | grep 'test result'` → `65 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/ui/rules.rs
git commit -m "feat(rules): focus_order and step for focus/next/prev

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 3: IPC server subscription and request handling in the app

**Files:**
- Create: `src/ui/ipc.rs`
- Modify: `src/ui/mod.rs` (`Msg`, `update`, `subscription`, new `handle_request`, quit path)

**Interfaces:**
- Consumes: `crate::ipc::{Request, Response, socket_path, MAX_LINE}` (Task 1); `rules::{focus_order, step, FocusItem}` (Task 2); existing `App::send(Cmd)`, `Cmd::Activate`, `App.hidden`, `reconcile_surfaces`, `Output.name`, `Client.position`, `Client.info.{login,activated,outputs}`, `output_for`.
- Produces: `ui::ipc::subscription() -> Subscription<IpcEvent>`; `IpcEvent { request: Request, reply: Responder }`; `Responder::respond(self, Response)`; `ui::ipc::remove_socket()`.

- [ ] **Step 1: Write `src/ui/ipc.rs`**

```rust
//! IPC server: `$XDG_RUNTIME_DIR/yutani.sock` as an iced subscription. Each
//! connection carries one request; the app answers through `Responder`.

use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::{SinkExt, StreamExt};
use cosmic::iced::{self, Subscription};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

use crate::ipc::{MAX_LINE, Request, Response, socket_path};

/// One-shot reply channel. `Clone` (so it can live in a `Msg`) but only the
/// first `respond` is delivered.
#[derive(Clone, Debug)]
pub struct Responder(Arc<Mutex<Option<oneshot::Sender<Response>>>>);

impl Responder {
    pub fn respond(&self, response: Response) {
        if let Some(tx) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(response);
        }
    }
}

#[derive(Clone, Debug)]
pub struct IpcEvent {
    pub request: Request,
    pub reply: Responder,
}

pub fn subscription() -> Subscription<IpcEvent> {
    Subscription::run(run)
}

/// Delete the socket file (on quit). Safe to call when it doesn't exist.
pub fn remove_socket() {
    let _ = std::fs::remove_file(socket_path());
}

/// Bind the listener, replacing a stale socket left by a crashed instance
/// but never one that still answers.
fn bind() -> std::io::Result<UnixListener> {
    let path = socket_path();
    if path.exists() {
        if StdUnixStream::connect(&path).is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "another yutani instance is listening"));
        }
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!("ipc listening on {}", path.display());
    Ok(listener)
}

fn run() -> impl iced::futures::Stream<Item = IpcEvent> {
    let (tx, rx) = mpsc::channel::<IpcEvent>(16);
    // The accept loop never yields items itself; it feeds `tx`. Selecting it
    // with `rx` keeps it alive for as long as the subscription runs.
    let accept_loop = iced::futures::stream::once(async move {
        let listener = match bind() {
            Ok(l) => l,
            Err(err) => {
                tracing::warn!("ipc unavailable: {err}; `yutani focus` and shortcuts will not work");
                return;
            }
        };
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!("ipc accept failed: {err}");
                    continue;
                }
            };
            let mut tx = tx.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut line = String::new();
                let mut reader = BufReader::new(read).take(MAX_LINE as u64);
                let response = match reader.read_line(&mut line).await {
                    Ok(0) => Response::Err("empty request".into()),
                    Ok(_) if !line.ends_with('\n') && line.len() >= MAX_LINE => Response::Err("request too long".into()),
                    Ok(_) => match Request::parse(&line) {
                        Err(msg) => Response::Err(msg),
                        Ok(request) => {
                            let (reply_tx, reply_rx) = oneshot::channel();
                            let reply = Responder(Arc::new(Mutex::new(Some(reply_tx))));
                            if tx.send(IpcEvent { request, reply }).await.is_err() {
                                Response::Err("app is shutting down".into())
                            } else {
                                reply_rx.await.unwrap_or_else(|_| Response::Err("no reply from app".into()))
                            }
                        }
                    },
                    Err(err) => Response::Err(format!("read failed: {err}")),
                };
                let _ = write.write_all(response.to_line().as_bytes()).await;
                let _ = write.shutdown().await;
            });
        }
    })
    .filter_map(|()| async { None::<IpcEvent> });
    iced::futures::stream::select(accept_loop, rx)
}
```

(`tokio::spawn` is fine here: libcosmic's iced runtime is tokio, and subscriptions run on it.)

- [ ] **Step 2: Wire into `src/ui/mod.rs`**

- `mod ipc;` next to `mod config_watch;` (note the crate-root `crate::ipc` vs `self::ipc` — refer to the server module as `ipc::` inside `ui`, and the protocol as `crate::ipc::`).
- `Msg` gains `Ipc(ipc::IpcEvent)`.
- `subscription`: add `ipc::subscription().map(Msg::Ipc)` to `subs`.
- `update`: `Msg::Ipc(ev) => { let (result, task) = self.handle_request(&ev.request); ev.reply.respond(match result { Ok(()) => crate::ipc::Response::Ok, Err(m) => crate::ipc::Response::Err(m) }); task }`
- Quit paths: change `Msg::Tray(tray::TrayEvent::Quit) => cosmic::iced::exit(),` to `Msg::Tray(tray::TrayEvent::Quit) => { ipc::remove_socket(); cosmic::iced::exit() }`.
- Add to `impl App`:

```rust
    /// Layout order of every known client (spec §7).
    fn focus_order(&self) -> Vec<Handle> {
        let items = self
            .clients
            .iter()
            .map(|(h, c)| rules::FocusItem {
                handle: h.clone(),
                label: c.info.login.label().to_string(),
                output: self
                    .output_for(&c.info)
                    .and_then(|o| self.outputs.iter().find(|k| k.handle == o).map(|k| k.name.clone()))
                    .unwrap_or_default(),
                position: c.position,
            })
            .collect();
        rules::focus_order(self.config.mode, items)
    }

    fn active_client(&self) -> Option<Handle> {
        self.clients.iter().find(|(_, c)| c.info.activated).map(|(h, _)| h.clone())
    }

    /// Execute one IPC request. `Err` is the text sent back after `err `.
    fn handle_request(&mut self, request: &crate::ipc::Request) -> (Result<(), String>, Task<cosmic::Action<Msg>>) {
        use crate::ipc::Request;
        match request {
            Request::Focus(n) => {
                let order = self.focus_order();
                match order.get(n - 1) {
                    Some(h) => {
                        self.send(Cmd::Activate(h.clone()));
                        (Ok(()), Task::none())
                    }
                    None => (Err(format!("no client {n} ({} known)", order.len())), Task::none()),
                }
            }
            Request::Next | Request::Prev => {
                let order = self.focus_order();
                let active = self.active_client();
                match rules::step(&order, active.as_ref(), matches!(request, Request::Next)) {
                    Some(h) => {
                        self.send(Cmd::Activate(h));
                        (Ok(()), Task::none())
                    }
                    None => (Err("no clients".into()), Task::none()),
                }
            }
            Request::Show => (Ok(()), self.set_hidden(false)),
            Request::Hide => (Ok(()), self.set_hidden(true)),
            Request::Toggle => {
                let h = !self.hidden;
                (Ok(()), self.set_hidden(h))
            }
            Request::Layout(_) | Request::Settings => {
                (Err("not supported yet (settings and layouts arrive in plan 5)".into()), Task::none())
            }
            Request::Quit => {
                ipc::remove_socket();
                (Ok(()), cosmic::iced::exit())
            }
        }
    }

    fn set_hidden(&mut self, hidden: bool) -> Task<cosmic::Action<Msg>> {
        if self.hidden == hidden {
            return Task::none();
        }
        self.hidden = hidden;
        self.reconcile_surfaces()
    }
```

and make the two tray arms use `set_hidden` too (`ToggleVisibility` → `let h = !self.hidden; self.set_hidden(h)`; `SetHidden(h)` → `self.set_hidden(h)`), so there is one visibility path.

Note on `Quit`: the reply is sent by `update` *after* `handle_request` returns, and `cosmic::iced::exit()` is a `Task` that runs afterwards — so the `ok` reply is delivered before the process exits. If in the smoke test `yutani quit` prints `no reply`, the exit raced the write: change the Quit arm to return `Task::none()` and instead schedule the exit via `Task::future(async { futures_timer::Delay::new(std::time::Duration::from_millis(50)).await; }).then(|_| cosmic::iced::exit())`. Document whichever was needed.

- [ ] **Step 3: Build and test**

Run: `cargo build -q 2>&1 | grep -E '^(warning|error)' -A6; cargo test -q 2>&1 | grep 'test result'` → clean; `65 passed`.

- [ ] **Step 4: Smoke (EVE running, no yutani running)**

```bash
LOG=/tmp/claude-1000/-home-user-Yutani/eb30ce00-6f26-46d7-ad81-bfd5bce301f6/scratchpad/yutani_ipc.log
(RUST_LOG=yutani=debug setsid nohup ./target/debug/yutani >$LOG 2>&1 &); sleep 4
ls -la $XDG_RUNTIME_DIR/yutani.sock                      # srw------- 
./target/debug/yutani; echo "exit=$?"                     # yutani is already running / exit=1
./target/debug/yutani focus 1; echo "exit=$?"             # exit=0; EVE gets focus (grep Activate in log)
./target/debug/yutani focus 9; echo "exit=$?"             # yutani: no client 9 (1 known) / exit=1
./target/debug/yutani next; echo "exit=$?"                # exit=0
./target/debug/yutani hide; sleep 1; grep -c destroy_surface $LOG   # ≥1 — thumbnail gone
./target/debug/yutani show; sleep 1; grep -c create_surface $LOG    # ≥2 — back
./target/debug/yutani toggle; ./target/debug/yutani toggle; echo "exit=$?"
printf 'bogus\n' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/yutani.sock   # err unknown command "bogus"  (if socat missing: python3 -c 'import socket,os;s=socket.socket(socket.AF_UNIX);s.connect(os.environ["XDG_RUNTIME_DIR"]+"/yutani.sock");s.sendall(b"bogus\n");print(s.recv(100))')
./target/debug/yutani quit; echo "exit=$?"; sleep 1
pgrep -f 'target/debug/yutani$' || echo "exited"; ls $XDG_RUNTIME_DIR/yutani.sock 2>&1   # exited; No such file
sed 's/\x1b\[[0-9;]*m//g' $LOG | grep -E 'ipc|panic|error' | cut -c1-140
```

- [ ] **Step 5: Commit**

```bash
git add src/ui/ipc.rs src/ui/mod.rs
git commit -m "feat(ipc): socket server subscription; focus/next/prev/show/hide/toggle/quit handled by the app

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 4: COSMIC shortcuts install/uninstall

**Files:**
- Modify: `src/model/config.rs` (`ShortcutsConfig`, `Modifier`)
- Create: `src/shortcuts.rs`
- Modify: `src/main.rs` (`shortcuts install|uninstall`)
- Modify: `docs/superpowers/specs/2026-09-11-yutani-design.md` (§7/§8 status)

**Interfaces:**
- Consumes: `Config` (Task-independent), `crate::ipc::Request::to_line` is *not* used — commands are plain strings.
- Produces:
```rust
// config.rs
pub enum Modifier { Super, Ctrl, Alt, Shift }
pub struct ShortcutsConfig { pub focus_prefix: Vec<Modifier>, pub next: String, pub prev: String }
// Config gains `pub shortcuts: ShortcutsConfig`
// shortcuts.rs
pub struct Binding { pub modifiers: Vec<Modifier>, pub key: Option<String>, pub keycode: Option<u32>, pub description: Option<String> }
pub fn desired(cfg: &ShortcutsConfig, exe: &str) -> Vec<(Binding, String)>;
pub fn is_ours(action_ron: &str) -> bool;
pub fn merge(existing: &str, ours: &[(Binding, String)]) -> anyhow::Result<String>;
pub fn strip(existing: &str) -> anyhow::Result<String>;
pub fn custom_path() -> PathBuf;
pub fn install(cfg: &ShortcutsConfig) -> anyhow::Result<usize>;   // entries written
pub fn uninstall() -> anyhow::Result<usize>;                       // entries removed
```

- [ ] **Step 1: Config — failing tests first** (append to `mod tests` in `src/model/config.rs`)

```rust
    #[test]
    fn shortcuts_default_and_parse() {
        let c = Config::default();
        assert_eq!(c.shortcuts.focus_prefix, vec![Modifier::Ctrl, Modifier::Alt]);
        assert_eq!(c.shortcuts.next, "Right");
        assert_eq!(c.shortcuts.prev, "Left");
        let c: Config = ron::from_str("(shortcuts: (focus_prefix: [Super], next: \"n\", prev: \"p\"))").unwrap();
        assert_eq!(c.shortcuts.focus_prefix, vec![Modifier::Super]);
        assert_eq!(c.shortcuts.next, "n");
        // Partial override keeps the other defaults.
        let c: Config = ron::from_str("(shortcuts: (next: \"Tab\"))").unwrap();
        assert_eq!(c.shortcuts.prev, "Left");
    }

    #[test]
    fn validate_rejects_empty_shortcut_keys() {
        let mut c = Config::default();
        c.shortcuts.next = String::new();
        let c = c.validate();
        assert_eq!(c.shortcuts.next, "Right");
    }
```

- [ ] **Step 2: Config — implement**

In `src/model/config.rs`, after `Edge`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}

/// Keys for `yutani shortcuts install` (spec §7). `next`/`prev` are xkb
/// keysym names as COSMIC writes them ("Right", "Left", "Tab", "a").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    pub focus_prefix: Vec<Modifier>,
    pub next: String,
    pub prev: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self { focus_prefix: vec![Modifier::Ctrl, Modifier::Alt], next: "Right".into(), prev: "Left".into() }
    }
}
```

Add `pub shortcuts: ShortcutsConfig,` as the last `Config` field (doc: `/// Keyboard shortcuts written by \`yutani shortcuts install\`.`) and `shortcuts: ShortcutsConfig::default(),` in `Default`. In `validate`, after the existing `check!` lines:

```rust
        if self.shortcuts.next.trim().is_empty() || self.shortcuts.prev.trim().is_empty() {
            tracing::warn!("config: shortcuts.next/prev must not be empty; using defaults");
            self.shortcuts = ShortcutsConfig::default();
        }
```

Run: `cargo test -q config:: 2>&1 | tail -2` → all config tests pass. (If `defaults_match_spec`/`round_trips_through_ron` need no change, good — `#[serde(default)]` keeps old files parsing.)

- [ ] **Step 3: `src/shortcuts.rs` — failing tests first**

Create the file with the module doc, the `Binding` type, and the tests; leave the functions as `todo!()`-free stubs only long enough to see the tests fail to compile, then implement in Step 4. Tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::Modifier;

    fn cfg() -> ShortcutsConfig {
        ShortcutsConfig::default()
    }

    #[test]
    fn desired_has_eleven_entries_with_absolute_commands() {
        let d = desired(&cfg(), "/opt/yutani/bin/yutani");
        assert_eq!(d.len(), 11);
        assert_eq!(d[0].0, Binding { modifiers: vec![Modifier::Ctrl, Modifier::Alt], key: Some("1".into()), keycode: None, description: Some("Yutani: focus client 1".into()) });
        assert_eq!(d[0].1, "/opt/yutani/bin/yutani focus 1");
        assert_eq!(d[8].1, "/opt/yutani/bin/yutani focus 9");
        assert_eq!(d[9].0.key.as_deref(), Some("Right"));
        assert_eq!(d[9].1, "/opt/yutani/bin/yutani next");
        assert_eq!(d[10].1, "/opt/yutani/bin/yutani prev");
    }

    #[test]
    fn is_ours_matches_spawn_of_yutani_only() {
        assert!(is_ours(r#"Spawn("yutani focus 1")"#));
        assert!(is_ours(r#"Spawn("/home/d/Yutani/target/debug/yutani next")"#));
        assert!(is_ours(r#" Spawn( "/usr/bin/yutani prev" ) "#));
        assert!(!is_ours(r#"Spawn("cosmic-term")"#));
        assert!(!is_ours(r#"Spawn("notyutani focus 1")"#));
        assert!(!is_ours(r#"Spawn("/usr/bin/yutani-helper x")"#));
        assert!(!is_ours("Close"));
    }

    const FOREIGN: &str = r#"{
    (modifiers: [Super], key: "t", description: Some("Terminal")): Spawn("cosmic-term"),
    (modifiers: [Super, Shift], key: "q"): Close,
}"#;

    #[test]
    fn merge_keeps_foreign_entries_and_adds_ours() {
        let out = merge(FOREIGN, &desired(&cfg(), "yutani")).unwrap();
        assert!(out.contains(r#"Spawn("cosmic-term")"#));
        assert!(out.contains("Close"));
        assert!(out.contains(r#"(modifiers: [Ctrl, Alt], key: "1", description: Some("Yutani: focus client 1")): Spawn("yutani focus 1")"#));
        assert!(out.contains(r#"key: "Right""#));
        // Parses back as a RON map with 13 entries.
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 13);
    }

    #[test]
    fn merge_replaces_stale_yutani_entries_even_on_other_keys() {
        let stale = r#"{ (modifiers: [Super], key: "F1"): Spawn("/old/yutani focus 1"), (modifiers: [Super], key: "t"): Spawn("cosmic-term") }"#;
        let out = merge(stale, &desired(&cfg(), "yutani")).unwrap();
        assert!(!out.contains("/old/yutani"));
        assert!(out.contains(r#"Spawn("cosmic-term")"#));
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 12);
    }

    #[test]
    fn merge_and_strip_handle_missing_or_empty_file() {
        let out = merge("", &desired(&cfg(), "yutani")).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 11);
        assert_eq!(strip("").unwrap().trim(), "{}");
        let back = strip(&out).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&back).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn strip_keeps_only_foreign_entries() {
        let mixed = merge(FOREIGN, &desired(&cfg(), "yutani")).unwrap();
        let back = strip(&mixed).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&back).unwrap();
        assert_eq!(map.len(), 2);
        assert!(back.contains("cosmic-term"));
    }

    #[test]
    fn merge_rejects_unparseable_input_instead_of_clobbering() {
        assert!(merge("{ this is not ron", &desired(&cfg(), "yutani")).is_err());
    }

    #[test]
    fn shell_quotes_paths_with_spaces() {
        let d = desired(&cfg(), "/home/me/My Apps/yutani");
        assert_eq!(d[0].1, "'/home/me/My Apps/yutani' focus 1");
        assert!(is_ours(r#"Spawn("'/home/me/My Apps/yutani' focus 1")"#));
    }
}
```

- [ ] **Step 4: `src/shortcuts.rs` — implement**

```rust
//! `yutani shortcuts install|uninstall`: our entries in COSMIC's custom
//! keyboard-shortcuts file (spec §7). Only entries whose action spawns
//! `yutani` are ever added, replaced or removed; everything else in the
//! file is preserved as the RON text it was.

use anyhow::Context as _;
use ron::value::RawValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::config::{Modifier, ShortcutsConfig};

/// cosmic-settings-daemon's `Binding`, field-for-field (it deserialises
/// with `deny_unknown_fields`, so nothing may be added here).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Binding {
    pub modifiers: Vec<Modifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub fn custom_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom")
}

/// `exe` quoted for `/bin/sh -c` if it needs it.
fn shell_word(exe: &str) -> String {
    if exe.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        exe.to_string()
    } else {
        format!("'{}'", exe.replace('\'', r"'\''"))
    }
}

/// The bindings we want, in a stable order: focus 1–9, next, prev.
pub fn desired(cfg: &ShortcutsConfig, exe: &str) -> Vec<(Binding, String)> {
    let exe = shell_word(exe);
    let bind = |key: &str, description: String| Binding {
        modifiers: cfg.focus_prefix.clone(),
        key: Some(key.to_string()),
        keycode: None,
        description: Some(description),
    };
    let mut out: Vec<(Binding, String)> = (1..=9)
        .map(|n| (bind(&n.to_string(), format!("Yutani: focus client {n}")), format!("{exe} focus {n}")))
        .collect();
    out.push((bind(&cfg.next, "Yutani: next client".into()), format!("{exe} next")));
    out.push((bind(&cfg.prev, "Yutani: previous client".into()), format!("{exe} prev")));
    out
}

/// True for `Spawn("<something>yutani <args>")` where the command's first
/// shell word is `yutani` or a path ending in `/yutani`.
pub fn is_ours(action_ron: &str) -> bool {
    let Ok(action) = RawValue::from_ron(action_ron.trim()) else { return false };
    let Ok(SpawnOnly::Spawn(cmd)) = action.into_rust::<SpawnOnly>() else { return false };
    let first = cmd.trim_start();
    let word = if let Some(rest) = first.strip_prefix('\'') {
        rest.split('\'').next().unwrap_or("")
    } else {
        first.split_whitespace().next().unwrap_or("")
    };
    word == "yutani" || word.ends_with("/yutani")
}

/// Just enough of cosmic's `Action` to recognise `Spawn`.
#[derive(Deserialize)]
enum SpawnOnly {
    Spawn(String),
}

type Entries = BTreeMap<Binding, Box<RawValue>>;

fn parse(existing: &str) -> anyhow::Result<Entries> {
    if existing.trim().is_empty() {
        return Ok(Entries::new());
    }
    ron::from_str(existing).context("cannot parse the existing custom shortcuts file; not touching it")
}

fn render(entries: &Entries) -> anyhow::Result<String> {
    let config = ron::ser::PrettyConfig::new().depth_limit(2).struct_names(false);
    Ok(ron::ser::to_string_pretty(entries, config)?)
}

/// `existing` with every yutani entry removed and `ours` added.
pub fn merge(existing: &str, ours: &[(Binding, String)]) -> anyhow::Result<String> {
    let mut entries = parse(existing)?;
    entries.retain(|_, action| !is_ours(action.get_ron()));
    for (binding, cmd) in ours {
        let action = ron::to_string(&SpawnOut::Spawn(cmd.clone()))?;
        entries.insert(binding.clone(), RawValue::from_boxed_ron(action.into_boxed_str())?);
    }
    render(&entries)
}

#[derive(Serialize)]
enum SpawnOut {
    Spawn(String),
}

/// `existing` with every yutani entry removed.
pub fn strip(existing: &str) -> anyhow::Result<String> {
    let mut entries = parse(existing)?;
    entries.retain(|_, action| !is_ours(action.get_ron()));
    render(&entries)
}

fn read_existing(path: &std::path::Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn write_atomic(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    let dir = path.parent().context("shortcuts path has no parent")?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("yutani.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Write our bindings; returns how many were written.
pub fn install(cfg: &ShortcutsConfig) -> anyhow::Result<usize> {
    let exe = std::env::current_exe().context("current_exe")?;
    let ours = desired(cfg, &exe.to_string_lossy());
    let path = custom_path();
    let merged = merge(&read_existing(&path)?, &ours)?;
    write_atomic(&path, &merged)?;
    Ok(ours.len())
}

/// Remove our bindings; returns how many were removed.
pub fn uninstall() -> anyhow::Result<usize> {
    let path = custom_path();
    let existing = read_existing(&path)?;
    let before = parse(&existing)?.len();
    let stripped = strip(&existing)?;
    let after = parse(&stripped)?.len();
    write_atomic(&path, &stripped)?;
    Ok(before - after)
}
```

Notes for the implementer:
- `Modifier` in `config.rs` needs `PartialOrd, Ord` derives for `Binding: Ord` (add them).
- `RawValue::from_boxed_ron` returns `SpannedResult`; `?` into `anyhow` works because `ron::error::SpannedError: std::error::Error`.
- If `to_string_pretty` renders map keys with `struct_names` or extra newlines that break the assertion in `merge_keeps_foreign_entries_and_adds_ours`, adjust `PrettyConfig` (e.g. `.compact_structs(true)` or `.new_line("\n")`) until keys render on one line as `(modifiers: [Ctrl, Alt], key: "1", description: Some("…"))` — that exact shape is what cosmic-settings writes. Do not weaken the test.
- `ron::to_string(&SpawnOut::Spawn(..))` renders `Spawn("…")`.

Add `mod shortcuts;` to `src/main.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -q shortcuts:: 2>&1 | tail -3` → `8 passed`; `cargo test -q 2>&1 | grep 'test result'` → `75 passed`.

- [ ] **Step 6: CLI subcommand**

In `src/main.rs` add to `Command`:

```rust
    /// Install or remove the COSMIC keyboard shortcuts (Ctrl+Alt+1..9, Right, Left by default)
    Shortcuts {
        #[command(subcommand)]
        action: ShortcutsAction,
    },
```

```rust
#[derive(Subcommand)]
enum ShortcutsAction {
    /// Write Yutani's bindings into COSMIC's custom shortcuts (idempotent)
    Install,
    /// Remove Yutani's bindings, leaving everything else untouched
    Uninstall,
}
```

and the match arms:

```rust
        Some(Command::Shortcuts { action: ShortcutsAction::Install }) => {
            let config = model::config::Config::load();
            shortcuts::install(&config.shortcuts).map(|n| {
                println!("installed {n} shortcuts into {}", shortcuts::custom_path().display());
                ExitCode::SUCCESS
            })
        }
        Some(Command::Shortcuts { action: ShortcutsAction::Uninstall }) => shortcuts::uninstall().map(|n| {
            println!("removed {n} shortcuts from {}", shortcuts::custom_path().display());
            ExitCode::SUCCESS
        }),
```

- [ ] **Step 7: Smoke**

```bash
cargo build -q 2>&1 | grep -E '^(warning|error)' -A4
P=~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom
cp -n $P $P.bak 2>/dev/null || echo "(no pre-existing custom file)"
./target/debug/yutani shortcuts install && cat $P
./target/debug/yutani shortcuts install && grep -c 'yutani' $P     # idempotent: still 11
```
Then start the app (`(RUST_LOG=yutani=info setsid nohup ./target/debug/yutani >/tmp/claude-1000/-home-user-Yutani/eb30ce00-6f26-46d7-ad81-bfd5bce301f6/scratchpad/yutani_sc.log 2>&1 &)`), and **ask Daniel** to press `Ctrl+Alt+1` with a non-EVE window focused: EVE must come to the front; `Ctrl+Alt+Right`/`Left` cycle (one client → stays). Then:
```bash
./target/debug/yutani shortcuts uninstall && cat $P               # {} or only foreign entries
./target/debug/yutani quit
[ -f $P.bak ] && mv $P.bak $P || rm -f $P
```
If cosmic-comp does not pick the new file up live, log out/in is the documented fallback — record what happened in the report.

- [ ] **Step 8: Spec status and commit**

In `docs/superpowers/specs/2026-09-11-yutani-design.md`: at the end of §7 add *Status after plan 4: implemented as `yutani shortcuts install|uninstall` (CLI; the settings-window buttons come with plan 5). Commands are written with the absolute path of the installed binary.* At the end of §8 add *Status after plan 4: implemented; `layout` and `settings` reply `err not supported yet` until plan 5. A second `yutani` launch prints `yutani is already running`.*

```bash
git add src/model/config.rs src/shortcuts.rs src/main.rs docs/superpowers/specs/2026-09-11-yutani-design.md
git commit -m "feat(shortcuts): yutani shortcuts install|uninstall writes COSMIC custom shortcuts; shortcuts config section

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

## Self-review

**Spec coverage:** §3 CLI role → Task 1; §8 socket path/mode/removal/protocol/replies/CLI exit codes → Tasks 1 & 3; §8 command set (incl. `layout`/`settings` stubs) → Tasks 1 & 3; §7 shortcut file format, install/uninstall touching only `Spawn("yutani …")`, default bindings, `focus <n>` order and `next`/`prev` wrap semantics → Tasks 2 & 4; §9 `shortcuts` config → Task 4; ledger follow-up "already running message via IPC" → Task 1. Not here (plan 5): settings window, named layouts, tray Layouts/Settings items.

**Placeholder scan:** none. The one API uncertainty (`futures` `select` vs a non-existent `merge_with`) is resolved inline in Task 3 Step 1; the RON pretty-print shape carries an explicit acceptance criterion instead of a guess.

**Type consistency:** `Request`/`Response` names and variants identical across Tasks 1 and 3; `rules::FocusItem { handle, label, output, position }`, `focus_order(mode, items)`, `step(order, active, forward)` identical in Tasks 2 and 3; `Modifier`/`ShortcutsConfig` defined in Task 4 Step 2 and used in Step 4; `ipc::Responder::respond`, `ipc::remove_socket`, `IpcEvent { request, reply }` consistent between Task 3 Steps 1 and 2; test totals 57 → 62 → 65 → 75.
