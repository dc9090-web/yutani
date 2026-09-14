# Steam Launch-Options Check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Yutani notices when Steam's launch options for EVE Online name a `yutani` that no longer exists (or no `yutani launch` at all) and says so on the panel icon, in the applet popover, and on the settings window's Steam page, before the user clicks Play.

**Architecture:** A new pure library module `src/steam.rs` scans each Steam account's `localconfig.vdf` for EVE's (app 8500) `LaunchOptions` and judges it. The daemon runs that scan on a 30 s subscription, keeps the non-`Ok` findings on `App`, and copies them into the IPC `status` reply (new `steam` field, `serde(default)`). The applet turns a non-empty `steam` list into the warning dot and a popover notice; the settings window's Steam page shows the same sentence as a banner and re-scans when opened.

**Tech Stack:** Rust, libcosmic/iced (`Subscription`, `Task`), serde/serde_json, tokio `spawn_blocking`, futures-timer. Spec: `docs/superpowers/specs/2026-09-14-steam-launch-check-design.md`.

## Global Constraints

- Yutani never writes any Steam file. Every filesystem failure in the scan is a skipped account, never an error.
- Never run `cargo fmt` in this repo (it is not rustfmt-clean; a plain fmt touches 55 files).
- The bin crate reaches the library as `yutani::…` (main.rs does `use yutani::{ipc, model, tunnel};`); inside `src/` library files use `crate::…`.
- New `Status` fields carry `#[serde(default)]` so an older applet or daemon still parses.
- The popover sentences, verbatim:
  - Broken: `Steam launches EVE through <path>, which is missing. Open Settings → Steam.`
  - NoWrapper: `Steam launches EVE without yutani, so it runs outside the tunnel. Open Settings → Steam.`
  The settings banner is the same sentence without ` Open Settings → Steam.`
- Test command for one module: `cargo test --lib <module>::` ; the whole suite: `cargo test`. Both must stay green after every task. `cargo clippy --all-targets` must stay clean.
- Commit after each task with the attribution lines from the session's system reminder.

## Deviations from the spec (decided while planning, keep)

- `launch_options` returns `Some(String::new())` for an EVE block that has no `LaunchOptions` key (Steam stores no key for an empty field), and `None` only when there is no EVE block. So an account that played EVE without ever setting launch options is judged `NoWrapper`, which is the honest verdict.
- `Finding` serialises as `{"verdict":{"problem":"broken","path":"…"},"file":"…"}` rather than flattening the verdict into the finding. Flattening an internally tagged enum is a serde corner best avoided.
- The subscription re-reads the files every 30 s (the file is 87 KB) and emits only when the finding list changed, instead of memoising mtimes. Same cost, less state.

---

## File structure

- **Create** `src/steam.rs`: everything about Steam's launch options. Pure scanner (`launch_options`, `quoted`), verdict (`wrapper_path`, `judge`, `Verdict`), filesystem scan (`steam_roots`, `scan`, `resolves`, `problems`), the shared warning sentence (`first_message`), and the daemon subscription (`subscription`).
- **Modify** `src/lib.rs`: register `pub mod steam;`, fix `STEAM_LAUNCH_ARGS_ABSOLUTE`.
- **Modify** `src/tunnel/status.rs`: `Status.steam`.
- **Modify** `src/ui/mod.rs`: `App.steam_findings`, `Msg::SteamChecked`, subscription batch, status assembly, settings hooks, `recheck_steam`.
- **Modify** `src/applet/icon.rs`: `icon_state(…, steam_problem)`.
- **Modify** `src/applet/display.rs`: `Popover.notice`, `popover_height(…, notice)`, `graph_fits(…, notice)`.
- **Modify** `src/applet/theme.rs`: `warning` colour.
- **Modify** `src/bin/yutani-applet/view.rs`: `Role::Warning`, notice row, icon call.
- **Modify** `src/applet/client.rs`, `src/tunnel/status.rs` tests: `steam: Vec::new()` in `Status` literals.
- **Modify** `src/ui/settings.rs`: `State.steam_findings`, banner on the Steam page.
- **Modify** `src/ui/settings_ui.rs`: `Tint::Warning`, `warning_note`.
- **Modify** `README.md`, `docs/superpowers/specs/2026-09-14-yutani-package-design.md`.

---

### Task 1: The scanner and the verdict (`src/steam.rs`, pure)

**Files:**
- Create: `src/steam.rs`
- Modify: `src/lib.rs:7-16` (module list)

**Interfaces:**
- Produces:
  - `pub fn launch_options(localconfig: &str) -> Option<String>`
  - `pub fn wrapper_path(launch_options: &str) -> Option<&str>`
  - `pub enum Verdict { Ok, Broken { path: String }, NoWrapper }` with `pub fn message(&self) -> Option<String>`
  - `pub fn judge(launch_options: &str, resolves: &dyn Fn(&str) -> bool) -> Verdict`

- [ ] **Step 1: Write the failing tests**

Create `src/steam.rs` with only the module doc and the test module:

