# EVE Log Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each thumbnail captions its character's solar system and turns red with `dps` / `SCRAM` / `POINT` / `JAM` / `NEUT` tags while the EVE log reports hostile activity against that character.

**Architecture:** A new library module `yutani::eve_logs` (pure parser, file tailer, iced subscription) feeds `LogEvent`s to the daemon, which keeps a pure per-client `Status` (`src/ui/status.rs`) and renders it in `src/ui/thumbnail.rs`. Spec: `docs/superpowers/specs/2026-11-07-eve-log-status-design.md`.

**Tech Stack:** Rust, libcosmic/iced subscriptions, `notify` 8 (already a dependency), `futures-timer`, tokio blocking pool. No new crates.

## Global Constraints

- Never run `cargo fmt` on this repo (memory: it touches 55 files).
- No regex crate; plain string matching only.
- All file I/O in the subscription runs on `tokio::task::spawn_blocking`.
- Nothing in the feature may panic or stall the UI runtime; parse failures are dropped.
- Hold times: damage window 5 s, tackle 10 s, jam 5 s, neut 5 s. Red border `#e11d48`.
- Config fields: `show_status: bool = true`, `eve_logs_dir: Option<String> = None` (absolute path or unset).
- Commit after every task with the `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer.
- Test with `cargo test --lib <filter>`; build the package with `makepkg -sf` in `packaging/` and install with `pkexec pacman -U --noconfirm <pkg>`.

---

## File map

- Create `src/eve_logs/mod.rs` — module root, `LogEvent`, `EventKind`, `EveTime`, `discover()`.
- Create `src/eve_logs/parse.rs` — line parser (pure).
- Create `src/eve_logs/tail.rs` — decoding, offsets, header reading, directory scan, `Reader`.
- Create `src/eve_logs/watch.rs` — `subscription(dir) -> Subscription<Vec<LogEvent>>`.
- Create `src/ui/status.rs` — `Status`, `caption()`.
- Modify `src/lib.rs` — `pub mod eve_logs;`.
- Modify `src/model/config.rs` — two fields + validation.
- Modify `src/ui/mod.rs` — `Client.status`, `App.logs_dir`, `Msg::EveLog`, `Msg::StatusTick`, subscription, routing.
- Modify `src/ui/thumbnail.rs` — caption and border.
- Modify `src/ui/settings.rs` — `ShowStatus` toggle, `LogsDir` text field.
- Modify `src/doctor.rs` — `EVE logs:` line.
- Modify `README.md`, `docs/` — one paragraph.

---

### Task 1: Event types, timestamp, discovery

**Files:**
- Create: `src/eve_logs/mod.rs`
- Modify: `src/lib.rs`

**Interfaces produced:**
```rust
pub struct EveTime(pub u64);                       // seconds since Unix epoch, UTC
impl EveTime { pub fn parse(s: &str) -> Option<EveTime> }  // "2026.11.07 09:09:53"
pub enum EventKind { System(String), Docked, Undocked(String),
    Damage { amount: u32, from: String, weapon: Option<String> },
    Scrambled(String), Disrupted(String), Jammed(String), Neuted { gj: u32, from: String } }
