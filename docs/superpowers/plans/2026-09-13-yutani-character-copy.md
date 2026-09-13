# Character Settings Copy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Characters" page in the settings window that copies one character's EVE interface files (`core_char_<id>.dat`, optionally `core_user_<id>.dat`) over every other character's and account's, with backups, a restore button, ESI-resolved character names, and a guard against running clients.

**Architecture:** A new library module `yutani::eve_settings` holds everything that is not UI: profile discovery (`mod.rs`), the copy/backup/restore file operations (`copy.rs`) and ESI name resolution with a RON cache (`names.rs`). All of it is pure or takes explicit paths, so it is tested against temp directories. The settings window gains a `Page::Characters` whose state and view live in `src/ui/characters.rs`; the daemon (`src/ui/mod.rs`) wires the messages, runs the copy, and fetches names on the blocking pool exactly like the tunnel disconnect task.

**Tech Stack:** Rust 2024, libcosmic (rev `a401af8`), serde/ron, serde_json, `curl` via `yutani::proc::output_with_timeout`. No new crate dependencies.

**Spec:** `docs/superpowers/specs/2026-09-13-yutani-character-copy-design.md`.

## Global Constraints

- No new entries in `Cargo.toml`. Time is formatted by hand (`relative_age`, `backup_name`); HTTP goes through `curl` and `yutani::proc::output_with_timeout`.
- Only files whose name is exactly `core_char_<digits>.dat` or `core_user_<digits>.dat` are ever read, listed, backed up or overwritten. `core_char__.dat`, `core_user__.dat`, `core_char_('char', None, 'dat').dat`, `core_public__.yaml`, `prefs.ini` are never touched.
- Source files are never modified. Every overwrite is: back up the target to `<backup dir>/<file name>`, write the new bytes to `<target>.tmp` in the same directory, rename over the target.
- Backup directory: `~/.local/share/yutani/backups/<YYYYMMDDTHHMMSSZ>/` (`dirs::data_dir()`), UTC.
- Name cache: `~/.config/yutani/characters.ron` (`dirs::config_dir()`), written with `crate::model::write_atomic`.
- ESI: `POST https://esi.evetech.net/latest/universe/names/` with a JSON array body, headers `Content-Type: application/json`, `Accept: application/json`, `User-Agent: yutani (COSMIC EVE companion)`; `curl -sS -m 8`, process timeout 10 s. Only entries with `"category": "character"` are kept. If the batch fails, each id is retried alone (ESI answers 404 for the whole batch when one id is unknown).
- Config key: `eve_settings_dir: Option<String>` on `Config`, default `None`; `validate` drops a relative path back to `None` with the usual warning.
- Page order in `settings::PAGES`: Display, Behavior, Layouts, Characters, Steam.
- Copy button disabled while `App::clients` is non-empty or fewer than two characters are listed; the reason is shown as a caption under the button.
- Note text after a copy: `copied <source label> to <n> characters and <m> accounts; backup in <backup dir>` (`and <m> accounts` only when accounts were copied).
- Tests: `cargo test` must stay green (currently 297 tests); both binaries build warning-free (`cargo build --release 2>&1 | grep -E '^(warning|error)'` prints nothing).
- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01QVnCPYL1bQJqdXAK6dRnD9
  ```

---

### Task 1: Profile discovery and file listing (`src/eve_settings/mod.rs`) + config key

**Files:**
- Create: `src/eve_settings/mod.rs`
- Modify: `src/lib.rs` (add `pub mod eve_settings;` after `pub mod assets;`)
- Modify: `src/model/config.rs` (`Config` gains `eve_settings_dir: Option<String>`; `Default`; `validate`)

**Interfaces:**
- Consumes: `crate::model::config::Config` (serde `#[serde(default)]` struct).
- Produces (used by Tasks 2–4):
  ```rust
  pub const EVE_STEAM_APP_ID: &str = "8500";
  pub struct Entry { pub id: u64, pub path: PathBuf, pub size: u64, pub modified: SystemTime }
  pub struct Listing { pub dir: PathBuf, pub characters: Vec<Entry>, pub accounts: Vec<Entry> }
  pub enum Kind { Character, Account }
  pub fn parse_file_name(name: &str) -> Option<(Kind, u64)>;
  pub fn steam_libraries(vdf: &str) -> Vec<PathBuf>;
  pub fn steam_vdf_candidates(home: &Path) -> [PathBuf; 3];
  pub fn profile_dir_in(library: &Path) -> Option<PathBuf>;
  pub fn discover(override_dir: Option<&Path>, home: &Path) -> Result<PathBuf, String>;
  pub fn list(dir: &Path) -> std::io::Result<Listing>;
  pub fn relative_age(then: SystemTime, now: SystemTime) -> String;
  impl Listing { pub fn character(&self, id: u64) -> Option<&Entry>; pub fn account(&self, id: u64) -> Option<&Entry>; }
  ```

- [ ] **Step 1: Write the failing tests**

Create `src/eve_settings/mod.rs` with only the test module first:

```rust
//! EVE's per-profile settings files (spec: `docs/superpowers/specs/2026-09-13-yutani-character-copy-design.md`).
//!
//! Discovery of the profile directory, listing of the per-character and
//! per-account files, and nothing else: the copy lives in [`copy`], the
//! names in [`names`]. Everything takes explicit paths so it runs against
//! a temp directory in tests.

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-eve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_numeric_core_files_are_settings() {
        assert_eq!(parse_file_name("core_char_90000001.dat"), Some((Kind::Character, 90000001)));
        assert_eq!(parse_file_name("core_user_571002.dat"), Some((Kind::Account, 571002)));
        for junk in [
            "core_char__.dat",
            "core_user__.dat",
            "core_char_('char', None, 'dat').dat",
            "core_public__.yaml",
            "prefs.ini",
            "core_char_12.dat.bak",
            "core_char_12.tmp",
            "core_char_.dat",
            "Core_char_12.dat",
        ] {
            assert_eq!(parse_file_name(junk), None, "{junk}");
        }
    }

    #[test]
    fn steam_libraries_come_from_the_path_keys() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/user/.local/share/Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"8500\"\t\t\"123\"\n\t\t}\n\t}\n}\n";
        assert_eq!(
            steam_libraries(vdf),
            vec![PathBuf::from("/home/user/.local/share/Steam"), PathBuf::from("/mnt/games/SteamLibrary")]
        );
        // Windows-style escaping in the value is unescaped; garbage lines are skipped.
        assert_eq!(steam_libraries("\"path\" \"C:\\\\Games\\\\Steam\"\n\"path\"\n"), vec![PathBuf::from("C:\\Games\\Steam")]);
        assert!(steam_libraries("").is_empty());
    }

    #[test]
    fn the_vdf_candidates_cover_native_and_flatpak_steam() {
        let c = steam_vdf_candidates(Path::new("/home/d"));
        assert_eq!(c[0], PathBuf::from("/home/d/.local/share/Steam/config/libraryfolders.vdf"));
        assert_eq!(c[1], PathBuf::from("/home/d/.steam/steam/config/libraryfolders.vdf"));
        assert_eq!(
            c[2],
            PathBuf::from("/home/d/.var/app/com.valvesoftware.Steam/.local/share/Steam/config/libraryfolders.vdf")
        );
    }

    #[test]
    fn the_profile_dir_prefers_tranquility_and_settings_default() {
        let lib = tmpdir("lib");
        let eve = lib.join("steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE");
        assert_eq!(profile_dir_in(&lib), None, "no EVE prefix yet");
        std::fs::create_dir_all(eve.join("c_ccp_eve_tq_tranquility/settings_Other")).unwrap();
        std::fs::create_dir_all(eve.join("c_ccp_eve_sisi_singularity/settings_Default")).unwrap();
        assert_eq!(
            profile_dir_in(&lib),
            Some(eve.join("c_ccp_eve_tq_tranquility/settings_Other")),
            "the only settings_* dir on Tranquility"
        );
        std::fs::create_dir_all(eve.join("c_ccp_eve_tq_tranquility/settings_Default")).unwrap();
        assert_eq!(profile_dir_in(&lib), Some(eve.join("c_ccp_eve_tq_tranquility/settings_Default")));
        std::fs::remove_dir_all(&lib).unwrap();
    }

    #[test]
    fn discover_uses_the_override_first_and_explains_a_miss() {
        let home = tmpdir("home");
        let profile = home.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        assert_eq!(discover(Some(&profile), &home), Ok(profile.clone()));
        let missing = home.join("nope");
        let err = discover(Some(&missing), &home).unwrap_err();
        assert!(err.contains("eve_settings_dir") && err.contains("nope"), "{err}");
        let err = discover(None, &home).unwrap_err();
        assert!(err.contains("libraryfolders.vdf") && err.contains("eve_settings_dir"), "{err}");
        // A library listed in the vdf that holds the prefix is found.
        let lib = home.join("Library");
        let found = lib.join("steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE/c_ccp_eve_tq_tranquility/settings_Default");
        std::fs::create_dir_all(&found).unwrap();
        let vdf = home.join(".local/share/Steam/config");
        std::fs::create_dir_all(&vdf).unwrap();
        std::fs::write(vdf.join("libraryfolders.vdf"), format!("\"path\" \"{}\"\n", lib.display())).unwrap();
        assert_eq!(discover(None, &home), Ok(found));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn listing_skips_junk_and_sorts_newest_first() {
        let dir = tmpdir("list");
        for (name, bytes) in [
            ("core_char_1.dat", 10usize),
            ("core_char_2.dat", 20),
            ("core_user_7.dat", 30),
            ("core_char__.dat", 1),
            ("core_user__.dat", 1),
            ("core_char_('char', None, 'dat').dat", 1),
            ("prefs.ini", 1),
        ] {
            std::fs::write(dir.join(name), vec![0x7e; bytes]).unwrap();
        }
        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::open(dir.join("core_char_2.dat")).unwrap().set_modified(old).unwrap();
        let listing = list(&dir).unwrap();
        assert_eq!(listing.dir, dir);
        assert_eq!(listing.characters.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2], "newest first");
        assert_eq!(listing.accounts.iter().map(|e| e.id).collect::<Vec<_>>(), vec![7]);
        assert_eq!(listing.character(2).unwrap().size, 20);
        assert_eq!(listing.character(2).unwrap().path, dir.join("core_char_2.dat"));
        assert_eq!(listing.account(7).unwrap().size, 30);
        assert!(listing.character(3).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relative_age_reads_like_a_person_wrote_it() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let ago = |s: u64| relative_age(now - Duration::from_secs(s), now);
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(59 * 60), "59 min ago");
        assert_eq!(ago(3600), "1 h ago");
        assert_eq!(ago(23 * 3600), "23 h ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(ago(14 * 86_400), "2 weeks ago");
        assert_eq!(ago(400 * 86_400), "57 weeks ago");
        // A file from the future (clock skew) is "just now", not a panic.
        assert_eq!(relative_age(now + Duration::from_secs(5), now), "just now");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib eve_settings 2>&1 | tail -5`
Expected: compile errors (`parse_file_name` etc. not found) after adding `pub mod eve_settings;` to `src/lib.rs`.

- [ ] **Step 3: Write the implementation**

Above the test module in `src/eve_settings/mod.rs`:

```rust
pub mod copy;   // Task 2 creates this file; until then leave this line out.
pub mod names;  // Task 3 creates this file; until then leave this line out.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// EVE Online's Steam app id: the Proton prefix lives under `compatdata/8500`.
pub const EVE_STEAM_APP_ID: &str = "8500";

/// Inside a Steam library, the Wine `AppData\Local\CCP\EVE` directory.
const EVE_IN_LIBRARY: &str = "steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE";
const TRANQUILITY_SERVER_DIR: &str = "c_ccp_eve_tq_tranquility";
const DEFAULT_PROFILE_DIR: &str = "settings_Default";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Character,
    Account,
}

/// One `core_char_<id>.dat` or `core_user_<id>.dat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// The settings files of one profile directory, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Listing {
    pub dir: PathBuf,
    pub characters: Vec<Entry>,
    pub accounts: Vec<Entry>,
}

impl Listing {
    pub fn character(&self, id: u64) -> Option<&Entry> {
        self.characters.iter().find(|e| e.id == id)
    }
    pub fn account(&self, id: u64) -> Option<&Entry> {
        self.accounts.iter().find(|e| e.id == id)
    }
}

/// `core_char_<digits>.dat` / `core_user_<digits>.dat`, exactly. EVE also
/// leaves `core_char__.dat` and `core_char_('char', None, 'dat').dat` on
/// the login screen; those are not a character and are never touched.
pub fn parse_file_name(name: &str) -> Option<(Kind, u64)> {
    let stem = name.strip_suffix(".dat")?;
    let (kind, digits) = if let Some(d) = stem.strip_prefix("core_char_") {
        (Kind::Character, d)
    } else if let Some(d) = stem.strip_prefix("core_user_") {
        (Kind::Account, d)
    } else {
        return None;
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((kind, digits.parse().ok()?))
}

/// The `"path"` values of Steam's `libraryfolders.vdf`, in file order.
/// A line parser is enough: the file is Valve's KeyValues text, and the
/// only key wanted is a leaf with a quoted string value.
pub fn steam_libraries(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("\"path\"")?;
            let value = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
            Some(PathBuf::from(value.replace("\\\\", "\\")))
        })
        .collect()
}

/// Where Steam keeps `libraryfolders.vdf`: native (both the XDG path and the
/// legacy `~/.steam` symlink tree) and the Flatpak.
pub fn steam_vdf_candidates(home: &Path) -> [PathBuf; 3] {
    [
        home.join(".local/share/Steam/config/libraryfolders.vdf"),
        home.join(".steam/steam/config/libraryfolders.vdf"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/config/libraryfolders.vdf"),
    ]
}

/// The profile directory inside one Steam library, if EVE is installed
/// there: Tranquility's server directory (else the first `*_tranquility`),
/// then `settings_Default` (else the first `settings_*`).
pub fn profile_dir_in(library: &Path) -> Option<PathBuf> {
    let eve = library.join(EVE_IN_LIBRARY);
    let server = pick_dir(&eve, TRANQUILITY_SERVER_DIR, |name| name.ends_with("_tranquility"))?;
    pick_dir(&server, DEFAULT_PROFILE_DIR, |name| name.starts_with("settings_"))
}

/// `dir/preferred` when it is a directory, else the first (sorted)
/// subdirectory whose name passes `accept`.
fn pick_dir(dir: &Path, preferred: &str, accept: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let p = dir.join(preferred);
    if p.is_dir() {
        return Some(p);
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| accept(name))
        .collect();
    names.sort();
    names.first().map(|name| dir.join(name))
}

/// The profile directory: the config override when set, else the first
/// Steam library (from any `libraryfolders.vdf` under `home`) holding the
/// EVE prefix. The error says what was looked at and how to override it.
pub fn discover(override_dir: Option<&Path>, home: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = override_dir {
        return if dir.is_dir() {
            Ok(dir.to_path_buf())
        } else {
            Err(format!("eve_settings_dir {} is not a directory", dir.display()))
        };
    }
    let mut seen_vdf = false;
    for vdf in steam_vdf_candidates(home) {
        let Ok(text) = std::fs::read_to_string(&vdf) else { continue };
        seen_vdf = true;
        for library in steam_libraries(&text) {
            if let Some(profile) = profile_dir_in(&library) {
                return Ok(profile);
            }
        }
    }
    Err(if seen_vdf {
        "no Steam library holds an EVE Online prefix (compatdata/8500); set eve_settings_dir in config.ron to the \
         profile directory (…/CCP/EVE/c_ccp_eve_tq_tranquility/settings_Default)"
            .to_string()
    } else {
        "no Steam libraryfolders.vdf found; set eve_settings_dir in config.ron to the profile directory \
         (…/CCP/EVE/c_ccp_eve_tq_tranquility/settings_Default)"
            .to_string()
    })
}

/// The settings files in `dir`, newest first.
pub fn list(dir: &Path) -> std::io::Result<Listing> {
    let mut listing = Listing { dir: dir.to_path_buf(), ..Listing::default() };
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some((kind, id)) = name.to_str().and_then(parse_file_name) else { continue };
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let item = Entry { id, path: entry.path(), size: meta.len(), modified: meta.modified()? };
        match kind {
            Kind::Character => listing.characters.push(item),
            Kind::Account => listing.accounts.push(item),
        }
    }
    let newest_first = |a: &Entry, b: &Entry| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id));
    listing.characters.sort_by(newest_first);
    listing.accounts.sort_by(newest_first);
    Ok(listing)
}