```rust
//! Steam's launch options for EVE Online (app 8500): reading them from
//! each account's `localconfig.vdf`, judging whether the `yutani launch`
//! wrapper they name still exists, and carrying the verdict to the panel
//! applet and the settings window. Yutani only ever *reads* Steam's files.
//! See docs/superpowers/specs/2026-09-14-steam-launch-check-design.md.

#[cfg(test)]
mod tests {
    use super::*;

    /// Cut from a real `localconfig.vdf`: the `apptickets` decoy (a
    /// one-line `"8500"` pair), then the real block under `apps`, then a
    /// neighbour.
    const REAL: &str = "\"UserLocalConfigStore\"\n{\n\t\"apptickets\"\n\t{\n\t\t\"8500\"\t\t\"1\"\n\t}\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"8500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1789355329\"\n\t\t\t\t\t\t\"cloud\"\n\t\t\t\t\t\t{\n\t\t\t\t\t\t\t\"last_sync_state\"\t\t\"synchronized\"\n\t\t\t\t\t\t}\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n\t\t\t\t\t}\n\t\t\t\t\t\"8870\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn the_eve_launch_line_is_read_past_the_apptickets_decoy() {
        assert_eq!(
            launch_options(REAL).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%")
        );
    }

    #[test]
    fn a_neighbouring_app_is_never_mistaken_for_eve() {
        let only_neighbour = REAL.replace("\"8500\"\n", "\"8501\"\n");
        assert_eq!(launch_options(&only_neighbour), None);
    }

    #[test]
    fn an_eve_block_without_launch_options_is_an_empty_line() {
        let stripped = REAL.replace(
            "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n",
            "",
        );
        assert_eq!(launch_options(&stripped).as_deref(), Some(""));
    }

    #[test]
    fn a_nested_launch_options_key_does_not_count() {
        // Only a direct child of the EVE block is EVE's launch line.
        let nested = REAL.replace("\"last_sync_state\"\t\t\"synchronized\"", "\"LaunchOptions\"\t\t\"nested\"").replace(
            "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n",
            "",
        );
        assert_eq!(launch_options(&nested).as_deref(), Some(""));
    }

    #[test]
    fn vdf_escapes_are_undone() {
        let escaped = REAL.replace("/usr/local/bin/yutani launch", "\"/opt/y\\\\utani\" launch");
        // `\"` → `"`, `\\` → `\`.
        assert_eq!(
            launch_options(&escaped).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 \"/opt/y\\utani\" launch -- %command%")
        );
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(launch_options(""), None);
        assert_eq!(launch_options("\"8500\"\n{\n\"LaunchOptions\""), None);
        assert_eq!(launch_options("\"8500\"\n\"LaunchOptions\"\t\"x\"\n"), None);
    }

    #[test]
    fn the_wrapper_is_the_token_before_launch() {
        assert_eq!(wrapper_path("A=1 yutani launch -- %command%"), Some("yutani"));
        assert_eq!(wrapper_path("A=1 /usr/bin/yutani launch -- %command%"), Some("/usr/bin/yutani"));
        assert_eq!(wrapper_path("/home/d/Yutani/target/release/yutani launch -- %command%"), Some("/home/d/Yutani/target/release/yutani"));
        assert_eq!(wrapper_path("A=1 %command%"), None);
        assert_eq!(wrapper_path(""), None);
        // `launch` alone, or a different wrapper, is not ours.
        assert_eq!(wrapper_path("gamemoderun launch -- %command%"), None);
        assert_eq!(wrapper_path("yutani launch %command%"), None);
    }

    #[test]
    fn every_verdict() {
        let exists = |p: &str| p == "/usr/bin/yutani" || p == "yutani";
        assert_eq!(judge("A=1 yutani launch -- %command%", &exists), Verdict::Ok);
        assert_eq!(judge("A=1 /usr/bin/yutani launch -- %command%", &exists), Verdict::Ok);
        assert_eq!(
            judge("A=1 /usr/local/bin/yutani launch -- %command%", &exists),
            Verdict::Broken { path: "/usr/local/bin/yutani".into() }
        );
        assert_eq!(judge("A=1 %command%", &exists), Verdict::NoWrapper);
        assert_eq!(judge("", &exists), Verdict::NoWrapper);
    }

    #[test]
    fn the_messages_name_the_path_and_ok_says_nothing() {
        assert_eq!(Verdict::Ok.message(), None);
        assert_eq!(
            Verdict::Broken { path: "/usr/local/bin/yutani".into() }.message().as_deref(),
            Some("Steam launches EVE through /usr/local/bin/yutani, which is missing.")
        );
        assert_eq!(
            Verdict::NoWrapper.message().as_deref(),
            Some("Steam launches EVE without yutani, so it runs outside the tunnel.")
        );
    }

    #[test]
    fn a_verdict_serialises_with_a_problem_tag() {
        let json = serde_json::to_string(&Verdict::Broken { path: "/x/yutani".into() }).unwrap();
        assert_eq!(json, r#"{"problem":"broken","path":"/x/yutani"}"#);
        assert_eq!(serde_json::to_string(&Verdict::NoWrapper).unwrap(), r#"{"problem":"no_wrapper"}"#);
        assert_eq!(serde_json::from_str::<Verdict>(r#"{"problem":"ok"}"#).unwrap(), Verdict::Ok);
    }
}
```

Add to `src/lib.rs` after `pub mod service;`:

```rust
/// Steam's launch options for EVE: read, judged, never written.
pub mod steam;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib steam::`
Expected: compile errors, `cannot find function launch_options`, `cannot find type Verdict`.

- [ ] **Step 3: Implement the scanner and the verdict**

Insert above the `#[cfg(test)]` module in `src/steam.rs`:

```rust
use serde::{Deserialize, Serialize};

/// EVE Online's Steam app id.
pub const EVE_APP_ID: &str = "8500";

/// EVE's `LaunchOptions` from one account's `localconfig.vdf`.
///
/// The file is Valve's KeyValues text, nested
/// `UserLocalConfigStore → Software → Valve → Steam → apps → "8500" →
/// "LaunchOptions"`. A line walker is enough: the EVE block is the first
/// line that is exactly `"8500"` and is followed by `{` (the `apptickets`
/// section also has an `"8500"` key, but as a one-line pair, so it is
/// skipped), and the launch line is the `LaunchOptions` pair that is a
/// direct child of that block.
///
/// `None`: no EVE block at all. `Some("")`: a block without the key —
/// Steam stores no key for an empty field, so that is what "never set"
/// looks like.
pub fn launch_options(localconfig: &str) -> Option<String> {
    let key = format!("\"{EVE_APP_ID}\"");
    let mut lines = localconfig.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        if line != key {
            continue;
        }
        while lines.peek().is_some_and(|l| l.is_empty()) {
            lines.next();
        }
        if lines.peek() != Some(&"{") {
            continue;
        }
        lines.next();
        let mut depth = 1usize;
        for line in lines.by_ref() {
            match line {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(String::new());
                    }
                }
                _ => {
                    if depth == 1
                        && let Some(value) = quoted_pair(line, "LaunchOptions")
                    {
                        return Some(value);
                    }
                }
            }
        }
        // The block never closed: a truncated file. Nothing to judge.
        return None;
    }
    None
}

/// `"Key"   "value"` → the value, when the key is `key`.
fn quoted_pair(line: &str, key: &str) -> Option<String> {
    let (k, rest) = quoted(line)?;
    if k != key {
        return None;
    }
    let (v, _) = quoted(rest.trim_start())?;
    Some(v)
}