pub struct LogEvent { pub character: String, pub at: EveTime, pub kind: EventKind }
pub fn logs_dir_in(library: &Path) -> Option<PathBuf>
pub fn discover(override_dir: Option<&Path>, home: &Path) -> Result<PathBuf, String>
```

- [ ] **Step 1: Write the failing tests** at the bottom of `src/eve_logs/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eve_time_parses_the_log_stamp() {
        // date -u -d '2026-11-07 09:09:53' +%s
        assert_eq!(EveTime::parse("2026.11.07 09:09:53"), Some(EveTime(1794128993)));
        assert_eq!(EveTime::parse("1970.01.01 00:00:00"), Some(EveTime(0)));
        assert_eq!(EveTime::parse("2026-11-07 09:09:53"), None);
        assert_eq!(EveTime::parse("2026.11.07 09:09"), None);
    }

    #[test]
    fn discovery_prefers_the_override_and_walks_libraries() {
        let root = std::env::temp_dir().join(format!("yutani-logs-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lib = root.join("lib");
        let logs = lib.join("steamapps/compatdata/8500/pfx/drive_c/users/steamuser/Documents/EVE/logs");
        std::fs::create_dir_all(logs.join("Gamelogs")).unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".steam/steam/config")).unwrap();
        std::fs::write(home.join(".steam/steam/config/libraryfolders.vdf"), format!("\"path\" \"{}\"\n", lib.display())).unwrap();
        assert_eq!(discover(None, &home), Ok(logs.clone()));
        assert_eq!(discover(Some(&logs), &home), Ok(logs.clone()));
        assert!(discover(Some(&root.join("nope")), &home).is_err());
        assert!(discover(None, &root.join("nohome")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
```

- [ ] **Step 2: Run** `cargo test --lib eve_logs` → fails to compile (module missing).

- [ ] **Step 3: Implement** `src/eve_logs/mod.rs`:

```rust
//! EVE's game and chat logs, read live for the thumbnails' status line
//! (spec: `docs/superpowers/specs/2026-11-07-eve-log-status-design.md`).
//!
//! [`parse`] turns one line into an event, [`tail`] follows the files,
//! [`watch`] is the daemon's subscription. Shield, armour and hull are not
//! in the logs; "under fire" is derived from incoming hits.

pub mod parse;
pub mod tail;
pub mod watch;

use std::path::{Path, PathBuf};

/// Inside a Steam library, the Wine `Documents\EVE\logs` directory.
const LOGS_IN_LIBRARY: &str = "steamapps/compatdata/8500/pfx/drive_c/users/steamuser/Documents/EVE/logs";

/// An EVE log timestamp, seconds since the Unix epoch (EVE time is UTC).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EveTime(pub u64);

impl EveTime {
    /// `YYYY.MM.DD HH:MM:SS` exactly, as the logs write it.
    pub fn parse(s: &str) -> Option<EveTime> {
        let b = s.as_bytes();
        if b.len() != 19 || b[4] != b'.' || b[7] != b'.' || b[10] != b' ' || b[13] != b':' || b[16] != b':' {
            return None;
        }
        let num = |a: usize, z: usize| s.get(a..z)?.parse::<u64>().ok();
        let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
        let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
            return None;
        }
        // Howard Hinnant's days_from_civil.
        let (y, m, d) = (y as i64, m as i64, d as i64);
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        Some(EveTime((days * 86400 + (hh * 3600 + mm * 60 + ss) as i64) as u64))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Now in this solar system (Local chat, a gate jump, or an undock).
    System(String),
    /// A docking request was accepted.
    Docked,
    /// Left a station into this system.
    Undocked(String),
    /// A hit landed on you.
    Damage { amount: u32, from: String, weapon: Option<String> },
    Scrambled(String),
    Disrupted(String),
    Jammed(String),
    /// Capacitor taken from you by a neutraliser or nosferatu.
    Neuted { gj: u32, from: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEvent {
    pub character: String,
    pub at: EveTime,
    pub kind: EventKind,
}

/// The logs directory inside one Steam library, if EVE has written one.
pub fn logs_dir_in(library: &Path) -> Option<PathBuf> {
    let dir = library.join(LOGS_IN_LIBRARY);
    dir.is_dir().then_some(dir)
}

/// The logs directory: the config override when set, else the first Steam
/// library (from any `libraryfolders.vdf` under `home`) holding one.
pub fn discover(override_dir: Option<&Path>, home: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = override_dir {
        return if dir.is_dir() { Ok(dir.to_path_buf()) } else { Err(format!("eve_logs_dir {} is not a directory", dir.display())) };
    }
    let mut seen_vdf = false;
    for vdf in crate::eve_settings::steam_vdf_candidates(home) {
        let Ok(text) = std::fs::read_to_string(&vdf) else { continue };
        seen_vdf = true;
        for library in crate::eve_settings::steam_libraries(&text) {
            if let Some(dir) = logs_dir_in(&library) {
                return Ok(dir);
            }
        }
    }
    Err(if seen_vdf {
        "no Steam library holds EVE's Documents/EVE/logs; set eve_logs_dir in config.ron".to_string()
    } else {
        "no Steam libraryfolders.vdf found; set eve_logs_dir in config.ron to the Documents/EVE/logs directory".to_string()
    })
}
```

Add `pub mod eve_logs;` to `src/lib.rs` after `pub mod eve_settings;`. Create empty `parse.rs`, `tail.rs`, `watch.rs` (`//! stub` only) so the crate compiles.

- [ ] **Step 4: Run** `cargo test --lib eve_logs` → 2 passed.
- [ ] **Step 5: Commit** `git add src/lib.rs src/eve_logs && git commit -m "eve_logs: event types, timestamps, log-dir discovery"`.

---

### Task 2: Line parser

**Files:** `src/eve_logs/parse.rs`

**Interfaces produced:**
```rust
pub fn strip_markup(s: &str) -> String
pub fn split_stamp(line: &str) -> Option<(EveTime, &str)>   // "[ stamp ] rest"
pub fn game_line(line: &str) -> Option<(EveTime, EventKind)>
pub fn chat_line(line: &str) -> Option<(EveTime, EventKind)>
```

- [ ] **Step 1: Write the failing tests** (verbatim lines from this machine's logs):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::eve_logs::EventKind::*;

    fn kind(line: &str) -> Option<EventKind> { game_line(line).map(|(_, k)| k) }

    #[test]
    fn strip_markup_flattens_names() {
        assert_eq!(strip_markup("<font size=14><color=0xFFFFFF40>(<u><b>Tholos</b></u>)</color></font>Ishtar Komarovo [DDOG.]&lt;MSF.&gt;"),
            "(Tholos)Ishtar Komarovo [DDOG.]<MSF.>");
        assert_eq!(strip_markup("plain"), "plain");
    }

    #[test]
    fn stamp_is_split_off() {
        let (at, rest) = split_stamp("[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora").unwrap();
        assert_eq!(at, EveTime::parse("2026.11.07 09:16:47").unwrap());
        assert_eq!(rest, "(None) Jumping from Messoya to Akora");
        assert_eq!(split_stamp("  Listener: TrilliumONE"), None);
        assert!(split_stamp("\u{feff}[ 2026.11.07 09:20:47 ] EVE System > x").is_some());
    }

    #[test]
    fn navigation() {
        assert_eq!(kind("[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora"), Some(System("Akora".into())));
        assert_eq!(kind("[ 2026.11.07 03:40:12 ] (None) Undocking from Jita IV - Moon 4 - Caldari Navy Assembly Plant to Jita solar system."), Some(Undocked("Jita".into())));
        assert_eq!(kind("[ 2026.07.31 04:11:07 ] (notify) Docking operation already in progress.<br>Estimated time left: 9 seconds."), Some(Docked));
        assert_eq!(kind("[ 2026.11.07 09:15:36 ] (notify) Autopilot warping to Akora"), None);
    }

    #[test]
    fn incoming_damage_only() {
        let hit = "[ 2026.11.07 09:09:53 ] (combat) <color=0xffcc0000><b>50</b> <color=0x77ffffff><font size=10>from</font> <b><color=0xffffffff>Pith Extinguisher</b><font size=10><color=0x77ffffff> - Scourge Torpedo - Hits";
        assert_eq!(kind(hit), Some(Damage { amount: 50, from: "Pith Extinguisher".into(), weapon: Some("Scourge Torpedo".into()) }));
        let bare = "[ 2026.07.25 02:08:48 ] (combat) <color=0xffcc0000><b>28</b> <color=0x77ffffff><font size=10>from</font> <b><color=0xffffffff>Tower Sentry Guristas I</b><font size=10><color=0x77ffffff> - Penetrates";
        assert_eq!(kind(bare), Some(Damage { amount: 28, from: "Tower Sentry Guristas I".into(), weapon: None }));
        let out = "[ 2026.07.31 08:37:45 ] (combat) <color=0xff00ffff><b>31</b> <color=0x77ffffff><font size=10>to</font> <b><color=0xffffffff>Guristas Demolisher</b><font size=10><color=0x77ffffff> - Caldari Navy Scourge Heavy Assault Missile - Hits";
        assert_eq!(kind(out), None);
        assert_eq!(kind("[ 2026.11.07 09:09:52 ] (combat) Pithum Abolisher misses you completely"), None);
    }

    #[test]
    fn tackle_on_you_only() {
        let scram = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp scramble attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b>Emergent Preserver</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>you!";
        assert_eq!(kind(scram), Some(Scrambled("Emergent Preserver".into())));
        let point = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp disruption attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b>Blood Raiders Cruor</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>you!";
        assert_eq!(kind(point), Some(Disrupted("Blood Raiders Cruor".into())));
        let other = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp disruption attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b>Renouncer Harbinger</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>Republic Hurricane";
        assert_eq!(kind(other), None);
        let player = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp scramble attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b><font size=14><color=0xFFFFFF40>(<u><b>Tholos</b></u>)</color></font>Ishtar Komarovo [DDOG.]&lt;MSF.&gt;</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>you!";
        assert_eq!(kind(player), Some(Scrambled("(Tholos)Ishtar Komarovo [DDOG.]<MSF.>".into())));
    }

    #[test]
    fn jam_and_energy() {
        assert_eq!(kind("[ 2026.08.13 04:39:26 ] (combat) You're jammed by Guristas Saboteur - Guristas Saboteur"), Some(Jammed("Guristas Saboteur".into())));
        let neut = "[ 2026.08.13 04:39:26 ] (combat) <color=0xffe57f7f><b>30 GJ</b><color=0x77ffffff><font size=10> energy neutralized </font><b><color=0xffffffff>Emergent Watchman</b><color=0x77ffffff><font size=10> - Emergent Watchman</font>";
        assert_eq!(kind(neut), Some(Neuted { gj: 30, from: "Emergent Watchman".into() }));
        let nos = "[ 2026.08.13 08:24:29 ] (combat) <color=0xffe57f7f><b>-11 GJ</b><color=0x77ffffff><font size=10> energy drained to </font><b><color=0xffffffff>Blood Raiders Cruor</b><color=0x77ffffff><font size=10> - ♦ Cruor</font>";
        assert_eq!(kind(nos), Some(Neuted { gj: 11, from: "Blood Raiders Cruor".into() }));
        let mine = "[ 2026.08.01 01:56:53 ] (combat) <color=0xff7fffff><b>+0 GJ</b><color=0x77ffffff><font size=10> energy drained from </font><b><color=0xffffffff>Pithi Arrogator</b><color=0x77ffffff><font size=10> - Small Energy Nosferatu I</font>";
        assert_eq!(kind(mine), None);
    }

    #[test]
    fn chat_system_change() {
        let line = "\u{feff}[ 2026.11.07 09:20:47 ] EVE System > Channel changed to Local : Korsiki";
        assert_eq!(chat_line(line).map(|(_, k)| k), Some(System("Korsiki".into())));
        assert_eq!(chat_line("\u{feff}[ 2026.11.07 09:20:47 ] Some Pilot > o7"), None);
        assert_eq!(chat_line("          Listener:        TrilliumONE"), None);
    }
}
```

- [ ] **Step 2: Run** `cargo test --lib eve_logs::parse` → compile failure.

- [ ] **Step 3: Implement** `src/eve_logs/parse.rs`:

```rust
//! One log line → one event. Pure; every pattern here is a verbatim line
//! from a real log (see the spec's table of colour tags).

use super::{EveTime, EventKind};

const INCOMING_DAMAGE: &str = "<color=0xffcc0000><b>";
const INCOMING_ENERGY: &str = "<color=0xffe57f7f><b>";
const SCRAM: &str = "<b>Warp scramble attempt</b>";
const POINT: &str = "<b>Warp disruption attempt</b>";

/// Drop every `<…>` tag and decode the three entities the client writes.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            c => out.push(c),
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// `[ YYYY.MM.DD HH:MM:SS ] rest`, with any leading BOM or whitespace.
pub fn split_stamp(line: &str) -> Option<(EveTime, &str)> {
    let line = line.trim_start_matches(['\u{feff}', ' ', '\t']);
    let body = line.strip_prefix("[ ")?;
    let stamp = body.get(..19)?;
    let rest = body.get(19..)?.strip_prefix(" ] ")?;
    Some((EveTime::parse(stamp)?, rest))
}

/// The text of the first `<b>…</b>` in `s`, markup stripped.
fn bold(s: &str) -> Option<String> {
    let start = s.find("<b>")? + 3;
    // The name itself may contain nested <b>…</b> (ship-type prefix), so take
    // the last close tag before the next `<color=0x77ffffff>` (the dim suffix).
    let end = s[start..].find("</b> <color=0x77").or_else(|| s[start..].find("</b><color=0x77")).or_else(|| s[start..].rfind("</b>"))?;
    Some(strip_markup(&s[start..start + end]).trim().to_string())
}

pub fn game_line(line: &str) -> Option<(EveTime, EventKind)> {
    let (at, rest) = split_stamp(line)?;
    let body = rest.strip_prefix('(')?;
    let close = body.find(") ")?;
    let (_tag, text) = (&body[..close], &body[close + 2..]);
    let kind = if let Some(t) = text.strip_prefix("Jumping from ") {
        EventKind::System(t.rsplit(" to ").next()?.trim().to_string())
    } else if let Some(t) = text.strip_prefix("Undocking from ") {
        let sys = t.strip_suffix(" solar system.")?.rsplit(" to ").next()?;
        EventKind::Undocked(sys.trim().to_string())
    } else if text.starts_with("Docking operation already in progress") {
        EventKind::Docked
    } else if let Some(t) = text.strip_prefix(INCOMING_DAMAGE) {
        let amount: u32 = t[..t.find("</b>")?].parse().ok()?;
        let after = &t[t.find("from</font>")? + 11..];
        let from = bold(after)?;
        let tail = strip_markup(&after[after.find("</b>")? + 4..]);
        let parts: Vec<&str> = tail.split(" - ").map(str::trim).filter(|p| !p.is_empty()).collect();
        let weapon = (parts.len() >= 2).then(|| parts[0].to_string());
        EventKind::Damage { amount, from, weapon }
    } else if let Some(t) = text.strip_prefix(INCOMING_ENERGY) {
        let num = &t[..t.find(" GJ")?];
        let gj: u32 = num.trim_start_matches(['-', '+']).parse().ok()?;
        let from = bold(&t[t.find("</font>")?..])?;
        EventKind::Neuted { gj, from }
    } else if text.contains(SCRAM) || text.contains(POINT) {
        if !text.trim_end().ends_with("you!") {
            return None;
        }
        let after = &text[text.find("from</font>")? + 11..];
        let end = after.find("<color=0x77ffffff><font size=10>to")?;
        let from = strip_markup(&after[..end]).trim().to_string();
        if text.contains(SCRAM) { EventKind::Scrambled(from) } else { EventKind::Disrupted(from) }
    } else if let Some(t) = text.strip_prefix("You're jammed by ") {
        EventKind::Jammed(t.split(" - ").next()?.trim().to_string())
    } else {
        return None;
    };
    Some((at, kind))
}

pub fn chat_line(line: &str) -> Option<(EveTime, EventKind)> {
    let (at, rest) = split_stamp(line)?;
    let sys = rest.strip_prefix("EVE System > Channel changed to Local : ")?;
    Some((at, EventKind::System(sys.trim().to_string())))
}
```

- [ ] **Step 4: Run** `cargo test --lib eve_logs::parse` → 7 passed. If `bold()` picks the wrong span on the player-name case, adjust the end-marker search; the tests pin the expected names.
- [ ] **Step 5: Commit** `git add src/eve_logs/parse.rs && git commit -m "eve_logs: parse navigation, damage, tackle, jam and neut lines"`.

---

### Task 3: Decoding, tailing, headers, scan, Reader

**Files:** `src/eve_logs/tail.rs`

**Interfaces produced:**
```rust
pub enum Encoding { Utf8, Utf16Le }
pub fn decode_complete(bytes: &[u8], enc: Encoding) -> (Vec<String>, usize)
pub struct Tail { pub path: PathBuf, pub enc: Encoding, pub offset: u64 }
impl Tail { pub fn at_end(path, enc) -> io::Result<Tail>; pub fn read_new(&mut self) -> io::Result<Vec<String>> }
pub fn listener(path: &Path, enc: Encoding) -> Option<String>
pub fn last_event(path: &Path, enc: Encoding, parse: fn(&str) -> Option<(EveTime, EventKind)>) -> Option<EventKind>
pub struct Scan { pub game: BTreeMap<String, PathBuf>, pub local: BTreeMap<String, PathBuf> }
pub fn scan(logs_dir: &Path, now: SystemTime) -> Scan
pub struct Reader { ... }
impl Reader { pub fn new(dir: PathBuf) -> Reader; pub fn rescan(&mut self) -> Vec<LogEvent>; pub fn poll(&mut self) -> Vec<LogEvent>; pub fn characters(&self) -> Vec<String> }
```

- [ ] **Step 1: Write the failing tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::eve_logs::EventKind;

    fn utf16(s: &str) -> Vec<u8> { s.encode_utf16().flat_map(u16::to_le_bytes).collect() }

    #[test]
    fn utf8_keeps_the_partial_line_for_next_time() {
        let (lines, used) = decode_complete(b"a\r\nb\r\npartial", Encoding::Utf8);
        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(used, 6);
        assert_eq!(decode_complete(b"nothing yet", Encoding::Utf8), (vec![], 0));
    }

    #[test]
    fn utf16_with_boms_and_partial() {
        let bytes = utf16("\u{feff}[ x ] one\r\n\u{feff}[ x ] two\r\n\u{feff}[ x ] par");
        let (lines, used) = decode_complete(&bytes, Encoding::Utf16Le);
        assert_eq!(lines, vec!["[ x ] one", "[ x ] two"]);
        assert_eq!(used, utf16("\u{feff}[ x ] one\r\n\u{feff}[ x ] two\r\n").len());
        // An odd byte count (half a code unit) never advances past the pair.
        let (l, u) = decode_complete(&bytes[..3], Encoding::Utf16Le);
        assert!(l.is_empty() && u == 0);
    }

    #[test]
    fn tail_returns_only_new_complete_lines() {
        let path = std::env::temp_dir().join(format!("yutani-tail-{}.txt", std::process::id()));
        std::fs::write(&path, "old\r\n").unwrap();
        let mut t = Tail::at_end(path.clone(), Encoding::Utf8).unwrap();
        assert_eq!(t.read_new().unwrap(), Vec::<String>::new());
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write as _;
        f.write_all(b"new1\r\nhalf").unwrap();
        assert_eq!(t.read_new().unwrap(), vec!["new1"]);
        f.write_all(b"-done\r\n").unwrap();
        assert_eq!(t.read_new().unwrap(), vec!["half-done"]);
        std::fs::write(&path, "x\r\n").unwrap(); // truncated: start over
        assert_eq!(t.read_new().unwrap(), vec!["x"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn listener_and_backfill_from_real_headers() {
        let dir = std::env::temp_dir().join(format!("yutani-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("g.txt");
        std::fs::write(&game, "------------------------------------------------------------\r\n  Gamelog\r\n  Listener: TrilliumONE\r\n  Session Started: 2026.11.07 02:25:00\r\n------------------------------------------------------------\r\n[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora\r\n[ 2026.11.07 09:18:12 ] (None) Jumping from Akora to Oijanen\r\n[ 2026.11.07 09:18:13 ] (notify) Autopilot engaged\r\n").unwrap();
        assert_eq!(listener(&game, Encoding::Utf8).as_deref(), Some("TrilliumONE"));
        assert_eq!(last_event(&game, Encoding::Utf8, crate::eve_logs::parse::game_line), Some(EventKind::System("Oijanen".into())));
        let stub = dir.join("stub.txt");
        std::fs::write(&stub, "------------------------------------------------------------\r\n  Gamelog\r\n  Session Started: 2026.11.07 06:08:30\r\n------------------------------------------------------------\r\n").unwrap();
        assert_eq!(listener(&stub, Encoding::Utf8), None);
        let local = dir.join("l.txt");
        std::fs::write(&local, utf16("\u{feff}\r\n\r\n        ---------------------------------------------------------------\r\n          Channel ID:      local\r\n          Channel Name:    Local\r\n          Listener:        TrilliumONE\r\n          Session started: 2026.11.07 02:25:00\r\n        ---------------------------------------------------------------\r\n\u{feff}[ 2026.11.07 09:19:24 ] EVE System > Channel changed to Local : Airaken\r\n\u{feff}[ 2026.11.07 09:20:47 ] EVE System > Channel changed to Local : Korsiki\r\n")).unwrap();
        assert_eq!(listener(&local, Encoding::Utf16Le).as_deref(), Some("TrilliumONE"));
        assert_eq!(last_event(&local, Encoding::Utf16Le, crate::eve_logs::parse::chat_line), Some(EventKind::System("Korsiki".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_keeps_the_newest_file_per_character_and_skips_stale_and_stubs() {
        let dir = std::env::temp_dir().join(format!("yutani-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Gamelogs")).unwrap();
        std::fs::create_dir_all(dir.join("Chatlogs")).unwrap();
        let hdr = |n: &str| format!("---\r\n  Gamelog\r\n  Listener: {n}\r\n  Session Started: x\r\n---\r\n");
        std::fs::write(dir.join("Gamelogs/20261107_010000_1.txt"), hdr("Alpha")).unwrap();
        std::fs::write(dir.join("Gamelogs/20261107_020000_1.txt"), hdr("Alpha")).unwrap();
        std::fs::write(dir.join("Gamelogs/20261107_020000.txt"), "---\r\n  Gamelog\r\n  Session Started: x\r\n---\r\n").unwrap();
        std::fs::write(dir.join("Chatlogs/Local_20261107_020000_1.txt"), utf16(&format!("          Listener:        Alpha\r\n"))).unwrap();
        std::fs::write(dir.join("Chatlogs/Corp_20261107_020000_1.txt"), utf16("          Listener:        Alpha\r\n")).unwrap();
        let now = std::time::SystemTime::now();
        let s = scan(&dir, now);
        assert_eq!(s.game.get("Alpha"), Some(&dir.join("Gamelogs/20261107_020000_1.txt")));
        assert_eq!(s.local.get("Alpha"), Some(&dir.join("Chatlogs/Local_20261107_020000_1.txt")));
        assert_eq!(s.game.len(), 1);
        // Two hours in the future, everything is "stale".
        let later = now + std::time::Duration::from_secs(2 * 3600);
        assert!(scan(&dir, later).game.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reader_backfills_system_then_streams_events() {
        let dir = std::env::temp_dir().join(format!("yutani-reader-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Gamelogs")).unwrap();
        std::fs::create_dir_all(dir.join("Chatlogs")).unwrap();
        let game = dir.join("Gamelogs/20261107_020000_1.txt");
        std::fs::write(&game, "---\r\n  Gamelog\r\n  Listener: Alpha\r\n  Session Started: x\r\n---\r\n[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora\r\n").unwrap();
        let mut r = Reader::new(dir.clone());
        let first = r.rescan();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].character, "Alpha");
        assert_eq!(first[0].kind, EventKind::System("Akora".into()));
        assert!(r.poll().is_empty());
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(&game).unwrap();
        f.write_all(b"[ 2026.11.07 09:17:00 ] (combat) You're jammed by Guristas Saboteur - Guristas Saboteur\r\n").unwrap();
        let ev = r.poll();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, EventKind::Jammed("Guristas Saboteur".into()));
        assert_eq!(r.characters(), vec!["Alpha".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run** `cargo test --lib eve_logs::tail` → compile failure.

- [ ] **Step 3: Implement** `src/eve_logs/tail.rs`:

```rust
//! Following the log files: which ones, from where, decoded how.

use std::collections::BTreeMap;
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{EveTime, EventKind, LogEvent, parse};

/// Files not written in this long are not a running client's.
pub const FRESH: Duration = Duration::from_secs(3600);
/// Header lines to look at for `Listener:`.
const HEADER_LINES: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Game logs.
    Utf8,
    /// Chat logs (a BOM on every line).
    Utf16Le,
}

/// Decode every complete line in `bytes` (up to the last line feed) and say
/// how many bytes that consumed; a trailing partial line is left for later.
/// CR and BOM are stripped. Undecodable bytes become U+FFFD rather than
/// stalling the file.
pub fn decode_complete(bytes: &[u8], enc: Encoding) -> (Vec<String>, usize) {
    let text = match enc {
        Encoding::Utf8 => {
            let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else { return (Vec::new(), 0) };
            let used = end + 1;
            (String::from_utf8_lossy(&bytes[..used]).into_owned(), used)
        }
        Encoding::Utf16Le => {
            let even = bytes.len() & !1;
            let units: Vec<u16> = bytes[..even].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
            let Some(end) = units.iter().rposition(|&u| u == 0x000A) else { return (Vec::new(), 0) };
            (String::from_utf16_lossy(&units[..=end]), (end + 1) * 2)
        }
    };
    let (text, used) = text;
    let lines = text.lines().map(|l| l.trim_matches(['\u{feff}', '\r']).to_string()).collect();
    (lines, used)
}

/// One file followed from a byte offset.
#[derive(Debug)]
pub struct Tail {
    pub path: PathBuf,
    pub enc: Encoding,
    pub offset: u64,
}

impl Tail {
    /// Start at the end: only lines written from now on are returned.
    pub fn at_end(path: PathBuf, enc: Encoding) -> io::Result<Tail> {
        let len = std::fs::metadata(&path)?.len();
        let offset = match enc { Encoding::Utf16Le => len & !1, Encoding::Utf8 => len };
        Ok(Tail { path, enc, offset })
    }

    /// Every complete line appended since the last call. A file that shrank
    /// is read again from the start.
    pub fn read_new(&mut self) -> io::Result<Vec<String>> {
        let mut f = std::fs::File::open(&self.path)?;
        let len = f.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
        }
        if len == self.offset {
            return Ok(Vec::new());
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        f.read_to_end(&mut buf)?;
        let (lines, used) = decode_complete(&buf, self.enc);
        self.offset += used as u64;
        Ok(lines)
    }
}

fn read_all(path: &Path, enc: Encoding) -> Option<Vec<String>> {
    let bytes = std::fs::read(path).ok()?;
    let mut bytes = bytes;
    // Make the final line complete so it is decoded too.
    match enc { Encoding::Utf8 => bytes.push(b'\n'), Encoding::Utf16Le => bytes.extend_from_slice(&[0x0A, 0]) }
    Some(decode_complete(&bytes, enc).0)
}

/// The `Listener:` name from the header block, if the file has one.
pub fn listener(path: &Path, enc: Encoding) -> Option<String> {
    let lines = read_all(path, enc)?;
    lines.iter().take(HEADER_LINES).find_map(|l| l.trim().strip_prefix("Listener:").map(|n| n.trim().to_string())).filter(|n| !n.is_empty())
}

/// The last event in the file that `parse` accepts (backfill on first open).
pub fn last_event(path: &Path, enc: Encoding, parse: fn(&str) -> Option<(EveTime, EventKind)>) -> Option<EventKind> {
    read_all(path, enc)?.iter().rev().find_map(|l| parse(l).map(|(_, k)| k))
}

/// Newest game log and Local log per character name.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub game: BTreeMap<String, PathBuf>,
    pub local: BTreeMap<String, PathBuf>,
}

/// Files under `logs_dir` written within [`FRESH`] of `now` that carry a
/// `Listener:`; per character, the one with the lexically greatest name
/// (they start with the session timestamp).
pub fn scan(logs_dir: &Path, now: SystemTime) -> Scan {
    let mut out = Scan::default();
    for (sub, prefix, enc, map) in [
        ("Gamelogs", "", Encoding::Utf8, &mut out.game),
        ("Chatlogs", "Local_", Encoding::Utf16Le, &mut out.local),
    ] {
        let Ok(entries) = std::fs::read_dir(logs_dir.join(sub)) else { continue };
        let mut fresh: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(prefix)))
            .filter(|p| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|m| now.duration_since(m).is_ok_and(|age| age <= FRESH)))
            .collect();
        fresh.sort();
        for path in fresh {
            if let Some(name) = listener(&path, enc) {
                map.insert(name, path);
            }
        }
    }
    out
}