/// "just now", "5 min ago", "3 h ago", "2 days ago", "3 weeks ago".
pub fn relative_age(then: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(then).map(|d| d.as_secs()).unwrap_or(0);
    let plural = |n: u64, unit: &str| if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") };
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..1_209_600 => plural(secs / 86_400, "day"),
        _ => plural(secs / 604_800, "week"),
    }
}
```

In `src/lib.rs`, after `pub mod assets;` add `pub mod eve_settings;`.

In `src/model/config.rs`, add to `Config` (after the `tunnel` field, with a doc comment):

```rust
    /// EVE's profile directory (the one holding `core_char_*.dat`), when
    /// Steam's library list cannot find it. Absolute path.
    pub eve_settings_dir: Option<String>,
```

`Default`: `eve_settings_dir: None,`. In `validate`, after the existing `check!` lines:

```rust
        check!(eve_settings_dir, |v: &Option<String>| v.as_deref().is_none_or(|p| Path::new(p).is_absolute()), "absolute path or unset");
```

(import `std::path::Path` if the file does not already.) Add to the existing `validate_replaces_bad_values_with_defaults` test: construct with `eve_settings_dir: Some("relative/dir".into())` and assert it comes back `None`; and in `validate_keeps_good_values` set `eve_settings_dir: Some("/abs".into())` and assert it is kept.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib 2>&1 | grep -E 'test result|FAILED|panicked'`
Expected: all pass, count up by 7 (eve_settings) — config tests still pass.

- [ ] **Step 5: Build warning-free and commit**

Run: `cargo build --release 2>&1 | grep -E '^(warning|error)'` → prints nothing.

```bash
git add src/eve_settings/mod.rs src/lib.rs src/model/config.rs
git commit -m "feat(eve-settings): discover the EVE profile dir and list core_char/core_user files; eve_settings_dir config override"
```

---

### Task 2: Copy, backup and restore (`src/eve_settings/copy.rs`)

**Files:**
- Create: `src/eve_settings/copy.rs`
- Modify: `src/eve_settings/mod.rs` (add `pub mod copy;` at the top)