/// The quoted string at the start of `s`, with VDF's `\"`, `\\`, `\n`
/// and `\t` undone, and whatever follows the closing quote.
fn quoted(s: &str) -> Option<(String, &str)> {
    let body = s.strip_prefix('"')?;
    let mut chars = body.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                let (_, e) = chars.next()?;
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
            }
            '"' => return Some((out, &body[i + 1..])),
            c => out.push(c),
        }
    }
    None
}

/// The `yutani` the launch line runs the game through: the token right
/// before `launch --`, when it is `yutani` or a path ending in `/yutani`.
/// Tokens are split on whitespace; a quoted path with spaces is not
/// supported, and the README never suggests one.
pub fn wrapper_path(launch_options: &str) -> Option<&str> {
    let tokens: Vec<&str> = launch_options.split_whitespace().collect();
    tokens
        .windows(3)
        .find(|w| w[1] == "launch" && w[2] == "--" && (w[0] == "yutani" || w[0].ends_with("/yutani")))
        .map(|w| w[0])
}

/// What the launch line means for the next press of Play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum Verdict {
    /// `yutani launch --` is there and the binary it names resolves.
    Ok,
    /// The line names a yutani that does not resolve: an absolute path
    /// that does not exist, or a bare `yutani` not on PATH. Play fails in
    /// `/bin/sh` before Proton runs, with nothing on screen.
    Broken { path: String },
    /// No `yutani launch --` at all: EVE starts outside the tunnel.
    NoWrapper,
}

impl Verdict {
    /// The sentence the popover and the settings banner share; `None`
    /// when there is nothing to say.
    pub fn message(&self) -> Option<String> {
        match self {
            Verdict::Ok => None,
            Verdict::Broken { path } => Some(format!("Steam launches EVE through {path}, which is missing.")),
            Verdict::NoWrapper => Some("Steam launches EVE without yutani, so it runs outside the tunnel.".to_string()),
        }
    }
}

