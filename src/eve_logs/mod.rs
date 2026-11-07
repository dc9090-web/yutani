//! EVE's game and chat logs, read live for the thumbnails' status line
//! (spec: `docs/superpowers/specs/2026-11-07-eve-log-status-design.md`).
//!
//! [`parse`] turns one line into an event, [`tail`] follows the files,
//! [`watch`] is the daemon's subscription. Shield, armour and hull are not
//! in the logs; "under fire" is derived from incoming hits.

pub mod parse;
pub mod systems;
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
        return if dir.is_dir() {
            Ok(dir.to_path_buf())
        } else {
            Err(format!("eve_logs_dir {} is not a directory", dir.display()))
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eve_time_parses_the_log_stamp() {
        // date -u -d '2026-11-07 09:09:53' +%s
        assert_eq!(EveTime::parse("2026.11.07 09:09:53"), Some(EveTime(1794042593)));
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