#[derive(Debug)]
struct Tracked {
    character: String,
    tail: Tail,
}

/// Every tracked file and its offset; `rescan` picks files up and drops
/// them, `poll` reads what is new.
#[derive(Debug)]
pub struct Reader {
    dir: PathBuf,
    game: BTreeMap<PathBuf, Tracked>,
    local: BTreeMap<PathBuf, Tracked>,
}

impl Reader {
    pub fn new(dir: PathBuf) -> Reader {
        Reader { dir, game: BTreeMap::new(), local: BTreeMap::new() }
    }

    /// Characters with at least one tracked file.
    pub fn characters(&self) -> Vec<String> {
        let mut v: Vec<String> = self.game.values().chain(self.local.values()).map(|t| t.character.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Re-list the directory. New files start at their end; a Local log's
    /// last system line (else the game log's) is returned as a backfill
    /// event so the system is known at once.
    pub fn rescan(&mut self) -> Vec<LogEvent> {
        let found = scan(&self.dir, SystemTime::now());
        let mut events = Vec::new();
        let wanted_game: Vec<&PathBuf> = found.game.values().collect();
        let wanted_local: Vec<&PathBuf> = found.local.values().collect();
        self.game.retain(|p, _| wanted_game.contains(&p));
        self.local.retain(|p, _| wanted_local.contains(&p));
        for (character, path) in &found.local {
            if self.local.contains_key(path) { continue }
            if let Ok(tail) = Tail::at_end(path.clone(), Encoding::Utf16Le) {
                if let Some(kind) = last_event(path, Encoding::Utf16Le, parse::chat_line) {
                    events.push(LogEvent { character: character.clone(), at: EveTime(0), kind });
                }
                self.local.insert(path.clone(), Tracked { character: character.clone(), tail });
            }
        }
        for (character, path) in &found.game {
            if self.game.contains_key(path) { continue }
            if let Ok(tail) = Tail::at_end(path.clone(), Encoding::Utf8) {
                let have_system = events.iter().any(|e| &e.character == character);
                if !have_system
                    && let Some(kind) = last_event(path, Encoding::Utf8, |l| parse::game_line(l).filter(|(_, k)| matches!(k, EventKind::System(_) | EventKind::Undocked(_))))
                {
                    events.push(LogEvent { character: character.clone(), at: EveTime(0), kind });
                }
                self.game.insert(path.clone(), Tracked { character: character.clone(), tail });
            }
        }
        events
    }

    /// Every event appended to any tracked file since the last call.
    pub fn poll(&mut self) -> Vec<LogEvent> {
        let mut events = Vec::new();
        for (tracked, parse) in self
            .game
            .values_mut()
            .map(|t| (t, parse::game_line as fn(&str) -> Option<(EveTime, EventKind)>))
            .chain(self.local.values_mut().map(|t| (t, parse::chat_line as fn(&str) -> Option<(EveTime, EventKind)>)))
        {
            let lines = match tracked.tail.read_new() {
                Ok(lines) => lines,
                Err(e) => { tracing::debug!("eve log {}: {e}", tracked.tail.path.display()); continue }
            };
            for line in lines {
                if let Some((at, kind)) = parse(&line) {
                    events.push(LogEvent { character: tracked.character.clone(), at, kind });
                }
            }
        }
        events
    }
}
```

- [ ] **Step 4: Run** `cargo test --lib eve_logs::tail` → 6 passed.
- [ ] **Step 5: Commit** `git add src/eve_logs/tail.rs && git commit -m "eve_logs: tail game and Local logs per character"`.

---

### Task 4: Subscription

**Files:** `src/eve_logs/watch.rs`

**Interfaces produced:** `pub fn subscription(dir: PathBuf) -> Subscription<Vec<LogEvent>>`

No unit test (it is wiring over tested parts); the compile and the live run cover it.

- [ ] **Step 1: Implement** `src/eve_logs/watch.rs`:

```rust
//! The daemon's subscription: wake on a directory event (or every 500 ms),
//! read what is new on the blocking pool, emit the events.

use std::path::PathBuf;
use std::time::Duration;

use cosmic::iced::futures::{StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use notify::{RecursiveMode, Watcher};

use super::LogEvent;
use super::tail::Reader;

const FALLBACK_POLL: Duration = Duration::from_millis(500);
/// Re-list the directory this often even without a create event.
const RESCAN_EVERY: u32 = 40;

/// Keyed on `dir`, so a config change to another directory restarts it.
pub fn subscription(dir: PathBuf) -> Subscription<Vec<LogEvent>> {
    Subscription::run_with_id(("yutani-eve-logs", dir.clone()), run(dir))
}

fn run(dir: PathBuf) -> impl iced::futures::Stream<Item = Vec<LogEvent>> {
    let (tx, rx) = mpsc::channel::<bool>(16);
    let watcher = {
        let mut tx = tx.clone();
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res && !matches!(ev.kind, notify::EventKind::Access(_)) {
                let created = matches!(ev.kind, notify::EventKind::Create(_) | notify::EventKind::Remove(_));
                let _ = tx.try_send(created);
            }
        })
        .and_then(|mut w| {
            w.watch(&dir.join("Gamelogs"), RecursiveMode::NonRecursive)?;
            w.watch(&dir.join("Chatlogs"), RecursiveMode::NonRecursive)?;
            Ok(w)
        })
    };
    if let Err(e) = &watcher {
        tracing::warn!("EVE log watcher unavailable, polling only: {e}");
    }
    let reader = Some(Reader::new(dir));
    iced::futures::stream::unfold((rx, watcher, reader, 0u32, true), |(mut rx, watcher, reader, mut n, mut rescan)| async move {
        let mut reader = reader?;
        loop {
            // Wait for a wake-up or the fallback tick.
            let mut delay = futures_timer::Delay::new(FALLBACK_POLL);
            let mut woke = false;
            iced::futures::select! {
                ev = rx.next() => { if let Some(created) = ev { rescan |= created; woke = true; } }
                _ = (&mut delay).fuse() => {}
            }
            if woke {
                // Coalesce the burst (a frame of combat writes many lines).
                futures_timer::Delay::new(Duration::from_millis(50)).await;
                while let Ok(Some(created)) = rx.try_next() { rescan |= created; }
            }
            n += 1;
            let do_rescan = rescan || n % RESCAN_EVERY == 1;
            rescan = false;
            let (events, back) = match tokio::task::spawn_blocking(move || {
                let mut r = reader;
                let mut events = if do_rescan { r.rescan() } else { Vec::new() };
                events.extend(r.poll());
                (events, r)
            }).await {
                Ok(x) => x,
                Err(e) => { tracing::warn!("EVE log read failed: {e}"); return None }
            };
            reader = back;
            if !events.is_empty() {
                return Some((events, (rx, watcher, Some(reader), n, rescan)));
            }
            // Nothing new: keep looping without emitting.
            let _ = &watcher;
            // (loop continues; state is still in scope)
            // SAFETY of the design: no await point holds the reader across the loop
            // boundary other than the move into spawn_blocking above.
            continue;
        }
    })
}
```

Note: the `unfold` state must be rebuilt on `continue`; if the borrow checker rejects reusing `reader` after the move, restructure as a `loop` that reassigns a local tuple `state` each iteration and returns `Some((events, state))` — the shape used by `steam::run`. `FusedFuture` for `select!` comes from `iced::futures::FutureExt::fuse`; `rx.next()` is already fused.

- [ ] **Step 2: Run** `cargo build` → compiles (fix the state threading as noted).
- [ ] **Step 3: Commit** `git add src/eve_logs/watch.rs && git commit -m "eve_logs: watch subscription"`.

---

### Task 5: Status model and caption

**Files:** `src/ui/status.rs`, add `pub mod status;` in `src/ui/mod.rs` next to the other `mod` lines.

**Interfaces produced:**
```rust
pub const DAMAGE_WINDOW: Duration = 5 s; TACKLE_HOLD = 10 s; JAM_HOLD = 5 s; NEUT_HOLD = 5 s;
pub enum Tackle { Scram, Point }
#[derive(Debug, Default)] pub struct Status { pub system: Option<String>, pub docked: bool, ... }
impl Status {
    pub fn apply(&mut self, kind: &EventKind, now: Instant)
    pub fn tick(&mut self, now: Instant)            // expire
    pub fn active(&self, now: Instant) -> bool      // anything timed still live
    pub fn dps(&self, now: Instant) -> Option<u32>
    pub fn under_fire(&self, now: Instant) -> bool
    pub fn tackled(&self, now: Instant) -> bool
    pub fn tags(&self, now: Instant) -> Vec<&'static str>
    pub fn alarm(&self, now: Instant) -> bool       // under_fire || tackled → red border
}
pub fn caption(status: &Status, now: Instant) -> Option<String>
```

- [ ] **Step 1: Write the failing tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use yutani::eve_logs::EventKind;

    fn dmg(n: u32) -> EventKind { EventKind::Damage { amount: n, from: "x".into(), weapon: None } }

    #[test]
    fn dps_is_the_window_sum_over_its_length() {
        let t0 = Instant::now();
        let mut s = Status::default();
        assert_eq!(s.dps(t0), None);
        s.apply(&dmg(100), t0);
        s.apply(&dmg(150), t0 + Duration::from_secs(1));
        assert_eq!(s.dps(t0 + Duration::from_secs(1)), Some(50));
        assert!(s.under_fire(t0 + Duration::from_secs(1)));
        s.tick(t0 + Duration::from_secs(7));
        assert_eq!(s.dps(t0 + Duration::from_secs(7)), None);
        assert!(!s.under_fire(t0 + Duration::from_secs(7)));
        assert!(!s.active(t0 + Duration::from_secs(7)));
    }

    #[test]
    fn holds_expire_and_tags_keep_their_order() {
        let t0 = Instant::now();
        let mut s = Status::default();
        s.apply(&EventKind::Neuted { gj: 1, from: "n".into() }, t0);
        s.apply(&EventKind::Jammed("j".into()), t0);
        s.apply(&EventKind::Scrambled("s".into()), t0);
        assert_eq!(s.tags(t0), vec!["SCRAM", "JAM", "NEUT"]);
        assert!(s.alarm(t0));
        let t6 = t0 + Duration::from_secs(6);
        assert_eq!(s.tags(t6), vec!["SCRAM"]);
        let t11 = t0 + Duration::from_secs(11);
        assert_eq!(s.tags(t11), Vec::<&str>::new());
        assert!(!s.alarm(t11));
        s.apply(&EventKind::Disrupted("p".into()), t11);
        assert_eq!(s.tags(t11), vec!["POINT"]);
    }

    #[test]
    fn system_and_dock_state() {
        let t0 = Instant::now();
        let mut s = Status::default();
        s.apply(&EventKind::System("Jita".into()), t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("Jita"));
        s.apply(&EventKind::Docked, t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("Docked · Jita"));
        s.apply(&EventKind::Undocked("Perimeter".into()), t0);
        assert!(!s.docked);
        assert_eq!(caption(&s, t0).as_deref(), Some("Perimeter"));
        s.apply(&EventKind::System("Urlen".into()), t0);
        assert_eq!(s.system.as_deref(), Some("Urlen"));
    }

    #[test]
    fn caption_composes_system_dps_and_tags() {
        let t0 = Instant::now();
        let mut s = Status::default();
        assert_eq!(caption(&s, t0), None);
        s.apply(&dmg(1200), t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("240 dps"));
        s.apply(&EventKind::System("Korsiki".into()), t0);
        s.apply(&EventKind::Scrambled("s".into()), t0);
        assert_eq!(caption(&s, t0).as_deref(), Some("Korsiki · 240 dps · SCRAM"));
    }
}
```

- [ ] **Step 2: Run** `cargo test --bin yutani ui::status` → compile failure.

- [ ] **Step 3: Implement** `src/ui/status.rs`:

```rust
//! Per-client state derived from the EVE log (spec §Status): where the
//! character is and what is being done to it right now. Pure; the clock is
//! passed in.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use yutani::eve_logs::EventKind;

pub const DAMAGE_WINDOW: Duration = Duration::from_secs(5);
pub const TACKLE_HOLD: Duration = Duration::from_secs(10);
pub const JAM_HOLD: Duration = Duration::from_secs(5);
pub const NEUT_HOLD: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tackle {
    Scram,
    Point,
}

#[derive(Debug, Default)]
pub struct Status {
    pub system: Option<String>,
    pub docked: bool,
    hits: VecDeque<(Instant, u32)>,
    tackled_until: Option<(Instant, Tackle)>,
    jammed_until: Option<Instant>,
    neuted_until: Option<Instant>,
}

impl Status {
    pub fn apply(&mut self, kind: &EventKind, now: Instant) {
        match kind {
            EventKind::System(s) => self.system = Some(s.clone()),
            EventKind::Docked => self.docked = true,
            EventKind::Undocked(s) => {
                self.docked = false;
                self.system = Some(s.clone());
            }
            EventKind::Damage { amount, .. } => self.hits.push_back((now, *amount)),
            EventKind::Scrambled(_) => self.tackled_until = Some((now + TACKLE_HOLD, Tackle::Scram)),
            EventKind::Disrupted(_) => {
                // A scram outranks a point while both hold.
                if !matches!(self.tackled_until, Some((until, Tackle::Scram)) if until > now) {
                    self.tackled_until = Some((now + TACKLE_HOLD, Tackle::Point));
                }
            }
            EventKind::Jammed(_) => self.jammed_until = Some(now + JAM_HOLD),
            EventKind::Neuted { .. } => self.neuted_until = Some(now + NEUT_HOLD),
        }
        self.tick(now);
    }

    /// Drop what has expired.
    pub fn tick(&mut self, now: Instant) {
        while self.hits.front().is_some_and(|(at, _)| now.duration_since(*at) > DAMAGE_WINDOW) {
            self.hits.pop_front();
        }
        if self.tackled_until.is_some_and(|(u, _)| u <= now) { self.tackled_until = None }
        if self.jammed_until.is_some_and(|u| u <= now) { self.jammed_until = None }
        if self.neuted_until.is_some_and(|u| u <= now) { self.neuted_until = None }
    }

    /// Something timed is still live (the 1 Hz tick is needed).
    pub fn active(&self, now: Instant) -> bool {
        self.under_fire(now) || self.tackled(now) || self.jammed_until.is_some_and(|u| u > now) || self.neuted_until.is_some_and(|u| u > now)
    }

    pub fn under_fire(&self, now: Instant) -> bool {
        self.hits.iter().any(|(at, _)| now.duration_since(*at) <= DAMAGE_WINDOW)
    }

    pub fn tackled(&self, now: Instant) -> bool {
        self.tackled_until.is_some_and(|(u, _)| u > now)
    }

    /// Incoming damage per second over the window, `None` when no hit landed.
    pub fn dps(&self, now: Instant) -> Option<u32> {
        let sum: u32 = self.hits.iter().filter(|(at, _)| now.duration_since(*at) <= DAMAGE_WINDOW).map(|(_, n)| n).sum();
        (sum > 0).then(|| sum / DAMAGE_WINDOW.as_secs() as u32)
    }

    pub fn tags(&self, now: Instant) -> Vec<&'static str> {
        let mut v = Vec::new();
        match self.tackled_until {
            Some((u, Tackle::Scram)) if u > now => v.push("SCRAM"),
            Some((u, Tackle::Point)) if u > now => v.push("POINT"),
            _ => {}
        }
        if self.jammed_until.is_some_and(|u| u > now) { v.push("JAM") }
        if self.neuted_until.is_some_and(|u| u > now) { v.push("NEUT") }
        v
    }

    /// Red border: shot at or held.
    pub fn alarm(&self, now: Instant) -> bool {
        self.under_fire(now) || self.tackled(now)
    }
}

/// The second caption line, `None` when there is nothing to say.
pub fn caption(status: &Status, now: Instant) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    match (&status.system, status.docked) {
        (Some(s), true) => parts.push(format!("Docked · {s}")),
        (Some(s), false) => parts.push(s.clone()),
        (None, true) => parts.push("Docked".into()),
        (None, false) => {}
    }
    if let Some(dps) = status.dps(now) {
        parts.push(format!("{dps} dps"));
    }
    parts.extend(status.tags(now).into_iter().map(str::to_string));
    (!parts.is_empty()).then(|| parts.join(" · "))
}
```

- [ ] **Step 4: Run** `cargo test --bin yutani ui::status` → 4 passed.
- [ ] **Step 5: Commit** `git add src/ui/status.rs src/ui/mod.rs && git commit -m "ui: per-client log status and caption"`.

---

### Task 6: Config fields and settings page

**Files:** `src/model/config.rs`, `src/ui/settings.rs`

- [ ] **Step 1: Tests** in `src/model/config.rs` tests module (next to the `eve_settings_dir` ones):

```rust
    #[test]
    fn status_fields_default_and_validate() {
        let c = Config::default();
        assert!(c.show_status);
        assert_eq!(c.eve_logs_dir, None);
        let c = Config { eve_logs_dir: Some("relative".into()), ..Config::default() }.validated();
        assert_eq!(c.eve_logs_dir, None);
    }
```
(Use the same method name the existing `eve_settings_dir` relative-path test calls — read that test at `src/model/config.rs:480-500` and mirror it.)

In `src/ui/settings.rs` tests, next to the `ShowNames` ones:
```rust
        assert_eq!(apply_config_field(&mut c, &Msg::ShowStatus(false)), Ok(true));
        assert!(!c.show_status);
        assert_eq!(apply_config_field(&mut c, &Msg::LogsDir("/mnt/x/logs".into())), Ok(true));
        assert_eq!(c.eve_logs_dir.as_deref(), Some("/mnt/x/logs"));
        assert_eq!(apply_config_field(&mut c, &Msg::LogsDir(String::new())), Ok(true));
        assert_eq!(c.eve_logs_dir, None);
```

- [ ] **Step 2: Run** `cargo test --lib model::config && cargo test --bin yutani settings` → compile failure.

- [ ] **Step 3: Implement.** In `Config` after `show_names`:
```rust
    /// The second caption line: solar system, incoming dps, SCRAM/JAM/NEUT.
    pub show_status: bool,
```
and after `eve_settings_dir`:
```rust
    /// EVE's `Documents/EVE/logs` directory when Steam's library list
    /// cannot find it. Absolute path.
    pub eve_logs_dir: Option<String>,
```
Defaults `show_status: true`, `eve_logs_dir: None`. Validation line after the `eve_settings_dir` check:
```rust
        check!(eve_logs_dir, |v: &Option<String>| v.as_deref().is_none_or(|p| Path::new(p).is_absolute()), "absolute path or unset");
```
In `settings.rs`: `Msg::ShowStatus(bool)` and `Msg::LogsDir(String)` in the enum after `ShowNames`; in `apply_config_field`:
```rust
        Msg::ShowStatus(v) => config.show_status = *v,
        Msg::LogsDir(text) => config.eve_logs_dir = (!text.trim().is_empty()).then(|| text.trim().to_string()),
```
On the Thumbnails page after the `names` row:
```rust
    let status = ui::row("Show system and combat status", Some("Second caption line: solar system, incoming dps, SCRAM / POINT / JAM / NEUT. Red border while shot at or tackled."), ui::toggle(config.show_status, Msg::ShowStatus));
    let logs_dir = ui::row(
        "EVE logs folder",
        Some("Leave empty to find it through Steam. Documents/EVE/logs inside the Proton prefix."),
        widget::text_input("auto", config.eve_logs_dir.as_deref().unwrap_or("")).on_input(Msg::LogsDir).on_submit(Msg::Commit).width(Length::Fixed(ui::SLIDER_W)).into(),
    );
    let thumbnails = ui::card(vec![preview_strip(config), width, zoom, opacity, names, status, logs_dir]);
```
Check how `ActiveBorder(String)` text inputs are built in this file and mirror their `on_input`/commit handling so a typed path is saved (if `is_live_only` lists text messages, `LogsDir` must not be live-only).

- [ ] **Step 4: Run** `cargo test --lib model::config && cargo test --bin yutani settings` → pass.
- [ ] **Step 5: Commit** `git add src/model/config.rs src/ui/settings.rs && git commit -m "config: show_status and eve_logs_dir"`.

---

### Task 7: Wire the daemon

**Files:** `src/ui/mod.rs`

- [ ] **Step 1: Test** in `src/ui/mod.rs` tests (use the existing fake-backend helpers near the `LaunchTick` test at ~line 2530):

```rust
    #[test]
    fn log_events_reach_the_named_client_and_the_tick_runs_while_active() {
        let (mut app, fake) = test_app();           // whatever the existing helper is called
        let h = fake.handle();
        let _ = app.on_backend(Event::ClientAdded(h.clone(), info_named("TrilliumONE")));  // mirror the existing helpers
        let ev = yutani::eve_logs::LogEvent { character: "TrilliumONE".into(), at: yutani::eve_logs::EveTime(0), kind: yutani::eve_logs::EventKind::System("Jita".into()) };
        let _ = app.update(Msg::EveLog(vec![ev]));
        assert_eq!(app.clients[&h].status.system.as_deref(), Some("Jita"));
        let stray = yutani::eve_logs::LogEvent { character: "Nobody".into(), at: yutani::eve_logs::EveTime(0), kind: yutani::eve_logs::EventKind::Jammed("x".into()) };
        let _ = app.update(Msg::EveLog(vec![stray]));
        assert!(!app.status_active());
        let jam = yutani::eve_logs::LogEvent { character: "TrilliumONE".into(), at: yutani::eve_logs::EveTime(0), kind: yutani::eve_logs::EventKind::Jammed("x".into()) };
        let _ = app.update(Msg::EveLog(vec![jam]));
        assert!(app.status_active());
    }
```
Adapt the helper names to the ones in the file.

- [ ] **Step 2: Run** `cargo test --bin yutani log_events_reach` → compile failure.

- [ ] **Step 3: Implement.**
  - `Client` gets `pub status: status::Status,` (init `status::Status::default()` in the `or_insert_with`).
  - `App` gets `pub logs_dir: Option<PathBuf>,`. Set in the constructor (where `App` is built from `AppFlags`) via:
    ```rust
    fn resolve_logs_dir(config: &Config) -> Option<PathBuf> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        match yutani::eve_logs::discover(config.eve_logs_dir.as_deref().map(Path::new), &home) {
            Ok(dir) => { tracing::info!("EVE logs: {}", dir.display()); Some(dir) }
            Err(e) => { tracing::warn!("EVE logs off: {e}"); None }
        }
    }
    ```
    and refreshed in `apply_config` when `new.eve_logs_dir != self.config.eve_logs_dir`.
  - `Msg::EveLog(Vec<yutani::eve_logs::LogEvent>)` and `Msg::StatusTick`.
  - In `update`:
    ```rust
    Msg::EveLog(events) => {
        let now = Instant::now();
        for ev in events {
            if let Some(c) = self.clients.values_mut().find(|c| matches!(&c.info.login, Login::LoggedIn(n) if *n == ev.character)) {
                c.status.apply(&ev.kind, now);
            }
        }
        Task::none()
    }
    Msg::StatusTick => {
        let now = Instant::now();
        for c in self.clients.values_mut() { c.status.tick(now); }
        Task::none()
    }
    ```
    Both arms need a redraw of the thumbnail surfaces; libcosmic redraws layer surfaces on state change via `view_window`, so nothing extra is needed. Verify live; if captions lag, request a redraw the same way `on_backend` does after a frame.
  - `fn status_active(&self) -> bool { let now = Instant::now(); self.clients.values().any(|c| c.status.active(now)) }`.
  - In `subscription`: 
    ```rust
    if let Some(dir) = self.logs_dir.clone() { subs.push(yutani::eve_logs::watch::subscription(dir).map(Msg::EveLog)); }
    if self.status_active() { subs.push(cosmic::iced::time::every(Duration::from_secs(1)).map(|_| Msg::StatusTick)); }
    ```

- [ ] **Step 4: Run** `cargo test --bin yutani` → all pass.
- [ ] **Step 5: Commit** `git add src/ui/mod.rs && git commit -m "daemon: feed EVE log events into client status"`.

---

### Task 8: Thumbnail caption and red border

**Files:** `src/ui/thumbnail.rs`

- [ ] **Step 1: Test** (in the existing tests module; `view` cannot be rendered in a test, so test the colour choice helper):

```rust
    #[test]
    fn alarm_outranks_active_and_inactive() {
        assert_eq!(border_rgba(true, true, None, [0.1, 0.1, 0.1, 1.0], None), ALARM);
        assert_eq!(border_rgba(false, false, None, [0.1, 0.1, 0.1, 1.0], None), [0.1, 0.1, 0.1, 1.0]);
        assert_eq!(border_rgba(false, true, Some([0.0, 1.0, 0.0, 1.0]), [0.1, 0.1, 0.1, 1.0], None), [0.0, 1.0, 0.0, 1.0]);
    }
```

- [ ] **Step 2: Run** `cargo test --bin yutani thumbnail` → compile failure.

- [ ] **Step 3: Implement.**
```rust
/// `#e11d48`.
pub const ALARM: [f32; 4] = [0.882, 0.114, 0.282, 1.0];

/// The border colour: alarm, else the active override or the accent (as
/// `[r,g,b,a]` passed in by the caller who has the theme), else inactive.
pub fn border_rgba(alarm: bool, activated: bool, active_override: Option<[f32; 4]>, inactive: [f32; 4], accent: Option<[f32; 4]>) -> [f32; 4] {
    if alarm { return ALARM }
    match (activated, active_override, accent) {
        (true, Some(c), _) => c,
        (true, None, Some(c)) => c,
        _ => inactive,
    }
}
```
In `view`: `let now = Instant::now(); let alarm = client.status.alarm(now);` and in the frame style closure replace the `match` with `let [r,g,b,a] = border_rgba(alarm, activated, active_override, inactive, Some(accent_of(theme)));` where `accent_of` reads `theme.cosmic().accent_color()` into an array. 

Caption: when `config.show_names`, build the label as a column:
```rust
        let mut lines = widget::column().spacing(0).push(widget::text(client.info.login.label()).size(13).class(text_ink));
        if config.show_status && let Some(line) = status::caption(&client.status, now) {
            let ink = if alarm { theme::Text::Color(with_opacity(Color { r: ALARM[0], g: ALARM[1], b: ALARM[2], a: 1.0 }, alpha)) } else { text_ink };
            lines = lines.push(widget::text(line).size(11).class(ink));
        }
        let label = widget::container(lines).padding([2, 6]).class(/* same dark pill */);
```
When `show_names` is false but `show_status` is true and a caption exists, show the caption line alone in the same pill (so the status is still visible).

- [ ] **Step 4: Run** `cargo test --bin yutani` → pass; `cargo build` → ok.
- [ ] **Step 5: Commit** `git add src/ui/thumbnail.rs && git commit -m "thumbnail: system/combat caption and alarm border"`.

---

### Task 9: Doctor line and docs

**Files:** `src/doctor.rs`, `README.md`, `docs/` (whichever page lists thumbnail settings; `grep -rl show_names docs README.md`).

- [ ] **Step 1:** In `doctor::run` after the GL status, before the verdict:
```rust
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let logs = match crate::eve_logs::discover(config.eve_logs_dir.as_deref().map(Path::new), &home) {
        Ok(dir) => {
            let chars = crate::eve_logs::tail::scan(&dir, std::time::SystemTime::now());
            format!("{} ({} character(s) with a fresh log)", dir.display(), chars.game.len().max(chars.local.len()))
        }
        Err(e) => format!("off: {e}"),
    };
    tolerate_broken_pipe(writeln!(out, "EVE logs: {logs}"))?;
```
`run` needs the config: check how `doctor` is invoked from `main.rs`/`cli.rs` and pass `&Config` (load it the way the daemon does, `Config::load()` or equivalent).

- [ ] **Step 2:** README: under "What it does", add a row: `🛰️ **Where and how they're doing** | Each caption shows the character's solar system, and turns red with the incoming dps and SCRAM / POINT / JAM / NEUT tags, straight from EVE's own log files.` Add `show_status` and `eve_logs_dir` to the config table if one exists.

- [ ] **Step 3:** `cargo test` (whole crate) → pass; `cargo build --release` → ok.
- [ ] **Step 4: Commit** `git add -A src docs README.md && git commit -m "doctor + docs: EVE log status"`.

---

### Task 10: Package, install, live check

- [ ] **Step 1:** `cd packaging && makepkg -sf` (memory: pkgver is computed by makepkg; commit the `pkgver` bump it writes, as the last commits do).
- [ ] **Step 2:** `pkexec pacman -U --noconfirm packaging/yutani-*.pkg.tar.zst` (newest file).
- [ ] **Step 3:** Restart the daemon: `systemctl --user restart yutani.service` (the applet needs no restart; never pkill cosmic-panel).
- [ ] **Step 4:** `journalctl --user -u yutani -n 20` → expect `EVE logs: /mnt/tank/.../Documents/EVE/logs`.
- [ ] **Step 5:** Look at the running clients' thumbnails: system name under each name; ask the user to take a hit / get a jump in and confirm the caption and the red border change within a second.
- [ ] **Step 6:** Commit the pkgver bump; push when the user says so.