/// Judge one launch line. `resolves` says whether a wrapper token names a
/// real binary; it is injected so every verdict is testable without a
/// filesystem (the production one is [`resolves`]).
pub fn judge(launch_options: &str, resolves: &dyn Fn(&str) -> bool) -> Verdict {
    match wrapper_path(launch_options) {
        None => Verdict::NoWrapper,
        Some(p) if resolves(p) => Verdict::Ok,
        Some(p) => Verdict::Broken { path: p.to_string() },
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib steam::`
Expected: `test result: ok. 10 passed`.

Run: `cargo clippy --all-targets`
Expected: no warnings from `src/steam.rs`. (If clippy objects to `let` chains on this toolchain, rewrite the `_ =>` arm as `if depth == 1 { if let Some(value) = … { return Some(value); } }`.)

- [ ] **Step 5: Commit**

```bash
git add src/steam.rs src/lib.rs
git commit -m "feat(steam): read and judge EVE's Steam launch line

launch_options walks localconfig.vdf to EVE's block, wrapper_path finds
the yutani it runs the game through, and judge says whether that yutani
still exists. Pure; the filesystem scan comes next.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_015SKJc3TpQiPf631xfgAYyF"
```

---

### Task 2: Scanning every Steam account (`src/steam.rs`, filesystem)

**Files:**
- Modify: `src/steam.rs`

**Interfaces:**
- Consumes: `launch_options`, `judge`, `Verdict` from Task 1.
- Produces:
  - `pub struct Finding { pub verdict: Verdict, pub file: PathBuf }` (Serialize/Deserialize/Clone/Debug/PartialEq/Eq)
  - `pub fn steam_roots(home: &Path) -> [PathBuf; 3]`
  - `pub fn scan(home: &Path, resolves: &dyn Fn(&str) -> bool) -> Vec<Finding>`
  - `pub fn resolves(name: &str) -> bool`
  - `pub fn problems() -> Vec<Finding>` (only non-`Ok` findings, real home, real PATH)
  - `pub fn first_message(findings: &[Finding]) -> Option<String>`

- [ ] **Step 1: Write the failing tests**

Append inside the `tests` module of `src/steam.rs`:

```rust
    fn sandbox(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-steam-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_account(root: &std::path::Path, id: &str, launch: Option<&str>) -> std::path::PathBuf {
        let dir = root.join("userdata").join(id).join("config");
        std::fs::create_dir_all(&dir).unwrap();
        let text = match launch {
            Some(line) => REAL.replace("/usr/local/bin/yutani launch -- %command%", line).replace(
                "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 ",
                "",
            ),
            None => REAL.replace("\"8500\"\n", "\"8501\"\n"),
        };
        let file = dir.join("localconfig.vdf");
        std::fs::write(&file, text).unwrap();
        file
    }

    #[test]
    fn the_roots_cover_native_legacy_and_flatpak_steam() {
        let home = std::path::Path::new("/home/x");
        assert_eq!(
            steam_roots(home),
            [
                std::path::PathBuf::from("/home/x/.local/share/Steam"),
                std::path::PathBuf::from("/home/x/.steam/steam"),
                std::path::PathBuf::from("/home/x/.var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ]
        );
    }

    #[test]
    fn scan_reports_one_finding_per_account_with_an_eve_block() {
        let home = sandbox("scan");
        let native = home.join(".local/share/Steam");
        let fine = write_account(&native, "1001", Some("/usr/bin/yutani launch -- %command%"));
        let broken = write_account(&native, "1002", Some("/usr/local/bin/yutani launch -- %command%"));
        let _no_eve = write_account(&native, "1003", None);
        // A non-numeric entry and an account with no config are skipped.
        std::fs::create_dir_all(native.join("userdata/anonymous")).unwrap();
        std::fs::create_dir_all(native.join("userdata/1004")).unwrap();
        // The legacy tree is a symlink to the native one: not a second copy.
        std::fs::create_dir_all(home.join(".steam")).unwrap();
        std::os::unix::fs::symlink(&native, home.join(".steam/steam")).unwrap();

        let exists = |p: &str| p == "/usr/bin/yutani";
        let found = scan(&home, &exists);
        // `scan` walks the canonical root, so compare canonical paths: the
        // temp dir itself may sit behind a symlink.
        let seen: Vec<(Verdict, std::path::PathBuf)> =
            found.iter().map(|f| (f.verdict.clone(), f.file.canonicalize().unwrap())).collect();
        assert_eq!(
            seen,
            vec![
                (Verdict::Ok, fine.canonicalize().unwrap()),
                (Verdict::Broken { path: "/usr/local/bin/yutani".into() }, broken.canonicalize().unwrap()),
            ]
        );
        assert_eq!(
            first_message(&found).as_deref(),
            Some("Steam launches EVE through /usr/local/bin/yutani, which is missing.")
        );
        assert_eq!(first_message(&found[..1]), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn no_steam_at_all_is_nothing_to_report() {
        let home = sandbox("nosteam");
        assert!(scan(&home, &|_| true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn resolves_checks_paths_on_disk_and_bare_names_on_path() {
        assert!(resolves("/bin/sh"));
        assert!(!resolves("/nonexistent/yutani"));
        assert!(resolves("sh"));
        assert!(!resolves("yutani-surely-not-installed-under-this-name"));
    }

    #[test]
    fn a_finding_round_trips_as_json() {
        let f = Finding { verdict: Verdict::NoWrapper, file: "/h/localconfig.vdf".into() };
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, r#"{"verdict":{"problem":"no_wrapper"},"file":"/h/localconfig.vdf"}"#);
        assert_eq!(serde_json::from_str::<Finding>(&json).unwrap(), f);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib steam::`
Expected: compile errors, `cannot find function steam_roots`, `cannot find struct Finding`.

- [ ] **Step 3: Implement the scan**

Insert after `judge` in `src/steam.rs`:

```rust
use std::path::{Path, PathBuf};

/// One account's verdict and the file it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub verdict: Verdict,
    /// The `localconfig.vdf` the launch line was read from.
    pub file: PathBuf,
}

/// Where Steam keeps `userdata/`: native (the XDG path and the legacy
/// `~/.steam` symlink tree) and the Flatpak. Mirrors
/// `eve_settings::steam_vdf_candidates`, one level up.
pub fn steam_roots(home: &Path) -> [PathBuf; 3] {
    [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
}

/// Every account's verdict, in root then account-id order. A root that
/// canonicalises to one already seen (the legacy symlink) is skipped, so
/// no account is reported twice; an unreadable file or an account with no
/// EVE block is silently skipped — a missing Steam is not Yutani's problem
/// to report.
pub fn scan(home: &Path, resolves: &dyn Fn(&str) -> bool) -> Vec<Finding> {
    let mut seen_roots: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for root in steam_roots(home) {
        let Ok(root) = root.canonicalize() else { continue };
        if seen_roots.contains(&root) {
            continue;
        }
        seen_roots.push(root.clone());
        let Ok(entries) = std::fs::read_dir(root.join("userdata")) else { continue };
        let mut accounts: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        accounts.sort();
        for account in accounts {
            let file = account.join("config").join("localconfig.vdf");
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let Some(line) = launch_options(&text) else { continue };
            out.push(Finding { verdict: judge(&line, resolves), file });
        }
    }
    out
}

/// The production resolver: a token with a `/` is a path that must be a
/// file; a bare name is searched on this process's `PATH`.
pub fn resolves(name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).is_file();
    }
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// What the daemon reports: only the accounts whose launch line is not
/// [`Verdict::Ok`], from the real home and the real `PATH`.
pub fn problems() -> Vec<Finding> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    scan(&home, &resolves).into_iter().filter(|f| f.verdict != Verdict::Ok).collect()
}

/// The first thing worth saying about `findings`, for the popover and the
/// settings banner.
pub fn first_message(findings: &[Finding]) -> Option<String> {
    findings.iter().find_map(|f| f.verdict.message())
}
```

Move the `use std::path::{Path, PathBuf};` line up to sit with `use serde::…` at the top of the file.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib steam::`
Expected: `test result: ok. 15 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/steam.rs
git commit -m "feat(steam): scan every Steam account's launch line

steam_roots/scan read userdata/<id>/config/localconfig.vdf under the
native, legacy and Flatpak roots (deduplicated through canonicalize),
problems() keeps the non-Ok findings, first_message is the sentence the
applet and settings share.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_015SKJc3TpQiPf631xfgAYyF"
```

---

### Task 3: The daemon carries the findings in `status`

**Files:**
- Modify: `src/tunnel/status.rs:116-129` (`Status`), tests at `:335-350`
- Modify: `src/applet/client.rs:190-200` (`sample_status` test literal)
- Modify: `src/applet/display.rs:258-280` (`status` test helper literal)
- Modify: `src/steam.rs` (subscription)
- Modify: `src/ui/mod.rs:108-165` (`App`), `:166-200` (`Msg`), `:540-580` (status assembly), `:2222-2245` and `:2518-2536` (App literals), `:2346-2354` (handlers), `:2356-2390` (subscription)

**Interfaces:**
- Consumes: `steam::Finding`, `steam::problems`.
- Produces:
  - `Status.steam: Vec<steam::Finding>` (`serde(default)`)
  - `pub fn steam::subscription() -> Subscription<Vec<Finding>>`
  - `App.steam_findings: Vec<yutani::steam::Finding>`, `Msg::SteamChecked(Vec<yutani::steam::Finding>)`

- [ ] **Step 1: Write the failing tests**

In `src/tunnel/status.rs` tests, extend `a_status_json_without_the_new_fields_still_parses` by adding after the last assert:

```rust
        assert!(back.steam.is_empty(), "an older daemon reports no Steam findings");
```

and in `status_json_round_trips` change the literal and the asserts:

```rust
        let st = Status {
            clients: vec![ClientStatus {
                name: "KestrelVance".into(),
                active: true,
            }],
            hidden: false,
            shortcuts: None,
            outputs: Vec::new(),
            steam: vec![crate::steam::Finding {
                verdict: crate::steam::Verdict::Broken { path: "/usr/local/bin/yutani".into() },
                file: "/h/localconfig.vdf".into(),
            }],
            tunnel: assemble(Some(&file()), true, Some((1, 2)), true, false, "London", 1021),
        };
        let json = serde_json::to_string(&st).unwrap();
        let back: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clients[0].name, "KestrelVance");
        assert_eq!(back.tunnel.handshake_age_s, Some(21));
        assert_eq!(back.steam, st.steam);
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tunnel::status::`
Expected: `no field steam on type Status`.

- [ ] **Step 3: Add the field and fix every literal**

In `src/tunnel/status.rs`, after the `outputs` field of `Status`:

```rust
    /// Steam accounts whose EVE launch line is not `steam::Verdict::Ok`:
    /// a `yutani` path that no longer exists, or no `yutani launch` at
    /// all. Empty when every account is fine or Steam was not found, and
    /// (`serde(default)`) from a daemon older than this field.
    #[serde(default)]
    pub steam: Vec<crate::steam::Finding>,
```

Add `steam: Vec::new(),` to the `Status` literals in `src/applet/client.rs` (`sample_status`, ~line 191) and `src/applet/display.rs` (`status`, ~line 259).

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: all green.

- [ ] **Step 5: The subscription**

Append to `src/steam.rs` (above the tests):

```rust
use cosmic::iced::{self, Subscription};
use std::time::Duration;

/// How often the daemon re-reads Steam's files. Steam rewrites
/// `localconfig.vdf` when Properties closes and when it exits, so a fix
/// made in Steam's UI shows up within this.
pub const PERIOD: Duration = Duration::from_secs(30);

/// The daemon's watch: [`problems`] at startup and every [`PERIOD`],
/// emitted only when the list changed. The scan reads a few small files
/// on the blocking pool; the subscription's future shares the one-worker
/// UI runtime and must not block it.
pub fn subscription() -> Subscription<Vec<Finding>> {
    Subscription::run(run)
}

fn run() -> impl iced::futures::Stream<Item = Vec<Finding>> {
    iced::futures::stream::unfold(None::<Vec<Finding>>, |mut last| async move {
        loop {
            if last.is_some() {
                futures_timer::Delay::new(PERIOD).await;
            }
            let now = tokio::task::spawn_blocking(problems).await.unwrap_or_default();
            if last.as_ref() != Some(&now) {
                return Some((now.clone(), Some(now)));
            }
            last = Some(now);
        }
    })
}
```

`cosmic` is already a dependency of the library (`libcosmic` is used by `src/applet/theme.rs`); `futures-timer` and `tokio` are in `Cargo.toml`.

- [ ] **Step 6: The daemon keeps and reports the findings**

In `src/ui/mod.rs`:

1. `App` (after `tunnel_in_flight`):

```rust
    /// Steam accounts whose EVE launch line is broken or lacks `yutani
    /// launch` (`yutani::steam`), from the 30 s subscription; copied into
    /// every `status` reply and into the settings window when it is open.
    pub steam_findings: Vec<yutani::steam::Finding>,
```

2. `Msg` (after `Adopt`):

```rust
    /// The Steam launch-line scan finished (the periodic subscription, or a
    /// re-check when the settings window opens or shows its Steam page).
    SteamChecked(Vec<yutani::steam::Finding>),
```

3. Both `App` literals (the one in `init` near line 2224 and the test `app()` near line 2519): add `steam_findings: Vec::new(),`.

4. Subscription batch: after `ipc::subscription().map(Msg::Ipc),` add

```rust
            yutani::steam::subscription().map(Msg::SteamChecked),
```

5. Status assembly (`Request::Status` arm): after `let outputs … .collect();` add

```rust
                let steam = self.steam_findings.clone();
```

and change the struct literal inside the async block to

```rust
                        let status = crate::tunnel::status::Status { clients, hidden, tunnel, shortcuts, outputs, steam };
```

6. Handler, next to `Msg::Adopt`:

```rust
            Msg::SteamChecked(findings) => {
                // The subscription only emits on change, so this is one
                // warning per change, not one per tick.
                for f in &findings {
                    if let Some(m) = f.verdict.message() {
                        tracing::warn!(file = %f.file.display(), "{m}");
                    }
                }
                self.steam_findings = findings;
                Task::none()
            }
```

(The settings window's copy of the findings is added in Task 5, Step 3.)

- [ ] **Step 7: Build, test, and see it live**

Run: `cargo test && cargo clippy --all-targets`
Expected: green, no new warnings.

Run: `cargo build --release && target/release/yutani status | python3 -m json.tool | grep -A3 '"steam"'` against the running packaged daemon.
Expected: the packaged daemon predates the field, so the key is absent. That is fine; the applet-side default covers it. (A dev daemon started from `target/release/yutani` would show `"steam": []` on this machine now that the launch line is fixed.)

- [ ] **Step 8: Commit**

```bash
git add src/tunnel/status.rs src/applet/client.rs src/applet/display.rs src/steam.rs src/ui/mod.rs
git commit -m "feat(status): the daemon reports broken Steam launch lines

Status.steam (serde(default)) lists every account whose EVE launch line
names a missing yutani or no yutani at all. A 30 s subscription rescans
and emits on change; the daemon logs each change and copies it into the
IPC reply.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_015SKJc3TpQiPf631xfgAYyF"
```

---

### Task 4: The applet's warning dot and popover notice

**Files:**
- Modify: `src/applet/icon.rs:80-115` (`icon_state`), tests below
- Modify: `src/applet/display.rs:99-140` (`Popover`, `popover_height`, `graph_fits`), `:170-250` (`popover`), tests
- Modify: `src/applet/theme.rs:163-171` (colour roles)
- Modify: `src/bin/yutani-applet/view.rs:24-58` (`Role`), `:109-113` (`panel_button`), `:440-455` (`popup`)

**Interfaces:**
- Consumes: `Status.steam`, `steam::first_message`.
- Produces:
  - `pub fn icon_state(tunnel: Option<&TunnelStatus>, clients: usize, pending: bool, steam_problem: bool) -> IconState`
  - `Popover.notice: Option<String>`
  - `pub fn popover_height(accounts: usize, menu_open: bool, graph: bool, notice: bool) -> i32`
  - `pub fn graph_fits(available: Option<i32>, accounts: usize, menu_open: bool, notice: bool) -> bool`
  - `pub const NOTICE_ROW_PX: i32 = 26;` in `display.rs`
  - `pub fn theme::warning(c: &Cosmic) -> Color`

- [ ] **Step 1: Write the failing icon tests**

In `src/applet/icon.rs` tests, every existing `icon_state(a, b, c)` call gains a fourth argument `false`. Then add:

```rust
    /// A broken Steam launch line is the same kind of news as a failed
    /// unit: Yutani is running and something the user set up is wrong.
    /// It outranks every live state but not the dim mark, which already
    /// says there is nothing to run through.
    #[test]
    fn a_steam_problem_demands_attention_unless_the_mark_is_dim() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 0, false, true), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), 2, false, true), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, None)), 0, false, true), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), 0, true, true), IconState::Attention);
        assert_eq!(icon_state(None, 0, false, true), IconState::Dim);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), 0, false, true), IconState::Dim);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib applet::icon::`
Expected: `this function takes 3 arguments but 4 were supplied`.

- [ ] **Step 3: Implement `icon_state`**

Replace the signature and the top of the function in `src/applet/icon.rs`:

```rust
/// `tunnel` is `None` when the daemon did not answer. `clients` is how many
/// EVE clients the daemon is tracking — it is what separates the dotted
/// [`IconState::Active`] mark from the plain one. `pending` is true for
/// [`super::PENDING_S`] after a `tunnel connect|disconnect` was sent.
/// `steam_problem` is true when the daemon found a Steam account whose EVE
/// launch line is broken (`Status::steam`): the warning dot, unless the
/// mark is dim anyway.
pub fn icon_state(tunnel: Option<&TunnelStatus>, clients: usize, pending: bool, steam_problem: bool) -> IconState {
    let Some(t) = tunnel else { return IconState::Dim };
    if !t.installed {
        return IconState::Dim;
    }
    // A launch line that will fail (or bypass the tunnel) is set-up news
    // the user must see before pressing Play; it outranks a pending
    // action's sync ring.
    if steam_problem {
        return IconState::Attention;
    }
    if pending {
        return IconState::Sync;
    }
```

The rest of the function is unchanged.

- [ ] **Step 4: Write the failing popover tests**

In `src/applet/display.rs` tests, add a helper and tests:

```rust
    fn with_steam(mut s: Status, verdict: crate::steam::Verdict) -> Status {
        s.steam = vec![crate::steam::Finding { verdict, file: "/h/localconfig.vdf".into() }];
        s
    }

    #[test]
    fn a_steam_problem_is_one_notice_line_and_a_healthy_steam_none() {
        let s = status(true, Some(21), 3);
        assert_eq!(popover(Some(&s), Rates::default(), flat(), None, false).notice, None);
        let broken = with_steam(status(true, Some(21), 3), crate::steam::Verdict::Broken { path: "/usr/local/bin/yutani".into() });
        assert_eq!(
            popover(Some(&broken), Rates::default(), flat(), None, false).notice.as_deref(),
            Some("Steam launches EVE through /usr/local/bin/yutani, which is missing. Open Settings → Steam.")
        );
        let bare = with_steam(status(false, None, 0), crate::steam::Verdict::NoWrapper);
        assert_eq!(
            popover(Some(&bare), Rates::default(), flat(), None, false).notice.as_deref(),
            Some("Steam launches EVE without yutani, so it runs outside the tunnel. Open Settings → Steam.")
        );
        assert_eq!(popover(None, Rates::default(), flat(), None, false).notice, None, "stopped: nothing to say");
    }

    #[test]
    fn the_notice_row_counts_toward_the_height() {
        assert_eq!(popover_height(3, false, true, true) - popover_height(3, false, true, false), NOTICE_ROW_PX);
        // With the notice taking its row, the graph drops one row sooner.
        let full = popover_height(3, false, true, false);
        let broken = with_steam(status(true, Some(21), 3), crate::steam::Verdict::NoWrapper);
        assert!(popover(Some(&broken), Rates::default(), flat(), Some(full + NOTICE_ROW_PX), false).graph);
        assert!(!popover(Some(&broken), Rates::default(), flat(), Some(full), false).graph);
    }
```

Update the existing `the_graph_card_is_dropped_first_on_a_short_screen` test: every `popover_height(a, b, c)` call gains a fourth argument `false`.

- [ ] **Step 5: Run to verify they fail**

Run: `cargo test --lib applet::display::`
Expected: `no field notice`, `this function takes 3 arguments but 4 were supplied`.

- [ ] **Step 6: Implement the notice**

In `src/applet/display.rs`:

1. `Popover` gains, after `primary`:

```rust
    /// One line of set-up news above the primary action — a Steam launch
    /// line that will fail or bypass the tunnel — `None` when there is
    /// nothing to say.
    pub notice: Option<String>,
```

2. Above `popover_height`:

```rust
/// The notice row's height in the estimate: `theme::NOTE_SIZE` text plus
/// `theme::NOTE_PAD`'s 10 px below it.
pub const NOTICE_ROW_PX: i32 = 26;
```

3. `popover_height` and `graph_fits`:

```rust
pub fn popover_height(accounts: usize, menu_open: bool, graph: bool, notice: bool) -> i32 {
    let header = 50;
    let rows = if accounts == 0 { 44 } else { 30 * accounts as i32 };
    let accounts_card = 36 + rows + 44 + 12;
    let tunnel = 12 + 26 + 32;
    let graph_card = if graph { 172 + 12 } else { 0 };
    let tiles = 60 + 12;
    let notice = if notice { NOTICE_ROW_PX } else { 0 };
    let action = 38 + 12;
    let menu = if menu_open { 8 + 3 * 31 + 8 } else { 0 };
    header + accounts_card + tunnel + graph_card + tiles + notice + action + menu
}

/// Whether the graph card fits in `available` pixels (`None`: unknown, so
/// it is kept). The handoff's invariant: the popover never exceeds the
/// panel work area, and the graph card is the first thing to drop.
pub fn graph_fits(available: Option<i32>, accounts: usize, menu_open: bool, notice: bool) -> bool {
    available.is_none_or(|h| popover_height(accounts, menu_open, true, notice) <= h)
}
```

4. In `popover`: the stopped literal gains `notice: None,` and `graph: graph_fits(available, 0, menu_open, false),`. In the running branch, before `Popover {`:

```rust
    let notice = crate::steam::first_message(&s.steam).map(|m| format!("{m} Open Settings → Steam."));
```

and in the literal: `graph: graph_fits(available, n, menu_open, notice.is_some()),` and `notice,` as the last field.

- [ ] **Step 7: Run the library tests**

Run: `cargo test --lib applet::`
Expected: green.

- [ ] **Step 8: Render it**

`src/applet/theme.rs`, next to `success`/`accent`/`destructive`:

```rust
pub fn warning(c: &Cosmic) -> Color {
    c.warning_color().into()
}
```

`src/bin/yutani-applet/view.rs`:

1. `Role` gains a variant after `Destructive`:

```rust
    /// Set-up news the user must act on (the Steam notice).
    Warning,
```

and `Role::class` gains `Role::Warning => |t| text_style(theme::warning(t.cosmic())),`.

2. `panel_button`:

```rust
    let clients = state.status.as_ref().map_or(0, |s| s.clients.len());
    let steam_problem = state.status.as_ref().is_some_and(|s| !s.steam.is_empty());
    let icon = icon_state(state.status.as_ref().map(|s| &s.tunnel), clients, state.pending(), steam_problem);
```

3. Above `action_row`, a renderer:

```rust
/// The Steam notice: one warning line above the primary action.
fn notice_line<'a>(text: &str) -> Element<'a, Msg> {
    widget::container(mono(text.to_string(), theme::NOTE_SIZE, Weight::Normal, Role::Warning)).width(Length::Fill).padding(theme::NOTE_PAD).into()
}
```

4. In `popup`, replace `content = content.push(tiles(&p)).push(action_row(&p, state.menu_open));` with

```rust
    content = content.push(tiles(&p));
    if let Some(notice) = &p.notice {
        content = content.push(notice_line(notice));
    }
    content = content.push(action_row(&p, state.menu_open));
```

- [ ] **Step 9: Build, test, look**

Run: `cargo test && cargo clippy --all-targets`
Expected: green.

Run: `cargo build --release`, then `pkill -x yutani-applet; sleep 1; pkill -x cosmic-panel` only if the panel is running the source applet; the packaged panel applet is unchanged until the next `makepkg -si`. Skip the visual check if the package is what runs; Task 6 reinstalls.

- [ ] **Step 10: Commit**

```bash
git add src/applet/icon.rs src/applet/display.rs src/applet/theme.rs src/bin/yutani-applet/view.rs
git commit -m "feat(applet): warning dot and popover notice for a broken Steam launch line

A non-empty Status.steam turns the mark's dot to warning (unless dim) and
adds one line above the primary action naming the missing path and
pointing at Settings → Steam. The height estimate counts the row.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_015SKJc3TpQiPf631xfgAYyF"
```

---

### Task 5: The settings Steam page banner and re-check

**Files:**
- Modify: `src/ui/settings_ui.rs:262-310` (`Tint`, `tint_color`, new `warning_note`)
- Modify: `src/ui/settings.rs:199-247` (`State`), `:250-300` (`State::new`), `:1063-1125` (`steam_page`), tests `:1125-1150` (`test_state`)
- Modify: `src/ui/mod.rs:1327-1350` (`open_settings_at`), `:1395-1404` (`S::Opened`), `:1508-1520` (`S::Page`), `Msg::SteamChecked` handler from Task 3, new `recheck_steam`

**Interfaces:**
- Consumes: `yutani::steam::{Finding, first_message, problems}`, `Msg::SteamChecked`.
- Produces:
  - `State.steam_findings: Vec<yutani::steam::Finding>`
  - `pub fn steam_banner(findings: &[yutani::steam::Finding]) -> Option<String>` in `settings.rs`
  - `pub fn ui::warning_note<'a, M: 'a>(body: String) -> Element<'a, M>`
  - `App::recheck_steam(&self) -> Task<cosmic::Action<Msg>>`

- [ ] **Step 1: Write the failing test**

In `src/ui/settings.rs` tests, add `steam_findings: Vec::new(),` to `test_state()` and add:

```rust
    /// The Steam page says the same thing the applet does, minus the
    /// "open Settings" tail — the reader is already here.
    #[test]
    fn the_steam_banner_is_the_first_problem_or_nothing() {
        assert_eq!(steam_banner(&[]), None);
        let broken = yutani::steam::Finding {
            verdict: yutani::steam::Verdict::Broken { path: "/usr/local/bin/yutani".into() },
            file: "/h/localconfig.vdf".into(),
        };
        assert_eq!(
            steam_banner(&[broken]).as_deref(),
            Some("Steam launches EVE through /usr/local/bin/yutani, which is missing.")
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --bin yutani ui::settings::`
Expected: `no field steam_findings`, `cannot find function steam_banner`.

- [ ] **Step 3: Implement**

`src/ui/settings_ui.rs`:

1. `Tint` gains `Warning,` and `tint_color` gains `Tint::Warning => c.warning_color().into(),`.

2. After `info_note`:

```rust
/// The warning note: a warning `!` glyph and a wrapping line, on the
/// warning tint. Owned text: the sentence names a path read at runtime.
pub fn warning_note<'a, M: 'a>(body: String) -> Element<'a, M> {
    let glyph = widget::container(mono("!", NOTE, Weight::Semibold, Role::Warning))
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(chip_class(false));
    widget::container(Row::new().spacing(9).align_y(Alignment::Start).push(glyph).push(prose(body, NOTE, Role::Ink)))
        .width(Length::Fill)
        .padding(NOTE_PAD)
        .class(panel_class(Tint::Warning, NOTE_RADIUS))
        .into()
}
```

`src/ui/settings.rs`:

1. `State` gains, after `exe_path`:

```rust
    /// Steam accounts whose EVE launch line is broken (`yutani::steam`),
    /// seeded from `App::steam_findings` when the window opens and
    /// refreshed by `Msg::SteamChecked`.
    pub steam_findings: Vec<yutani::steam::Finding>,
```

and `State::new`'s literal gains `steam_findings: Vec::new(),`.

2. Next to `launch_command`:

```rust
/// The Steam page's banner: the first problem's sentence, without the
/// popover's "Open Settings → Steam" tail.
pub fn steam_banner(findings: &[yutani::steam::Finding]) -> Option<String> {
    yutani::steam::first_message(findings)
}
```

3. In `steam_page`, replace the final `pane(…)` call with:

```rust
    let mut sections: Vec<Element<'_, Msg>> = Vec::new();
    if let Some(problem) = steam_banner(&state.steam_findings) {
        sections.push(ui::warning_note(problem));
    }
    sections.push(ui::card(vec![steps, block]));
    sections.push(ui::info_note("Launch each EVE account as usual afterwards — Yutani picks up every client automatically and gives it the next free hotkey."));
    pane("Steam", "One-time setup so Steam launches EVE through Yutani.", sections)
```

`src/ui/mod.rs`:

1. `Msg::SteamChecked` handler: add the block deferred from Task 3, so it reads

```rust
                self.steam_findings = findings.clone();
                if let Some(state) = self.settings.as_mut() {
                    state.steam_findings = findings;
                }
                Task::none()
```

2. `open_settings_at`, after `state.tunnel.busy = …;`:

```rust
        state.steam_findings = self.steam_findings.clone();
```

3. A method next to `refresh_tunnel`:

```rust
    /// Re-scan Steam's launch lines off the UI thread; the answer comes
    /// back as `Msg::SteamChecked`. Run when the settings window opens and
    /// when its Steam page is shown, so a fix made in Steam a moment ago
    /// is not reported as still broken for up to `steam::PERIOD`.
    fn recheck_steam(&self) -> Task<cosmic::Action<Msg>> {
        Task::perform(
            async { tokio::task::spawn_blocking(yutani::steam::problems).await.unwrap_or_default() },
            |findings| cosmic::Action::App(Msg::SteamChecked(findings)),
        )
    }
```

4. `S::Opened | S::Raise(None) | S::Recheck` arm: `return Task::batch([self.refresh_characters(), self.refresh_tunnel(), self.recheck_steam()]);`

5. `S::Page(entity)` arm:

```rust
            S::Page(entity) => {
                let mut to_steam = false;
                if let Some(state) = self.settings.as_mut() {
                    state.pages.activate(*entity);
                    // The note was about the page being left behind, and so
                    // was every armed confirm and open menu.
                    if settings::clears_note(&msg) {
                        state.note = None;
                    }
                    state.clear_transient();
                    to_steam = state.page() == settings::Page::Steam;
                }
                return if to_steam { self.recheck_steam() } else { Task::none() };
            }
```

- [ ] **Step 4: Test and clippy**

Run: `cargo test && cargo clippy --all-targets`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/ui/settings_ui.rs src/ui/settings.rs src/ui/mod.rs
git commit -m "feat(settings): Steam page banner for a broken launch line

The page shows the first problem's sentence on a warning note above the
launch line, seeded from the daemon's findings on open and re-scanned
when the window opens or the Steam page is shown.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_015SKJc3TpQiPf631xfgAYyF"
```

---

### Task 6: Docs, the absolute-path constant, and the reinstall

**Files:**
- Modify: `src/lib.rs:36-40` (`STEAM_LAUNCH_ARGS_ABSOLUTE`)
- Modify: `src/ui/settings.rs:1205-1223` (the pinned-string test), `:1147` (`test_state` exe_path)
- Modify: `README.md:40-80`
- Modify: `docs/superpowers/specs/2026-09-14-yutani-package-design.md:46-53`

- [ ] **Step 1: Update the pinned test first**

In `src/ui/settings.rs`, `the_steam_launch_arguments_are_exactly_what_eve_needs`: replace both `/usr/local/bin/yutani` with `/usr/bin/yutani`. In `test_state`, `exe_path: "/usr/bin/yutani".to_string(),`.

Run: `cargo test --bin yutani ui::settings::the_steam_launch_arguments`
Expected: FAIL, left `…/usr/local/bin/yutani…` vs right `…/usr/bin/yutani…`.

- [ ] **Step 2: Fix the constant**

`src/lib.rs`:

```rust
/// [`STEAM_LAUNCH_ARGS`] with the binary spelled out at the path the
/// package installs it to (`/usr/bin/yutani`; the settings window's
/// full-path toggle uses the running binary's real path instead).
pub const STEAM_LAUNCH_ARGS_ABSOLUTE: &str =
    "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/bin/yutani launch -- %command%";
```

Run: `cargo test`
Expected: green.

- [ ] **Step 3: README**

The README was rewritten for the GitHub page on 2026-09-14 (commit "docs: README for the GitHub page") and already says `/usr/bin`, describes the package as the source build, and has a "Steam launch check" row. Only verify:

Run: `grep -n 'usr/local' README.md`
Expected: no output. If there is any, change it to `/usr/bin`.

- [ ] **Step 4: Package spec migration step**

In `docs/superpowers/specs/2026-09-14-yutani-package-design.md`, "Migrating a source install (this machine)", extend the paragraph so it ends:

```markdown
…then `systemctl --user daemon-reload &&
systemctl --user restart yutani` and `pkill -x cosmic-panel`. Finally
re-check each EVE account's Steam launch options: a line spelling
`/usr/local/bin/yutani` must change to `/usr/bin/yutani` or bare
`yutani`, or Play fails silently in `/bin/sh` (this bit on 2026-09-14;
`docs/superpowers/specs/2026-09-14-steam-launch-check-design.md` is the
check that now catches it).
```

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/ui/settings.rs docs/superpowers/specs/2026-09-14-yutani-package-design.md
git commit -m "docs: the packaged binary lives in /usr/bin

STEAM_LAUNCH_ARGS_ABSOLUTE, the README and the package spec's migration
steps all said /usr/local/bin; the migration also never mentioned Steam's
launch options, which is how the 2026-09-14 silent Play failure happened.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_015SKJc3TpQiPf631xfgAYyF"
```

- [ ] **Step 6: Reinstall and verify end to end**

```bash
cd packaging && makepkg -si --noconfirm && cd ..
systemctl --user restart yutani && pkill -x cosmic-panel
sleep 3 && yutani status | python3 -m json.tool | grep -A2 '"steam"'
```

Expected: `"steam": []` (the launch line was fixed this morning).

Then break it on purpose to see the warning: in Steam, EVE Online → Properties → Launch Options, change `/usr/bin/yutani` to `/usr/local/bin/yutani`, close Properties, wait 30 s.
Expected: `yutani status` shows one `broken` finding with that path; the panel icon shows the warning dot; the popover's line reads `Steam launches EVE through /usr/local/bin/yutani, which is missing. Open Settings → Steam.`; Settings → Steam shows the banner. Put the launch line back to `/usr/bin/yutani`, close Properties: within 30 s the dot, the line and the banner go.

Report what was seen. Do not commit anything from this step; `packaging/PKGBUILD`'s pkgver may change and is committed separately if it did (`git status`).
