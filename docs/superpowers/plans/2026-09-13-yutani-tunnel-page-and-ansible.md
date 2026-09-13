# Tunnel Settings Page and Ansible Deployment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Tunnel" page in the settings window that takes the Proton VPN WireGuard `.conf` (browse, type, or drop) and installs/uninstalls/connects the tunnel; and an Ansible role that deploys Yutani end to end.

**Architecture:** Task 1 adds `src/ui/tunnel_page.rs` (state + view + pure helpers, like `src/ui/characters.rs`), new `settings::Msg` variants, a `Page::Tunnel` tab, a `FileDropped` window-event route in the daemon's subscription, and daemon wiring that runs the existing `tunnel::install::install` / `uninstall` / `control::connect` / `disconnect` on tokio's blocking pool. Task 2 is pure deployment tooling under `deploy/ansible/` plus a root `README.md`; it touches no Rust.

**Tech Stack:** Rust 2024, libcosmic rev `a401af8` (+ its `xdg-portal` feature for the file chooser), tokio blocking pool; Ansible ≥ 2.15 (installed locally: ansible 14.4), Arch/CachyOS `pacman`.

**Spec:** `docs/superpowers/specs/2026-09-13-yutani-tunnel-page-and-ansible-design.md`.

## Global Constraints

- Page order in `settings::PAGES`: Display, Behavior, Layouts, Characters, Tunnel, Steam (`[(&str, Page); 6]`).
- The only Cargo change allowed: add `"xdg-portal"` to the libcosmic `features` list in `Cargo.toml`. If that feature does not build at rev `a401af8`, drop it, leave the Browse… button out, and say so in the report — the typed path and drag-and-drop must still work.
- Private keys are never displayed, logged, or put in a note. The note may name the file (`file_name()`) and the parsed `location` only.
- All tunnel actions run through `tokio::task::spawn_blocking` inside `cosmic::iced::Task::perform` (the pattern at `src/ui/mod.rs` `Request::Quit` / `Request::TunnelConnect`), never on the UI thread. While one is in flight (`busy`), every tunnel button is disabled.
- No new `settings::Msg` variant may reach `apply_config_field` (the existing test in `src/ui/settings.rs` that asserts `Ok(false)` for non-config variants gets every new variant added).
- Ansible: root steps use `become: true`; user steps use `become: true` + `become_user: "{{ yutani_user }}"`; `systemctl --user` steps carry `environment: { XDG_RUNTIME_DIR: "/run/user/{{ yutani_uid }}", DBUS_SESSION_BUS_ADDRESS: "unix:path=/run/user/{{ yutani_uid }}/bus" }`. The WireGuard file is copied with `mode: "0600"` and removed in an `always:` block; `no_log: true` on the copy task.
- `cargo test` stays green; `cargo build --release 2>&1 | grep -E '^(warning|error)'` prints nothing; `ansible-playbook --syntax-check` passes; `ansible-lint` (if installed) reports no errors.
- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9
  ```

---

### Task 1: The Tunnel page (`src/ui/tunnel_page.rs`, `src/ui/settings.rs`, `src/ui/mod.rs`, `Cargo.toml`, docs)

**Files:**
- Create: `src/ui/tunnel_page.rs`
- Modify: `src/ui/settings.rs` (`Page::Tunnel`, `PAGES`, `State.tunnel`, `Msg` variants, `view` dispatch, tests)
- Modify: `src/ui/mod.rs` (`mod tunnel_page;`, `Msg::FileDropped`, subscription route, `on_settings` arms, `refresh_tunnel`, `run_tunnel_action`)
- Modify: `Cargo.toml` (libcosmic feature `"xdg-portal"`)
- Modify: `docs/superpowers/specs/2026-09-11-yutani-design.md` §6 page list (add Tunnel after Characters) and `docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md` (a short "Settings page" paragraph pointing at the new spec).

**Interfaces:**
- Consumes: `yutani::tunnel::install::{install, uninstall}` (blocking; `install(conf, dns_servers, dns_domains)` runs `pkexec`), `yutani::tunnel::control::{connect, disconnect, current_tunnel_status}`, `yutani::tunnel::status::TunnelStatus` (`installed`, `connected`, `location`, `failed`, `handshake_age_s`, `exit_address`), `Config.tunnel.{location, dns_servers, dns_domains}`.
- Produces: `tunnel_page::State`, `tunnel_page::view(state, config) -> Element<settings::Msg>`, pure helpers below.

- [ ] **Step 1: Write the failing tests** (in `src/ui/tunnel_page.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_drop_takes_the_first_conf_file_only() {
        let paths = [PathBuf::from("/tmp/readme.txt"), PathBuf::from("/tmp/EVE-UK-455.conf"), PathBuf::from("/tmp/b.conf")];
        assert_eq!(conf_candidate(&paths), Some(PathBuf::from("/tmp/EVE-UK-455.conf")));
        assert_eq!(conf_candidate(&[PathBuf::from("/tmp/readme.txt")]), None);
        assert_eq!(conf_candidate(&[]), None);
    }

    #[test]
    fn install_needs_a_readable_conf_and_no_action_in_flight() {
        let dir = std::env::temp_dir().join(format!("yutani-tunnel-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("x.conf");
        std::fs::write(&conf, "[Interface]\n").unwrap();
        let mut s = State::default();
        assert_eq!(install_blocker(&s), Some(NO_FILE));
        s.conf_path = conf.to_string_lossy().into_owned();
        assert_eq!(install_blocker(&s), None);
        s.busy = true;
        assert_eq!(install_blocker(&s), Some(BUSY));
        s.busy = false;
        s.conf_path = dir.join("missing.conf").to_string_lossy().into_owned();
        assert_eq!(install_blocker(&s), Some(NOT_A_FILE));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_summary_names_the_state_without_secrets() {
        let mut t = TunnelStatus::default();
        assert_eq!(summary(None), "Status unknown (is the daemon running?)");
        assert_eq!(summary(Some(&t)), "Not installed");
        t.installed = true;
        t.location = "London".into();
        assert_eq!(summary(Some(&t)), "Installed (London) · disconnected");
        t.connected = true;
        t.handshake_age_s = Some(12);
        t.exit_address = Some("203.0.113.42".into());
        assert_eq!(summary(Some(&t)), "Installed (London) · connected, exit 203.0.113.42");
        t.connected = false;
        t.failed = true;
        assert_eq!(summary(Some(&t)), "Installed (London) · unit failed");
    }

    #[test]
    fn the_install_note_names_the_file_not_its_contents() {
        assert_eq!(install_note(Path::new("/home/d/Downloads/EVE-UK-455.conf"), Ok(())), "tunnel installed from EVE-UK-455.conf; press Connect");
        assert_eq!(install_note(Path::new("/x/a.conf"), Err("install cancelled or failed (pkexec exit 126)".into())), "install cancelled or failed (pkexec exit 126)");
    }
}
```

`TunnelStatus` must implement `Default` for the summary test; if it does not, add `#[derive(Default)]` to it in `src/tunnel/status.rs` (all fields are `Option`/`bool`/`String`/`u64`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test tunnel_page` → compile errors (the `ui` module lives in the binary, so no `--lib`).

- [ ] **Step 3: Write `src/ui/tunnel_page.rs`**

```rust
//! The settings window's Tunnel page: take the WireGuard `.conf` (browse,
//! type, or drop it on the window) and install / connect / uninstall the
//! EVE-only tunnel (spec
//! `docs/superpowers/specs/2026-09-13-yutani-tunnel-page-and-ansible-design.md`).
//!
//! State and view only; the daemon (`App::run_tunnel_action`) does the work
//! on the blocking pool, because `install` shells out to `pkexec` and
//! `connect`/`disconnect` to `systemctl`.

use std::path::{Path, PathBuf};

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget;

use yutani::model::config::Config;
use yutani::tunnel::status::TunnelStatus;

use super::settings::Msg;

pub const NO_FILE: &str = "Choose the WireGuard configuration file first.";
pub const NOT_A_FILE: &str = "That path is not a readable file.";
pub const BUSY: &str = "Waiting for the previous action to finish…";

#[derive(Debug, Default)]
pub struct State {
    /// The `.conf` path as typed, browsed or dropped.
    pub conf_path: String,
    /// The last status read (`None` before the first refresh).
    pub status: Option<TunnelStatus>,
    /// An install/uninstall/connect/disconnect is in flight.
    pub busy: bool,
    /// Whether the file chooser could be built in (the `xdg-portal` feature).
    pub can_browse: bool,
}

/// The first `.conf` among dropped paths.
pub fn conf_candidate(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("conf"))).cloned()
}

/// Why Install is disabled, or `None`.
pub fn install_blocker(state: &State) -> Option<&'static str> {
    if state.busy {
        return Some(BUSY);
    }
    if state.conf_path.trim().is_empty() {
        return Some(NO_FILE);
    }
    if !Path::new(state.conf_path.trim()).is_file() {
        return Some(NOT_A_FILE);
    }
    None
}

/// One line of state; never a key, never an endpoint.
pub fn summary(status: Option<&TunnelStatus>) -> String {
    let Some(t) = status else { return "Status unknown (is the daemon running?)".to_string() };
    if !t.installed {
        return "Not installed".to_string();
    }
    let state = if t.failed {
        "unit failed".to_string()
    } else if t.connected {
        match &t.exit_address {
            Some(ip) => format!("connected, exit {ip}"),
            None => "connected".to_string(),
        }
    } else {
        "disconnected".to_string()
    };
    format!("Installed ({}) · {state}", t.location)
}

pub fn install_note(conf: &Path, result: Result<(), String>) -> String {
    match result {
        Ok(()) => format!(
            "tunnel installed from {}; press Connect",
            conf.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ),
        Err(e) => e,
    }
}

pub fn view<'a>(state: &'a State, config: &'a Config) -> Element<'a, Msg> {
    let installed = state.status.as_ref().is_some_and(|t| t.installed);
    let connected = state.status.as_ref().is_some_and(|t| t.connected);
    let blocker = install_blocker(state);

    let mut file = widget::settings::section().title("WireGuard configuration").add(widget::text::caption(
        "The wg-quick .conf you downloaded from Proton VPN (WireGuard, one server). Browse for it, type its \
         path, or drop the file anywhere on this window.",
    ));
    let mut row: Vec<Element<'a, Msg>> = vec![
        widget::text_input("/home/you/Downloads/EVE-UK-455.conf", state.conf_path.as_str())
            .on_input(Msg::TunnelConfPath)
            .width(Length::Fill)
            .into(),
    ];
    if state.can_browse {
        row.push(widget::button::standard("Browse…").on_press_maybe((!state.busy).then_some(Msg::BrowseTunnelConf)).into());
    }
    file = file.add(widget::settings::item_row(row));
    file = file.add(widget::settings::item_row(vec![
        widget::button::suggested(if installed { "Replace configuration" } else { "Install tunnel" })
            .on_press_maybe(blocker.is_none().then_some(Msg::InstallTunnel))
            .into(),
    ]));
    if let Some(reason) = blocker {
        file = file.add(widget::text::caption(reason));
    }
    file = file.add(widget::text::caption(
        "Installing asks for your password once (polkit). The file's private key goes to /etc/yutani, root-only; \
         it is never shown here.",
    ));

    let mut tunnel = widget::settings::section().title("Tunnel").add(widget::text::body(summary(state.status.as_ref())));
    if installed {
        tunnel = tunnel.add(widget::settings::item_row(vec![
            widget::button::suggested("Connect").on_press_maybe((!state.busy && !connected).then_some(Msg::TunnelConnect)).into(),
            widget::button::standard("Disconnect").on_press_maybe((!state.busy && connected).then_some(Msg::TunnelDisconnect)).into(),
            widget::button::destructive("Uninstall").on_press_maybe((!state.busy).then_some(Msg::UninstallTunnel)).into(),
            widget::button::text("Refresh").on_press(Msg::RefreshTunnel).into(),
        ]));
    } else {
        tunnel = tunnel.add(widget::settings::item_row(vec![widget::button::text("Refresh").on_press(Msg::RefreshTunnel).into()]));
    }

    let servers: Vec<String> = config.tunnel.dns_servers.iter().map(|s| s.to_string()).collect();
    let dns = widget::settings::section()
        .title("DNS inside the tunnel")
        .add(widget::settings::item("Resolvers", widget::text::body(servers.join(", "))))
        .add(widget::settings::item("Domains", widget::text::body(config.tunnel.dns_domains.join(", "))))
        .add(widget::text::caption(
            "From tunnel.dns_servers / tunnel.dns_domains in config.ron. They are written into the tunnel at install \
             time, so after changing them press Install tunnel again.",
        ));

    widget::settings::view_column(vec![file.into(), tunnel.into(), dns.into()]).into()
}
```

- [ ] **Step 4: Wire `src/ui/settings.rs`**

1. `Page::Tunnel`; `PAGES` becomes six entries: Display, Behavior, Layouts, Characters, Tunnel, Steam; update the page-order test to the new list.
2. `State` gains `pub tunnel: super::tunnel_page::State` (initialised `Default::default()`, with `can_browse: cfg!(feature = "xdg-portal")` — see Step 6 for the feature; if the feature is a libcosmic feature rather than ours, set `can_browse: true` when the file-chooser code compiled, i.e. use a `const CAN_BROWSE: bool` in `mod.rs` next to the dialog code).
3. `Msg` gains (documented):
   ```rust
   TunnelConfPath(String),
   BrowseTunnelConf,
   /// The file chooser answered (`None`: cancelled or unavailable).
   TunnelConfChosen(Option<PathBuf>),
   InstallTunnel,
   UninstallTunnel,
   TunnelConnect,
   TunnelDisconnect,
   RefreshTunnel,
   /// The tunnel's current state, re-read after every action and on open.
   TunnelStatus(Box<TunnelStatus>),
   /// An action finished: which one, and how it went.
   TunnelDone(TunnelAction, Result<(), String>),
   ```
   with `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum TunnelAction { Install, Uninstall, Connect, Disconnect }` in `settings.rs`.
4. `view`: `(Page::Tunnel, _) => super::tunnel_page::view(&state.tunnel, config)` (works with a broken `config.ron`, like Layouts/Characters/Steam — but note it reads `config`; pass the in-memory `Config` the daemon holds, which is the validated one).
5. The `apply_config_field` `Ok(false)` test: add every new variant.

- [ ] **Step 5: Wire `src/ui/mod.rs`**

1. `mod tunnel_page;` in the alphabetical `pub mod` list.
2. `Msg::FileDropped(SurfaceId, Vec<PathBuf>)`; in `subscription`'s `listen_with`, add before the catch-all:
   ```rust
   iced::Event::Window(iced::window::Event::FileDropped(paths)) => Some(Msg::FileDropped(id, paths)),
   ```
   (`FileHovered` stays in the catch-all: it repeats while hovering and must not become a message.)
3. `Msg::FileDropped(id, paths)` in `update`: if `self.settings` is open on `id` and `tunnel_page::conf_candidate(&paths)` is `Some(p)`, set `state.tunnel.conf_path = p.display().to_string()`, activate the Tunnel tab (`state.pages.activate_position(4)` — the index of Tunnel in `PAGES`; write a small `settings::page_index(Page) -> usize` helper with a test instead of hard-coding 4) and set the note `"dropped <file name>; press Install tunnel"`. Otherwise ignore.
4. In `on_settings`, window-state arms: `S::TunnelConfPath(text) => { state.tunnel.conf_path = text.clone(); return Task::none(); }`, `S::TunnelStatus(t) => { state.tunnel.status = Some(*t.clone()); return Task::none(); }`, `S::TunnelDone(action, result)` → `state.tunnel.busy = false`; note: for `Install` → `tunnel_page::install_note(&conf, result)` where `conf` is the current `conf_path`; for `Uninstall` → `"tunnel uninstalled"` / the error; `Connect`/`Disconnect` → `"tunnel connecting…"` / `"tunnel disconnected"` / the error; then `return self.refresh_tunnel()`.
5. Non-config actions:
   - `S::RefreshTunnel` → `return self.refresh_tunnel()`.
   - `S::BrowseTunnelConf` → `return self.browse_tunnel_conf()`.
   - `S::TunnelConfChosen(path)` → set `conf_path` when `Some`, note `"chose <file name>"`; `Task::none()`.
   - `S::InstallTunnel` → re-check `tunnel_page::install_blocker` (note + return when blocked), then `return self.run_tunnel_action(settings::TunnelAction::Install)`.
   - `S::UninstallTunnel | S::TunnelConnect | S::TunnelDisconnect` → `run_tunnel_action` with the matching action.
   - `S::Opened | S::Raise(None) | S::Recheck` and `S::Raise(Some(_))`: batch `self.refresh_tunnel()` with what they already return.
6. New `App` methods:
   ```rust
   /// Read the tunnel state off the UI thread (`is-failed` may spawn systemctl).
   fn refresh_tunnel(&self) -> Task<cosmic::Action<Msg>> {
       let location = self.config.tunnel.location.clone();
       cosmic::iced::Task::perform(
           async move { tokio::task::spawn_blocking(move || crate::tunnel::control::current_tunnel_status(&location)).await.ok() },
           |status| match status {
               Some(t) => cosmic::Action::App(Msg::Settings(settings::Msg::TunnelStatus(Box::new(t)))),
               None => cosmic::Action::None,
           },
       )
   }

   /// One tunnel action on the blocking pool; `busy` until `TunnelDone`.
   fn run_tunnel_action(&mut self, action: settings::TunnelAction) -> Task<cosmic::Action<Msg>> {
       let Some(state) = self.settings.as_mut() else { return Task::none() };
       state.tunnel.busy = true;
       let conf = PathBuf::from(state.tunnel.conf_path.trim());
       let (servers, domains) = (self.config.tunnel.dns_servers.clone(), self.config.tunnel.dns_domains.clone());
       cosmic::iced::Task::perform(
           async move {
               tokio::task::spawn_blocking(move || match action {
                   settings::TunnelAction::Install => crate::tunnel::install::install(&conf, &servers, &domains),
                   settings::TunnelAction::Uninstall => crate::tunnel::install::uninstall(),
                   settings::TunnelAction::Connect => crate::tunnel::control::connect(),
                   settings::TunnelAction::Disconnect => crate::tunnel::control::disconnect(),
               })
               .await
               .map_err(|e| format!("tunnel task failed: {e}"))
               .and_then(|r| r.map_err(|e| format!("{e:#}")))
           },
           move |result| cosmic::Action::App(Msg::Settings(settings::Msg::TunnelDone(action, result))),
       )
   }

   /// The XDG file-chooser portal, filtered to `*.conf`.
   fn browse_tunnel_conf(&self) -> Task<cosmic::Action<Msg>> {
       use cosmic::dialog::file_chooser::{self, FileFilter};
       cosmic::iced::Task::perform(
           async {
               let dialog = file_chooser::open::Dialog::new()
                   .title("WireGuard configuration")
                   .filter(FileFilter::new("WireGuard config").glob("*.conf"));
               dialog.open_file().await.ok().and_then(|r| r.url().to_file_path().ok())
           },
           |path| cosmic::Action::App(Msg::Settings(settings::Msg::TunnelConfChosen(path))),
       )
   }
   ```
   Check the exact `Dialog`/`FileFilter` API in `~/.cargo/git/checkouts/libcosmic-41009aea1d72760b/a401af8/src/dialog/file_chooser/` (`open.rs`: `Dialog::new()`, `.title(..)`, `.filter(..)`, `open_file()`; `FileResponse::url() -> &Url`) and adapt names minimally.
7. Make sure `Request::TunnelConnect`/`TunnelDisconnect` from IPC and the page's buttons cannot fight: they may run concurrently — that is acceptable (systemctl serialises), but the page's `busy` flag only tracks its own action.

- [ ] **Step 6: Cargo feature**

In `Cargo.toml`, add `"xdg-portal",` to the libcosmic `features` list. Build. If it fails at this rev, remove it, remove `browse_tunnel_conf` and the Browse… button (`can_browse: false`), and record the failure output in the report. When it builds, `can_browse` is `true` unconditionally (write it as a `pub const CAN_BROWSE: bool = true;` in `tunnel_page.rs` so the fallback is a one-line change).

- [ ] **Step 7: Docs**

`docs/superpowers/specs/2026-09-11-yutani-design.md` §6: six pages, Tunnel described in one sentence with a pointer to the 2026-09-13 spec; the "apply live / write config.ron" sentence excludes Tunnel too. `docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md`: a short "Settings page" paragraph after the CLI section saying the window's Tunnel page calls the same `install`/`uninstall`/`connect`/`disconnect` functions and that `install` still goes through `pkexec`.

- [ ] **Step 8: Test, build, commit**

`cargo test 2>&1 | grep -E '^test result|FAILED'` all green; `cargo build --release 2>&1 | grep -E '^(warning|error)'` empty.

```bash
git add Cargo.toml Cargo.lock src/ui/tunnel_page.rs src/ui/settings.rs src/ui/mod.rs src/tunnel/status.rs docs/superpowers/specs/2026-09-11-yutani-design.md docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md
git commit -m "feat(settings): Tunnel page — browse/type/drop the WireGuard conf, install, connect, uninstall"
```

---

### Task 2: Ansible role and README (`deploy/ansible/`, `README.md`)

**Files:**
- Create: `README.md` (repo root)
- Create: `deploy/ansible/README.md`, `deploy/ansible/playbook.yml`, `deploy/ansible/inventory.example.ini`, `deploy/ansible/ansible.cfg`
- Create: `deploy/ansible/roles/yutani/defaults/main.yml`, `.../tasks/main.yml`, `.../tasks/packages.yml`, `.../tasks/build.yml`, `.../tasks/install.yml`, `.../tasks/user.yml`, `.../tasks/tunnel.yml`, `.../meta/main.yml`

**Interfaces:**
- Consumes the CLI: `yutani applet install`, `yutani service install`, `yutani shortcuts install`, `yutani tunnel install-root --conf <path> --uid <n> --user <name> --exe <prefix>/yutani [--dns-servers a,b] [--dns-domains x,y]` (root; hidden subcommand; read `src/main.rs` `TunnelAction::InstallRoot` for the exact flags and `src/tunnel/install.rs` `install_root` for its report text and idempotency).
- Produces: nothing consumed by code.

- [ ] **Step 1: `defaults/main.yml`**

```yaml
---
# The desktop user Yutani runs as (required).
yutani_user: ""
yutani_repo: "https://github.com/dc9090-web/yutani.git"
yutani_version: "master"
yutani_src_dir: "/home/{{ yutani_user }}/.local/src/yutani"
yutani_prefix: "/usr/local/bin"
yutani_manage_packages: true
yutani_install_shortcuts: true
yutani_install_service: true
yutani_enable_service_at_login: false
# Local (controller-side) path to a wg-quick WireGuard .conf; empty = no tunnel.
yutani_wg_conf: ""
# Empty lists = the daemon's built-in defaults (1.1.1.1, 9.9.9.9 / eveonline.com …).
yutani_dns_servers: []
yutani_dns_domains: []
yutani_packages:
  pacman:
    - nftables
    - wireguard-tools
    - curl
    - polkit
    - gtk-update-icon-cache
    - rustup
    - git
    - base-devel
```

- [ ] **Step 2: tasks**

`tasks/main.yml`:
```yaml
---
- name: Require a user
  ansible.builtin.assert:
    that: yutani_user | length > 0
    fail_msg: "set yutani_user to the desktop user"

- name: Look the user up
  ansible.builtin.getent:
    database: passwd
    key: "{{ yutani_user }}"

- name: Remember the uid
  ansible.builtin.set_fact:
    yutani_uid: "{{ getent_passwd[yutani_user][1] }}"

- ansible.builtin.import_tasks: packages.yml
  when: yutani_manage_packages
- ansible.builtin.import_tasks: build.yml
- ansible.builtin.import_tasks: install.yml
- ansible.builtin.import_tasks: user.yml
- ansible.builtin.import_tasks: tunnel.yml
  when: yutani_wg_conf | length > 0
```

`tasks/packages.yml`:
```yaml
---
- name: Only pacman is mapped so far
  ansible.builtin.assert:
    that: ansible_facts.pkg_mgr in yutani_packages
    fail_msg: "no package list for {{ ansible_facts.pkg_mgr }}; add one to yutani_packages or set yutani_manage_packages: false"

- name: Install build and runtime packages
  become: true
  ansible.builtin.package:
    name: "{{ yutani_packages[ansible_facts.pkg_mgr] }}"
    state: present

- name: Check for a Rust toolchain
  become: true
  become_user: "{{ yutani_user }}"
  ansible.builtin.command: rustup show active-toolchain
  register: yutani_toolchain
  changed_when: false
  failed_when: false

- name: Select stable Rust
  become: true
  become_user: "{{ yutani_user }}"
  ansible.builtin.command: rustup default stable
  when: yutani_toolchain.rc != 0
```

`tasks/build.yml`:
```yaml
---
- name: Clone or update the source
  become: true
  become_user: "{{ yutani_user }}"
  ansible.builtin.git:
    repo: "{{ yutani_repo }}"
    dest: "{{ yutani_src_dir }}"
    version: "{{ yutani_version }}"
  register: yutani_git

- name: Build the release binaries
  become: true
  become_user: "{{ yutani_user }}"
  ansible.builtin.command:
    cmd: cargo build --release --locked
    chdir: "{{ yutani_src_dir }}"
  environment:
    PATH: "/home/{{ yutani_user }}/.cargo/bin:{{ ansible_env.PATH }}"
  register: yutani_build
  changed_when: "'Compiling' in yutani_build.stderr"
```

`tasks/install.yml`:
```yaml
---
- name: Install the binaries (root-owned — the tunnel unit refuses anything else)
  become: true
  ansible.builtin.copy:
    src: "{{ yutani_src_dir }}/target/release/{{ item }}"
    dest: "{{ yutani_prefix }}/{{ item }}"
    remote_src: true
    owner: root
    group: root
    mode: "0755"
  loop:
    - yutani
    - yutani-applet
```

`tasks/user.yml` (every task `become: true`, `become_user: "{{ yutani_user }}"`, and the `environment:` block from the Global Constraints):
```yaml
---
- name: Applet, icons and the Applications launcher
  ansible.builtin.command: "{{ yutani_prefix }}/yutani applet install"
  register: yutani_applet
  changed_when: false

- name: Systemd user unit (restart on crash)
  ansible.builtin.command: "{{ yutani_prefix }}/yutani service install"
  when: yutani_install_service
  changed_when: false

- name: Start Yutani at login
  ansible.builtin.systemd:
    scope: user
    name: yutani.service
    enabled: true
  when: yutani_install_service and yutani_enable_service_at_login

- name: COSMIC keyboard shortcuts
  ansible.builtin.command: "{{ yutani_prefix }}/yutani shortcuts install"
  when: yutani_install_shortcuts
  changed_when: false
```
(`changed_when: false` because the subcommands always rewrite the same files; if `install_root`/`applet install` print a distinguishable "unchanged" line, use it instead and say so in the report.)

`tasks/tunnel.yml`:
```yaml
---
- name: Install the EVE-only tunnel
  block:
    - name: Copy the WireGuard configuration (root-only, temporary)
      become: true
      ansible.builtin.copy:
        src: "{{ yutani_wg_conf }}"
        dest: /root/yutani-wg.conf
        owner: root
        group: root
        mode: "0600"
      no_log: true

    - name: Write the tunnel unit, conf and polkit rule
      become: true
      ansible.builtin.command:
        argv: >-
          {{ [yutani_prefix ~ '/yutani', 'tunnel', 'install-root',
              '--conf', '/root/yutani-wg.conf',
              '--uid', yutani_uid | string,
              '--user', yutani_user,
              '--exe', yutani_prefix ~ '/yutani']
             + (['--dns-servers', yutani_dns_servers | join(',')] if yutani_dns_servers | length > 0 else [])
             + (['--dns-domains', yutani_dns_domains | join(',')] if yutani_dns_domains | length > 0 else []) }}
      register: yutani_tunnel
      changed_when: true
  always:
    - name: Remove the temporary configuration
      become: true
      ansible.builtin.file:
        path: /root/yutani-wg.conf
        state: absent
```

`playbook.yml`:
```yaml
---
- name: Deploy Yutani
  hosts: yutani_hosts
  roles:
    - yutani
```

`inventory.example.ini`:
```ini
[yutani_hosts]
localhost ansible_connection=local

[yutani_hosts:vars]
yutani_user=daniel
```

`ansible.cfg`: `[defaults] inventory = inventory.example.ini`, `roles_path = roles`, `interpreter_python = auto_silent`.

`meta/main.yml`: `galaxy_info` with a description, `min_ansible_version: "2.15"`, platforms ArchLinux; `dependencies: []`.

- [ ] **Step 3: READMEs**

`deploy/ansible/README.md`: what the role does (the five steps from the spec), the variables table, `ansible-playbook -K playbook.yml` for localhost, `-e yutani_wg_conf=~/Downloads/EVE-UK-455.conf` for the tunnel, the note that the tunnel file never leaves root and is deleted after install, and that re-running is safe.

Root `README.md`: one paragraph on what Yutani is (live thumbnails, hotkeys, panel applet, EVE-only WireGuard tunnel, settings window), requirements (COSMIC ≥ 1.8, Arch/CachyOS packages), manual install (the six commands: build, `sudo install` both binaries, `applet install`, `service install`, `shortcuts install`, Steam launch options `PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 yutani launch -- %command%`), the tunnel (settings window Tunnel page or `yutani tunnel install <conf>`), and a pointer to `deploy/ansible/`. Keep it under 120 lines.

- [ ] **Step 4: Verify**

```bash
cd deploy/ansible && ansible-playbook --syntax-check playbook.yml
which ansible-lint && ansible-lint playbook.yml roles/ || echo "ansible-lint not installed"
ansible-playbook --check --diff playbook.yml -e yutani_user=daniel -e yutani_manage_packages=false 2>&1 | tail -20
```
(the check run will skip `command` tasks; that is expected — report what it did). Do NOT run the playbook for real: it needs the become password, and the controller will run it.

- [ ] **Step 5: Commit**

```bash
git add README.md deploy/
git commit -m "deploy: Ansible role that builds, installs and integrates Yutani (applet, service, shortcuts, optional tunnel); README"
```