**Interfaces:**
- Consumes: `super::{Entry, Listing, parse_file_name}`.
- Produces (used by Task 4):
  ```rust
  pub struct Plan { pub source_character: Entry, pub source_account: Option<Entry>, pub targets: Vec<Target> }
  pub struct Target { pub from: PathBuf, pub to: PathBuf, pub kind: Kind }
  pub struct Report { pub characters: usize, pub accounts: usize, pub backup: PathBuf }
  pub fn plan(listing: &Listing, character: u64, account: Option<u64>) -> Result<Plan, String>;
  pub fn backups_dir(data_dir: &Path) -> PathBuf;              // data_dir/yutani/backups
  pub fn backup_name(now: SystemTime) -> String;               // "20260913T024100Z"
  pub fn execute(plan: &Plan, backup: &Path) -> std::io::Result<Report>;
  pub fn latest_backup(backups: &Path) -> Option<PathBuf>;
  pub fn restore(backup: &Path, dir: &Path) -> std::io::Result<usize>;
  ```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-eve-copy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Three characters, two accounts, and every kind of junk file.
    fn profile(tag: &str) -> PathBuf {
        let dir = tmpdir(tag);
        for (name, byte) in [
            ("core_char_1.dat", b'A'),
            ("core_char_2.dat", b'B'),
            ("core_char_3.dat", b'C'),
            ("core_user_10.dat", b'X'),
            ("core_user_20.dat", b'Y'),
            ("core_char__.dat", b'j'),
            ("core_user__.dat", b'j'),
            ("core_char_('char', None, 'dat').dat", b'j'),
            ("prefs.ini", b'j'),
        ] {
            std::fs::write(dir.join(name), vec![byte; 4]).unwrap();
        }
        dir
    }

    #[test]
    fn a_plan_targets_every_other_character_and_optionally_every_other_account() {
        let dir = profile("plan");
        let listing = super::super::list(&dir).unwrap();
        let p = plan(&listing, 1, None).unwrap();
        assert_eq!(p.source_character.id, 1);
        assert!(p.source_account.is_none());
        let mut tos: Vec<_> = p.targets.iter().map(|t| t.to.file_name().unwrap().to_str().unwrap().to_string()).collect();
        tos.sort();
        assert_eq!(tos, vec!["core_char_2.dat", "core_char_3.dat"]);
        assert!(p.targets.iter().all(|t| t.from == dir.join("core_char_1.dat") && t.kind == Kind::Character));

        let p = plan(&listing, 2, Some(10)).unwrap();
        let mut tos: Vec<_> = p.targets.iter().map(|t| t.to.file_name().unwrap().to_str().unwrap().to_string()).collect();
        tos.sort();
        assert_eq!(tos, vec!["core_char_1.dat", "core_char_3.dat", "core_user_20.dat"]);
        assert_eq!(p.source_account.as_ref().unwrap().id, 10);

        assert!(plan(&listing, 9, None).unwrap_err().contains("9"));
        assert!(plan(&listing, 1, Some(99)).unwrap_err().contains("99"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lone_character_has_nothing_to_copy_to() {
        let dir = tmpdir("lone");
        std::fs::write(dir.join("core_char_1.dat"), b"A").unwrap();
        let listing = super::super::list(&dir).unwrap();
        assert!(plan(&listing, 1, None).unwrap_err().contains("no other character"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn execute_backs_up_then_overwrites_and_restore_undoes_it() {
        let dir = profile("exec");
        let listing = super::super::list(&dir).unwrap();
        let backup = dir.join("backups").join("20260913T024100Z");
        let report = execute(&plan(&listing, 1, Some(10)).unwrap(), &backup).unwrap();
        assert_eq!((report.characters, report.accounts), (2, 1));
        assert_eq!(report.backup, backup);
        for name in ["core_char_1.dat", "core_char_2.dat", "core_char_3.dat"] {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"AAAA", "{name}");
        }
        for name in ["core_user_10.dat", "core_user_20.dat"] {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"XXXX", "{name}");
        }
        // Junk untouched, no temporaries left behind, sources not backed up.
        assert_eq!(std::fs::read(dir.join("core_char__.dat")).unwrap(), b"jjjj");
        assert!(!dir.join("core_char_2.tmp").exists());
        assert_eq!(std::fs::read(backup.join("core_char_2.dat")).unwrap(), b"BBBB");
        assert_eq!(std::fs::read(backup.join("core_char_3.dat")).unwrap(), b"CCCC");
        assert_eq!(std::fs::read(backup.join("core_user_20.dat")).unwrap(), b"YYYY");
        assert!(!backup.join("core_char_1.dat").exists());
        assert!(!backup.join("core_user_10.dat").exists());

        assert_eq!(latest_backup(&dir.join("backups")), Some(backup.clone()));
        std::fs::create_dir_all(dir.join("backups/20200101T000000Z")).unwrap();
        assert_eq!(latest_backup(&dir.join("backups")), Some(backup.clone()), "newest by name");
        assert_eq!(latest_backup(&dir.join("nowhere")), None);

        assert_eq!(restore(&backup, &dir).unwrap(), 3);
        assert_eq!(std::fs::read(dir.join("core_char_2.dat")).unwrap(), b"BBBB");
        assert_eq!(std::fs::read(dir.join("core_char_3.dat")).unwrap(), b"CCCC");
        assert_eq!(std::fs::read(dir.join("core_user_20.dat")).unwrap(), b"YYYY");
        assert_eq!(std::fs::read(dir.join("core_char_1.dat")).unwrap(), b"AAAA", "source untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_ignores_anything_that_is_not_a_settings_file() {
        let dir = tmpdir("restore-junk");
        let backup = dir.join("b");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("core_char_5.dat"), b"five").unwrap();
        std::fs::write(backup.join("notes.txt"), b"nope").unwrap();
        std::fs::write(backup.join("core_char__.dat"), b"nope").unwrap();
        assert_eq!(restore(&backup, &dir).unwrap(), 1);
        assert_eq!(std::fs::read(dir.join("core_char_5.dat")).unwrap(), b"five");
        assert!(!dir.join("notes.txt").exists());
        assert!(!dir.join("core_char__.dat").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_names_are_utc_timestamps_that_sort() {
        let t = |s: u64| backup_name(SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        assert_eq!(t(0), "19700101T000000Z");
        assert_eq!(t(951_782_400), "20000229T000000Z"); // leap day
        assert_eq!(t(1_757_728_860), "20250913T020100Z");
        assert_eq!(t(1_789_264_860), "20260913T020100Z");
        assert!(t(1_789_264_860) > t(1_757_728_860));
        assert_eq!(backups_dir(Path::new("/home/d/.local/share")), PathBuf::from("/home/d/.local/share/yutani/backups"));
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib eve_settings::copy 2>&1 | tail -3` → compile errors.

- [ ] **Step 3: Write the implementation**

```rust
//! Copying one character's settings over the others, with a backup first.
//!
//! The `.dat` files are CCP's opaque binary format: copied whole, never
//! merged. Every overwrite is temp-file + rename in the target's directory,
//! after the target has been copied into the backup directory under its
//! own name, so `restore` is a plain copy back.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{Entry, Kind, Listing, parse_file_name};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub from: PathBuf,
    pub to: PathBuf,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub source_character: Entry,
    pub source_account: Option<Entry>,
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub characters: usize,
    pub accounts: usize,
    pub backup: PathBuf,
}

/// Every other character (and, with `account`, every other account) gets
/// the source file. A source that is not in the listing, or a listing with
/// no other character, is an error in words the note line can show.
pub fn plan(listing: &Listing, character: u64, account: Option<u64>) -> Result<Plan, String> {
    let source_character =
        listing.character(character).cloned().ok_or_else(|| format!("character {character} is not in the profile"))?;
    let mut targets: Vec<Target> = listing
        .characters
        .iter()
        .filter(|e| e.id != character)
        .map(|e| Target { from: source_character.path.clone(), to: e.path.clone(), kind: Kind::Character })
        .collect();
    if targets.is_empty() {
        return Err("no other character to copy to".to_string());
    }
    let source_account = match account {
        None => None,
        Some(id) => {
            let source = listing.account(id).cloned().ok_or_else(|| format!("account {id} is not in the profile"))?;
            targets.extend(
                listing
                    .accounts
                    .iter()
                    .filter(|e| e.id != id)
                    .map(|e| Target { from: source.path.clone(), to: e.path.clone(), kind: Kind::Account }),
            );
            Some(source)
        }
    };
    Ok(Plan { source_character, source_account, targets })
}

/// `<data dir>/yutani/backups`.
pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("yutani").join("backups")
}

/// `YYYYMMDDTHHMMSSZ` in UTC: sorts chronologically as a string, and
/// needs no date crate (days-to-civil after Howard Hinnant).
pub fn backup_name(now: SystemTime) -> String {
    let secs = now.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}T{h:02}{m:02}{s:02}Z")
}

/// Back up every target into `backup`, then overwrite each with its
/// source. A failure part-way leaves the files already written in place —
/// they are all in the backup, so `restore` puts everything back.
pub fn execute(plan: &Plan, backup: &Path) -> std::io::Result<Report> {
    std::fs::create_dir_all(backup)?;
    let mut report = Report { characters: 0, accounts: 0, backup: backup.to_path_buf() };
    for target in &plan.targets {
        let name = target.to.file_name().ok_or_else(|| std::io::Error::other("target has no file name"))?;
        std::fs::copy(&target.to, backup.join(name))?;
        let tmp = target.to.with_extension("tmp");
        std::fs::copy(&target.from, &tmp)?;
        std::fs::rename(&tmp, &target.to).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        match target.kind {
            Kind::Character => report.characters += 1,
            Kind::Account => report.accounts += 1,
        }
    }
    Ok(report)
}

/// The newest backup directory by name (names are UTC timestamps).
pub fn latest_backup(backups: &Path) -> Option<PathBuf> {
    std::fs::read_dir(backups)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .max()
        .map(|name| backups.join(name))
}

/// Copy every settings file in `backup` back over `dir`; returns how many.
pub fn restore(backup: &Path, dir: &Path) -> std::io::Result<usize> {
    let mut restored = 0;
    for entry in std::fs::read_dir(backup)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().and_then(parse_file_name).is_none() || !entry.path().is_file() {
            continue;
        }
        let to = dir.join(&name);
        let tmp = to.with_extension("tmp");
        std::fs::copy(entry.path(), &tmp)?;
        std::fs::rename(&tmp, &to).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        restored += 1;
    }
    Ok(restored)
}
```

Add `pub mod copy;` to the top of `src/eve_settings/mod.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib eve_settings 2>&1 | grep -E 'test result|FAILED|panicked'` → all pass (7 + 5).

- [ ] **Step 5: Commit**

```bash
git add src/eve_settings/copy.rs src/eve_settings/mod.rs
git commit -m "feat(eve-settings): plan/execute the copy with per-file backups, latest_backup and restore"
```

---

### Task 3: Character names via ESI with a RON cache (`src/eve_settings/names.rs`)

**Files:**
- Create: `src/eve_settings/names.rs`
- Modify: `src/eve_settings/mod.rs` (add `pub mod names;`)

**Interfaces:**
- Consumes: `crate::proc::output_with_timeout`, `crate::model::write_atomic`.
- Produces (used by Task 4):
  ```rust
  pub type Names = BTreeMap<u64, String>;
  pub const ESI_NAMES_URL: &str = "https://esi.evetech.net/latest/universe/names/";
  pub fn cache_path(config_dir: &Path) -> PathBuf;             // config_dir/yutani/characters.ron
  pub fn load_cache(path: &Path) -> Names;                     // missing/broken → empty
  pub fn save_cache(path: &Path, names: &Names) -> std::io::Result<()>;
  pub fn request_body(ids: &[u64]) -> String;                  // "[90000001,90000002]"
  pub fn parse_response(json: &str) -> Result<Names, String>;  // characters only
  pub fn curl_command(ids: &[u64]) -> std::process::Command;
  pub fn fetch(ids: &[u64]) -> Result<Names, String>;          // batch, then one-by-one on failure
  pub fn resolve(ids: Vec<u64>, cache: PathBuf) -> (Names, Option<String>);
  ```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_a_json_array_of_ids() {
        assert_eq!(request_body(&[90000001, 90000002]), "[90000001,90000002]");
        assert_eq!(request_body(&[]), "[]");
    }

    #[test]
    fn the_response_keeps_characters_only() {
        let json = r#"[{"category":"character","id":90000001,"name":"KestrelVance"},
                       {"category":"corporation","id":98000001,"name":"Some Corp"},
                       {"category":"character","id":90000002,"name":"Sasha-666"}]"#;
        let names = parse_response(json).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[&90000001], "KestrelVance");
        assert_eq!(names[&90000002], "Sasha-666");
        assert!(parse_response("not json").unwrap_err().contains("ESI"));
        assert!(parse_response(r#"{"error":"Ensure all IDs are valid before resolving."}"#).unwrap_err().contains("ESI"));
    }

    #[test]
    fn the_curl_command_posts_json_to_esi() {
        let cmd = curl_command(&[1, 2]);
        assert_eq!(cmd.get_program(), "curl");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&ESI_NAMES_URL.to_string()));
        assert!(args.contains(&"[1,2]".to_string()));
        assert!(args.iter().any(|a| a == "Content-Type: application/json"));
        assert!(args.iter().any(|a| a.starts_with("User-Agent: yutani")));
        assert!(args.windows(2).any(|w| w[0] == "-m" && w[1] == "8"));
        assert!(args.iter().any(|a| a == "-sS"));
    }

    #[test]
    fn the_cache_round_trips_and_tolerates_a_missing_or_broken_file() {
        let dir = std::env::temp_dir().join(format!("yutani-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = cache_path(&dir);
        assert_eq!(path, dir.join("yutani").join("characters.ron"));
        assert!(load_cache(&path).is_empty());
        let mut names = Names::new();
        names.insert(90000001, "KestrelVance".to_string());
        save_cache(&path, &names).unwrap();
        assert_eq!(load_cache(&path), names);
        std::fs::write(&path, "(((").unwrap();
        assert!(load_cache(&path).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Everything cached → no network at all, and the cache is untouched.
    #[test]
    fn resolve_is_offline_when_the_cache_already_has_every_id() {
        let dir = std::env::temp_dir().join(format!("yutani-names-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = cache_path(&dir);
        let mut names = Names::new();
        names.insert(5, "Five".to_string());
        save_cache(&path, &names).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let (got, err) = resolve(vec![5], path.clone());
        assert_eq!(got, names);
        assert!(err.is_none());
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib eve_settings::names 2>&1 | tail -3` → compile errors.

- [ ] **Step 3: Write the implementation**

```rust
//! Character names for the ids in the file names, from ESI's public
//! `universe/names` endpoint, cached in `~/.config/yutani/characters.ron`.
//!
//! `curl` rather than an HTTP crate, like the tunnel worker's exit-IP
//! lookup: one request, no new dependency, and a process timeout around it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::model::write_atomic;
use crate::proc::output_with_timeout;

pub type Names = BTreeMap<u64, String>;

pub const ESI_NAMES_URL: &str = "https://esi.evetech.net/latest/universe/names/";
const USER_AGENT: &str = "User-Agent: yutani (COSMIC EVE companion)";
/// curl's own limit; the process timeout is the backstop above it.
const CURL_MAX_TIME: &str = "8";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);

pub fn cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join("yutani").join("characters.ron")
}

/// A missing or unreadable cache is an empty one: it is only a cache.
pub fn load_cache(path: &Path) -> Names {
    std::fs::read_to_string(path).ok().and_then(|text| ron::from_str(&text).ok()).unwrap_or_default()
}

pub fn save_cache(path: &Path, names: &Names) -> std::io::Result<()> {
    let text = ron::ser::to_string_pretty(names, ron::ser::PrettyConfig::default())
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    write_atomic(path, &text)
}

pub fn request_body(ids: &[u64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Deserialize)]
struct Named {
    category: String,
    id: u64,
    name: String,
}

/// The characters in an ESI `universe/names` reply. Anything that is not
/// the expected array (ESI's `{"error": …}` objects included) is an error
/// that names ESI, so the note line reads as "ESI said no", not "broken".
pub fn parse_response(json: &str) -> Result<Names, String> {
    let entries: Vec<Named> = serde_json::from_str(json).map_err(|_| {
        let short: String = json.chars().take(120).collect();
        format!("ESI did not return names: {short}")
    })?;
    Ok(entries.into_iter().filter(|n| n.category == "character").map(|n| (n.id, n.name)).collect())
}

pub fn curl_command(ids: &[u64]) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-m", CURL_MAX_TIME, "-X", "POST"])
        .args(["-H", "Content-Type: application/json", "-H", "Accept: application/json", "-H", USER_AGENT])
        .args(["--data", &request_body(ids)])
        .arg(ESI_NAMES_URL);
    cmd
}

fn fetch_once(ids: &[u64]) -> Result<Names, String> {
    let output = output_with_timeout(&mut curl_command(ids), PROCESS_TIMEOUT)
        .map_err(|e| format!("cannot run curl: {e}"))?
        .ok_or_else(|| "ESI lookup timed out".to_string())?;
    if !output.status.success() {
        return Err(format!("curl failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_response(&String::from_utf8_lossy(&output.stdout))
}

/// One batch; if that fails (ESI rejects the whole batch when any id is
/// unknown — a biomassed character, say) each id alone, keeping what
/// resolves. The error, if any is left, is the batch's.
pub fn fetch(ids: &[u64]) -> Result<Names, String> {
    if ids.is_empty() {
        return Ok(Names::new());
    }
    match fetch_once(ids) {
        Ok(names) => Ok(names),
        Err(batch_error) if ids.len() > 1 => {
            let mut names = Names::new();
            for id in ids {
                if let Ok(one) = fetch_once(std::slice::from_ref(id)) {
                    names.extend(one);
                }
            }
            if names.is_empty() { Err(batch_error) } else { Ok(names) }
        }
        Err(e) => Err(e),
    }
}

/// The names for `ids`: cache first, ESI for the rest, cache updated when
/// anything new arrived. Returns whatever is known plus the fetch error,
/// so an offline machine still shows the cached names. Blocking — run it
/// on the blocking pool.
pub fn resolve(ids: Vec<u64>, cache: PathBuf) -> (Names, Option<String>) {
    let mut names = load_cache(&cache);
    let missing: Vec<u64> = ids.iter().copied().filter(|id| !names.contains_key(id)).collect();
    if missing.is_empty() {
        return (names, None);
    }
    match fetch(&missing) {
        Ok(fresh) => {
            if !fresh.is_empty() {
                names.extend(fresh);
                if let Err(e) = save_cache(&cache, &names) {
                    tracing::warn!("cannot write {}: {e}", cache.display());
                }
            }
            (names, None)
        }
        Err(e) => (names, Some(e)),
    }
}
```

Add `pub mod names;` to `src/eve_settings/mod.rs`. `serde` derive is already a dependency (`serde = { features = ["derive"] }`); `ron` and `serde_json` are too.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib eve_settings 2>&1 | grep -E 'test result|FAILED|panicked'` → all pass.

- [ ] **Step 5: Commit**

```bash
git add src/eve_settings/names.rs src/eve_settings/mod.rs
git commit -m "feat(eve-settings): character names from ESI universe/names via curl, cached in characters.ron"
```

---

### Task 4: The Characters page (`src/ui/characters.rs`, `src/ui/settings.rs`, `src/ui/mod.rs`)

**Files:**
- Create: `src/ui/characters.rs`
- Modify: `src/ui/settings.rs` (`Page::Characters`, `PAGES` 5 entries, `State.characters`, new `Msg` variants, `view` dispatch + `clients_running` parameter, `is_live_only`/`clears_note` untouched)
- Modify: `src/ui/mod.rs` (`mod characters;` next to `mod settings;`, the `on_settings` arms, `settings_copy_characters`, `settings_restore_backup`, names task; the `view` call passes `!self.clients.is_empty()`)
- Modify: `docs/superpowers/plans/2026-09-13-yutani-character-copy.md` — nothing; the spec is already written.

**Interfaces:**
- Consumes: `yutani::eve_settings::{discover, list, relative_age, Listing, Entry}`, `yutani::eve_settings::copy::{plan, execute, backups_dir, backup_name, latest_backup, restore, Report}`, `yutani::eve_settings::names::{Names, cache_path, load_cache, resolve}`; `Config.eve_settings_dir`; `App.clients: HashMap<_, _>`; `App::settings_note`.
- Produces: `characters::State`, `characters::view(state, clients_running) -> Element<settings::Msg>`, and pure helpers listed below.

- [ ] **Step 1: Write the failing tests** (in `src/ui/characters.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(id: u64, age_secs: u64, now: SystemTime) -> Entry {
        Entry { id, path: PathBuf::from(format!("/p/core_char_{id}.dat")), size: 1, modified: now - Duration::from_secs(age_secs) }
    }

    fn state_with(chars: &[u64], accounts: &[u64]) -> State {
        let now = SystemTime::now();
        let listing = Listing {
            dir: PathBuf::from("/p"),
            characters: chars.iter().map(|id| entry(*id, 0, now)).collect(),
            accounts: accounts.iter().map(|id| entry(*id, 0, now)).collect(),
        };
        let mut s = State::default();
        s.set_listing(Ok(listing));
        s
    }

    #[test]
    fn labels_show_the_name_when_known_and_the_id_when_not() {
        let now = SystemTime::now();
        let mut names = Names::new();
        names.insert(1, "KestrelVance".to_string());
        assert_eq!(character_label(&entry(1, 0, now), &names, now), "KestrelVance · just now");
        assert_eq!(character_label(&entry(2, 7200, now), &names, now), "2 · 2 h ago");
        assert_eq!(account_label(&entry(571002, 120, now), now), "account 571002 · 2 min ago");
    }

    #[test]
    fn selections_survive_a_refresh_and_fall_back_to_the_newest() {
        let mut s = state_with(&[1, 2, 3], &[10, 20]);
        s.source_character = 2;
        s.source_account = 1;
        assert_eq!(s.selected_character(), Some(3));
        assert_eq!(s.selected_account(), Some(20));
        // The files were rewritten: 3 is newest now, and 20 disappeared.
        let now = SystemTime::now();
        s.set_listing(Ok(Listing {
            dir: PathBuf::from("/p"),
            characters: vec![entry(3, 0, now), entry(1, 5, now), entry(2, 9, now)],
            accounts: vec![entry(10, 0, now)],
        }));
        assert_eq!(s.selected_character(), Some(3), "same id, new index");
        assert_eq!(s.selected_account(), Some(10), "gone → newest");
        s.set_listing(Err("nope".to_string()));
        assert_eq!(s.selected_character(), None);
        assert_eq!(s.error.as_deref(), Some("nope"));
    }

    #[test]
    fn the_copy_button_needs_two_characters_and_no_running_client() {
        let s = state_with(&[1, 2], &[10]);
        assert_eq!(copy_blocker(&s, false), None);
        assert_eq!(copy_blocker(&s, true), Some(RUNNING_CLIENT));
        let s = state_with(&[1], &[10]);
        assert_eq!(copy_blocker(&s, false), Some(ONE_CHARACTER));
        let mut s = State::default();
        s.set_listing(Err("no profile".to_string()));
        assert_eq!(copy_blocker(&s, false), Some(NO_PROFILE));
    }

    #[test]
    fn the_account_copy_is_off_until_asked_and_needs_a_second_account() {
        let mut s = state_with(&[1, 2], &[10]);
        assert_eq!(s.account_to_copy(), None, "off by default");
        s.copy_account = true;
        assert_eq!(s.account_to_copy(), None, "one account: nothing to copy to");
        let mut s = state_with(&[1, 2], &[10, 20]);
        s.copy_account = true;
        assert_eq!(s.account_to_copy(), Some(10));
    }

    #[test]
    fn the_note_counts_what_was_copied() {
        let report = Report { characters: 6, accounts: 2, backup: PathBuf::from("/b/20260913T024100Z") };
        assert_eq!(copy_note("KestrelVance", &report), "copied KestrelVance to 6 characters and 2 accounts; backup in /b/20260913T024100Z");
        let report = Report { characters: 1, accounts: 0, backup: PathBuf::from("/b/x") };
        assert_eq!(copy_note("KestrelVance", &report), "copied KestrelVance to 1 character; backup in /b/x");
    }

    #[test]
    fn ids_still_unnamed_are_the_ones_to_fetch() {
        let mut s = state_with(&[1, 2, 3], &[]);
        s.names.insert(2, "Two".to_string());
        assert_eq!(s.unnamed(), vec![1, 3]);
    }
}
```

And in `src/ui/settings.rs` tests, extend `every_page_has_a_tab_and_steam_comes_after_layouts` so it asserts the order `["Display", "Behavior", "Layouts", "Characters", "Steam"]` (read the existing test and keep its shape).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib ui::characters 2>&1 | tail -3` → compile errors.

- [ ] **Step 3: Write `src/ui/characters.rs`**

```rust
//! The settings window's Characters page: copy one character's EVE
//! interface files over every other character's (spec
//! `docs/superpowers/specs/2026-09-13-yutani-character-copy-design.md`).
//!
//! The page's own state and view live here; the messages are
//! `settings::Msg` variants, and the file work happens in `App`
//! (`settings_copy_characters`), which is the only place that knows
//! whether an EVE client is running.

use std::path::PathBuf;
use std::time::SystemTime;

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget;

use yutani::eve_settings::copy::Report;
use yutani::eve_settings::names::Names;
use yutani::eve_settings::{Entry, Listing, relative_age};

use super::settings::Msg;

pub const RUNNING_CLIENT: &str = "Close every EVE client first: the client rewrites these files when it logs out.";
pub const ONE_CHARACTER: &str = "Only one character has settings here; there is nothing to copy to.";
pub const NO_PROFILE: &str = "No EVE profile directory was found.";

#[derive(Debug, Default)]
pub struct State {
    /// The profile's files, refreshed whenever the window (re)opens, the
    /// page is refreshed, or a copy/restore finishes.
    pub listing: Option<Listing>,
    /// Why there is no listing.
    pub error: Option<String>,
    /// Character names by id (cache + ESI); an id without one shows as a number.
    pub names: Names,
    /// Why some names are missing (ESI unreachable); shown as a caption.
    pub names_error: Option<String>,
    /// A names fetch is in flight.
    pub fetching: bool,
    /// Dropdown indices into `listing.characters` / `listing.accounts`.
    pub source_character: usize,
    pub source_account: usize,
    pub copy_account: bool,
    /// The newest backup directory, for the Restore button.
    pub last_backup: Option<PathBuf>,
}

impl State {
    /// Take a fresh listing, keeping the selected ids where they still
    /// exist and falling back to the newest file (index 0) otherwise.
    pub fn set_listing(&mut self, listing: Result<Listing, String>) {
        let (character, account) = (self.selected_character(), self.selected_account());
        match listing {
            Ok(listing) => {
                self.source_character =
                    character.and_then(|id| listing.characters.iter().position(|e| e.id == id)).unwrap_or(0);
                self.source_account =
                    account.and_then(|id| listing.accounts.iter().position(|e| e.id == id)).unwrap_or(0);
                self.listing = Some(listing);
                self.error = None;
            }
            Err(e) => {
                self.listing = None;
                self.error = Some(e);
            }
        }
    }

    pub fn selected_character(&self) -> Option<u64> {
        self.listing.as_ref()?.characters.get(self.source_character).map(|e| e.id)
    }

    pub fn selected_account(&self) -> Option<u64> {
        self.listing.as_ref()?.accounts.get(self.source_account).map(|e| e.id)
    }

    /// The account whose file is copied: only when asked, and only when
    /// there is another account to copy to.
    pub fn account_to_copy(&self) -> Option<u64> {
        if !self.copy_account || self.listing.as_ref().is_none_or(|l| l.accounts.len() < 2) {
            return None;
        }
        self.selected_account()
    }

    /// Character ids with no name yet — what a names fetch asks ESI for.
    pub fn unnamed(&self) -> Vec<u64> {
        self.listing
            .as_ref()
            .map(|l| l.characters.iter().map(|e| e.id).filter(|id| !self.names.contains_key(id)).collect())
            .unwrap_or_default()
    }

    /// The selected character's label for the note line (name, else id).
    pub fn source_label(&self) -> String {
        match self.selected_character() {
            Some(id) => self.names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
            None => String::new(),
        }
    }
}

pub fn character_label(entry: &Entry, names: &Names, now: SystemTime) -> String {
    let name = names.get(&entry.id).cloned().unwrap_or_else(|| entry.id.to_string());
    format!("{name} · {}", relative_age(entry.modified, now))
}

pub fn account_label(entry: &Entry, now: SystemTime) -> String {
    format!("account {} · {}", entry.id, relative_age(entry.modified, now))
}

/// Why the Copy button is disabled, or `None` when it is enabled.
pub fn copy_blocker(state: &State, clients_running: bool) -> Option<&'static str> {
    let Some(listing) = state.listing.as_ref() else { return Some(NO_PROFILE) };
    if listing.characters.len() < 2 {
        return Some(ONE_CHARACTER);
    }
    if clients_running {
        return Some(RUNNING_CLIENT);
    }
    None
}

pub fn copy_note(source: &str, report: &Report) -> String {
    let plural = |n: usize, word: &str| if n == 1 { format!("{n} {word}") } else { format!("{n} {word}s") };
    let mut note = format!("copied {source} to {}", plural(report.characters, "character"));
    if report.accounts > 0 {
        note.push_str(&format!(" and {}", plural(report.accounts, "account")));
    }
    note.push_str(&format!("; backup in {}", report.backup.display()));
    note
}

pub fn view(state: &State, clients_running: bool) -> Element<'_, Msg> {
    let now = SystemTime::now();
    let mut copy = widget::settings::section().title("Copy interface settings").add(widget::text::caption(
        "Copies the overview, window positions and sizes, chat setup and UI layout of one character to every \
         other character in this EVE profile. The files are replaced whole; a backup is taken first.",
    ));
    if let Some(listing) = state.listing.as_ref() {
        let characters: Vec<String> = listing.characters.iter().map(|e| character_label(e, &state.names, now)).collect();
        copy = copy.add(widget::settings::item(
            "Copy from",
            widget::dropdown(characters, Some(state.source_character), Msg::SourceCharacter),
        ));
        copy = copy.add(widget::settings::item(
            "Also copy account settings (shortcuts, general, graphics, audio)",
            widget::toggler(state.copy_account).on_toggle(Msg::CopyAccount),
        ));
        if state.copy_account {
            let accounts: Vec<String> = listing.accounts.iter().map(|e| account_label(e, now)).collect();
            copy = copy.add(widget::settings::item(
                "Account to copy from",
                widget::dropdown(accounts, Some(state.source_account), Msg::SourceAccount),
            ));
            if listing.accounts.len() < 2 {
                copy = copy.add(widget::text::caption("Only one account has settings here; nothing to copy to."));
            }
        }
        if state.fetching {
            copy = copy.add(widget::text::caption("Looking up character names…"));
        } else if let Some(e) = state.names_error.as_deref() {
            copy = copy.add(widget::text::caption(format!("Names unavailable ({e}); ids are shown instead.")));
        }
    }
    let blocker = copy_blocker(state, clients_running);
    copy = copy.add(widget::settings::item_row(vec![
        widget::button::suggested("Copy to all characters").on_press_maybe(blocker.is_none().then_some(Msg::CopyCharacters)).into(),
        widget::button::standard("Refresh").on_press(Msg::RefreshCharacters).into(),
    ]));
    if let Some(reason) = blocker {
        copy = copy.add(widget::text::caption(reason));
    }

    let mut backups = widget::settings::section().title("Backups");
    match state.last_backup.as_ref() {
        Some(dir) => {
            backups = backups.add(widget::text::monotext(dir.display().to_string()).width(Length::Fill));
            backups = backups.add(widget::settings::item_row(vec![
                widget::button::standard("Restore last backup")
                    .on_press_maybe((!clients_running && state.listing.is_some()).then_some(Msg::RestoreBackup))
                    .into(),
            ]));
        }
        None => backups = backups.add(widget::text::caption("No backups yet. One is taken before every copy.")),
    }

    let mut profile = widget::settings::section().title("EVE profile");
    match (state.listing.as_ref(), state.error.as_deref()) {
        (Some(listing), _) => {
            profile = profile.add(widget::text::monotext(listing.dir.display().to_string()).width(Length::Fill));
        }
        (None, Some(error)) => {
            profile = profile.add(widget::text::body(error));
        }
        (None, None) => profile = profile.add(widget::text::caption("Not read yet.")),
    }
    profile = profile.add(widget::text::caption(
        "Found through Steam's library list. To point elsewhere, set eve_settings_dir in ~/.config/yutani/config.ron.",
    ));

    widget::settings::view_column(vec![copy.into(), backups.into(), profile.into()]).into()
}
```

If `widget::dropdown` at this libcosmic rev needs a slice of `impl AsRef<str>` with a borrowed lifetime, keep the `Vec<String>` alive by storing the labels on `State` (`pub character_labels: Vec<String>`, `pub account_labels: Vec<String>`, rebuilt in `set_listing`/when names arrive) and pass `&state.character_labels` — the existing pages pass `labels(&MODES)` (a `Vec<&'static str>`) so a `Vec<String>` by value is expected to work; check the compiler.

- [ ] **Step 4: Wire `src/ui/settings.rs`**

1. `Page` gains `Characters`; `PAGES` becomes 5 entries in the order Display, Behavior, Layouts, Characters, Steam (the const type is `[(&str, Page); 5]`).
2. `State` gains `pub characters: super::characters::State,` (initialised with `Default::default()` in `State::new`).
3. `Msg` gains, with doc comments, after `CopySteamArgs`:
   ```rust
   /// Characters page: dropdown index of the character to copy from.
   SourceCharacter(usize),
   CopyAccount(bool),
   SourceAccount(usize),
   CopyCharacters,
   RestoreBackup,
   RefreshCharacters,
   /// A names lookup finished: what is known, and the error if any id is still unnamed.
   Names(Names, Option<String>),
   ```
   (`use yutani::eve_settings::names::Names;`). `Msg` derives `Clone, Debug` — `Names` is a `BTreeMap`, fine.
4. `view` gains a fourth parameter `clients_running: bool` and dispatches `(Page::Characters, _) => super::characters::view(&state.characters, clients_running)` (this page works with a broken `config.ron` — it touches EVE's files, not ours — so it goes next to the Layouts/Steam arms).
5. `clears_note`: pressing `Page`, `Recheck`, `Opened`, `Raise` already clear; leave as is. `is_live_only` and `apply_config_field` do not mention the new variants (they fall to the `_ =>` arms / are intercepted earlier in `on_settings`) — confirm `apply_config_field`'s catch-all returns `Ok(false)` or similar for unknown variants and that these never reach it (Step 5 intercepts them).

- [ ] **Step 5: Wire `src/ui/mod.rs`**

1. `mod characters;` next to `mod settings;`.
2. Where `settings::view(state, &self.config, focused)` is called, pass `!self.clients.is_empty()` as the new argument.
3. In `on_settings`, in the "Messages that only touch the window's own state" block add:
   ```rust
   S::SourceCharacter(i) => { state.characters.source_character = *i; return Task::none(); }
   S::SourceAccount(i) => { state.characters.source_account = *i; return Task::none(); }
   S::CopyAccount(on) => { state.characters.copy_account = *on; return Task::none(); }
   S::Names(names, error) => {
       state.characters.names.extend(names.clone());
       state.characters.names_error = error.clone();
       state.characters.fetching = false;
       return Task::none();
   }
   ```
   and change the `S::Opened | S::Raise(None) | S::Recheck` arm so that after `state.refresh()` it returns `self.refresh_characters()` instead of `Task::none()`.
4. In the "Pages and actions that are not config fields" block add:
   ```rust
   S::RefreshCharacters => return self.refresh_characters(),
   S::CopyCharacters => { self.settings_copy_characters(); return self.refresh_characters(); }
   S::RestoreBackup => { self.settings_restore_backup(); return self.refresh_characters(); }
   ```
5. New methods on `App` (next to `settings_save_as`):
   ```rust
   /// Characters page: re-read the profile listing, the newest backup and
   /// the name cache, then ask ESI for any id still unnamed (blocking pool).
   fn refresh_characters(&mut self) -> Task<cosmic::Action<Msg>> {
       let Some(state) = self.settings.as_mut() else { return Task::none() };
       let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
       let override_dir = self.config.eve_settings_dir.as_deref().map(Path::new);
       let listing = yutani::eve_settings::discover(override_dir, &home)
           .and_then(|dir| yutani::eve_settings::list(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display())));
       state.characters.set_listing(listing);
       let backups = yutani::eve_settings::copy::backups_dir(&dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")));
       state.characters.last_backup = yutani::eve_settings::copy::latest_backup(&backups);
       let cache = yutani::eve_settings::names::cache_path(&dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")));
       state.characters.names.extend(yutani::eve_settings::names::load_cache(&cache));
       let missing = state.characters.unnamed();
       if missing.is_empty() || state.characters.fetching {
           return Task::none();
       }
       state.characters.fetching = true;
       cosmic::iced::Task::perform(
           async move {
               tokio::task::spawn_blocking(move || yutani::eve_settings::names::resolve(missing, cache))
                   .await
                   .unwrap_or_else(|e| (Default::default(), Some(format!("names task failed: {e}"))))
           },
           |(names, error)| cosmic::Action::App(Msg::Settings(settings::Msg::Names(names, error))),
       )
   }

   /// Characters page: the copy itself. Refused while any EVE toplevel
   /// exists — the client writes these files on logout.
   fn settings_copy_characters(&mut self) {
       let Some(state) = self.settings.as_ref() else { return };
       if let Some(reason) = characters::copy_blocker(&state.characters, !self.clients.is_empty()) {
           return self.settings_note(reason.to_string());
       }
       let Some(listing) = state.characters.listing.as_ref() else { return };
       let Some(character) = state.characters.selected_character() else { return };
       let account = state.characters.account_to_copy();
       let source = state.characters.source_label();
       let backups = yutani::eve_settings::copy::backups_dir(&dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")));
       let backup = backups.join(yutani::eve_settings::copy::backup_name(SystemTime::now()));
       let note = match yutani::eve_settings::copy::plan(listing, character, account) {
           Err(e) => e,
           Ok(plan) => match yutani::eve_settings::copy::execute(&plan, &backup) {
               Ok(report) => characters::copy_note(&source, &report),
               Err(e) => format!("copy failed: {e}; the files already replaced are in {}", backup.display()),
           },
       };
       self.settings_note(note);
   }

   fn settings_restore_backup(&mut self) {
       let Some(state) = self.settings.as_ref() else { return };
       if !self.clients.is_empty() {
           return self.settings_note(characters::RUNNING_CLIENT.to_string());
       }
       let (Some(backup), Some(listing)) = (state.characters.last_backup.clone(), state.characters.listing.as_ref()) else {
           return self.settings_note("nothing to restore".to_string());
       };
       let note = match yutani::eve_settings::copy::restore(&backup, &listing.dir) {
           Ok(n) => format!("restored {n} file{} from {}", if n == 1 { "" } else { "s" }, backup.display()),
           Err(e) => format!("restore failed: {e}"),
       };
       self.settings_note(note);
   }
   ```
   (`use std::path::{Path, PathBuf}; use std::time::SystemTime;` as needed — check what `mod.rs` already imports.)

- [ ] **Step 6: Run the whole suite and build**

Run: `cargo test 2>&1 | grep -E 'test result|FAILED|panicked'` → all pass (297 + Tasks 1–3 + 6 new here).
Run: `cargo build --release 2>&1 | grep -E '^(warning|error)'` → nothing.

- [ ] **Step 7: Commit**

```bash
git add src/ui/characters.rs src/ui/settings.rs src/ui/mod.rs
git commit -m "feat(settings): Characters page — copy one character's EVE settings to all others, with backups, restore and ESI names"
```

---

### Task 5: Hands-on acceptance (Daniel, with the controller's help)

Not a subagent task. After merge and install:

1. `cargo build --release && sudo install -o root -g root -m 0755 target/release/yutani /usr/local/bin/yutani && sudo install -o root -g root -m 0755 target/release/yutani-applet /usr/local/bin/yutani-applet`, `yutani quit`, relaunch `yutani` from Applications.
2. Open Preferences… from the applet → Characters tab: the seven characters appear by name, newest first, KestrelVance selectable; `~/.config/yutani/characters.ron` exists afterwards.
3. With an EVE client open the Copy button is disabled with the "Close every EVE client first" caption; close it and the button enables.
4. Press Copy → note says `copied KestrelVance to 6 characters; backup in …`; `ls -la` in the profile dir shows every `core_char_*.dat` at KestrelVance's size; backup dir holds the six old files.
5. Log a second character in: overview and windows match KestrelVance's.
6. Restore last backup → sizes return to the old ones.
