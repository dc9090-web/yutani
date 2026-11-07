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

type Parse = fn(&str) -> Option<(EveTime, EventKind)>;

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
    let (text, used) = match enc {
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
        let offset = match enc {
            Encoding::Utf16Le => len & !1,
            Encoding::Utf8 => len,
        };
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
    let mut bytes = std::fs::read(path).ok()?;
    // Make the final line complete so it is decoded too.
    match enc {
        Encoding::Utf8 => bytes.push(b'\n'),
        Encoding::Utf16Le => bytes.extend_from_slice(&[0x0A, 0]),
    }
    Some(decode_complete(&bytes, enc).0)
}

/// The `Listener:` name from the header block, if the file has one.
pub fn listener(path: &Path, enc: Encoding) -> Option<String> {
    let lines = read_all(path, enc)?;
    lines
        .iter()
        .take(HEADER_LINES)
        .find_map(|l| l.trim().strip_prefix("Listener:").map(|n| n.trim().to_string()))
        .filter(|n| !n.is_empty())
}

/// The last event in the file that `parse` accepts (backfill on first open).
pub fn last_event(path: &Path, enc: Encoding, parse: Parse) -> Option<EventKind> {
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
            .filter(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| now.duration_since(m).is_ok_and(|age| age <= FRESH))
            })
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

/// Only the navigation events are worth backfilling from a game log.
fn game_system_line(l: &str) -> Option<(EveTime, EventKind)> {
    parse::game_line(l).filter(|(_, k)| matches!(k, EventKind::System(_) | EventKind::Undocked(_)))
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
        self.game.retain(|p, _| found.game.values().any(|w| w == p));
        self.local.retain(|p, _| found.local.values().any(|w| w == p));
        for (character, path) in &found.local {
            if self.local.contains_key(path) {
                continue;
            }
            if let Ok(tail) = Tail::at_end(path.clone(), Encoding::Utf16Le) {
                if let Some(kind) = last_event(path, Encoding::Utf16Le, parse::chat_line) {
                    events.push(LogEvent { character: character.clone(), at: EveTime(0), kind });
                }
                self.local.insert(path.clone(), Tracked { character: character.clone(), tail });
            }
        }
        for (character, path) in &found.game {
            if self.game.contains_key(path) {
                continue;
            }
            if let Ok(tail) = Tail::at_end(path.clone(), Encoding::Utf8) {
                let have_system = events.iter().any(|e| &e.character == character);
                if !have_system && let Some(kind) = last_event(path, Encoding::Utf8, game_system_line) {
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
        let game = self.game.values_mut().map(|t| (t, parse::game_line as Parse));
        let local = self.local.values_mut().map(|t| (t, parse::chat_line as Parse));
        for (tracked, parse) in game.chain(local) {
            let lines = match tracked.tail.read_new() {
                Ok(lines) => lines,
                Err(e) => {
                    tracing::debug!("eve log {}: {e}", tracked.tail.path.display());
                    continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eve_logs::EventKind;
    use std::io::Write as _;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("yutani-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

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
        let (l, u) = decode_complete(&bytes[..3], Encoding::Utf16Le);
        assert!(l.is_empty() && u == 0);
    }

    #[test]
    fn tail_returns_only_new_complete_lines() {
        let dir = tmp("tail");
        let path = dir.join("t.txt");
        std::fs::write(&path, "old\r\n").unwrap();
        let mut t = Tail::at_end(path.clone(), Encoding::Utf8).unwrap();
        assert_eq!(t.read_new().unwrap(), Vec::<String>::new());
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"new1\r\nhalf").unwrap();
        assert_eq!(t.read_new().unwrap(), vec!["new1"]);
        f.write_all(b"-done\r\n").unwrap();
        assert_eq!(t.read_new().unwrap(), vec!["half-done"]);
        std::fs::write(&path, "x\r\n").unwrap();
        assert_eq!(t.read_new().unwrap(), vec!["x"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listener_and_backfill_from_real_headers() {
        let dir = tmp("hdr");
        let game = dir.join("g.txt");
        std::fs::write(&game, "------------------------------------------------------------\r\n  Gamelog\r\n  Listener: TrilliumONE\r\n  Session Started: 2026.11.07 02:25:00\r\n------------------------------------------------------------\r\n[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora\r\n[ 2026.11.07 09:18:12 ] (None) Jumping from Akora to Oijanen\r\n[ 2026.11.07 09:18:13 ] (notify) Autopilot engaged\r\n").unwrap();
        assert_eq!(listener(&game, Encoding::Utf8).as_deref(), Some("TrilliumONE"));
        assert_eq!(last_event(&game, Encoding::Utf8, parse::game_line), Some(EventKind::System("Oijanen".into())));
        let stub = dir.join("stub.txt");
        std::fs::write(&stub, "------------------------------------------------------------\r\n  Gamelog\r\n  Session Started: 2026.11.07 06:08:30\r\n------------------------------------------------------------\r\n").unwrap();
        assert_eq!(listener(&stub, Encoding::Utf8), None);
        let local = dir.join("l.txt");
        std::fs::write(&local, utf16("\u{feff}\r\n\r\n        ---------------------------------------------------------------\r\n          Channel ID:      local\r\n          Channel Name:    Local\r\n          Listener:        TrilliumONE\r\n          Session started: 2026.11.07 02:25:00\r\n        ---------------------------------------------------------------\r\n\u{feff}[ 2026.11.07 09:19:24 ] EVE System > Channel changed to Local : Airaken\r\n\u{feff}[ 2026.11.07 09:20:47 ] EVE System > Channel changed to Local : Korsiki\r\n")).unwrap();
        assert_eq!(listener(&local, Encoding::Utf16Le).as_deref(), Some("TrilliumONE"));
        assert_eq!(last_event(&local, Encoding::Utf16Le, parse::chat_line), Some(EventKind::System("Korsiki".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_keeps_the_newest_file_per_character_and_skips_stale_and_stubs() {
        let dir = tmp("scan");
        std::fs::create_dir_all(dir.join("Gamelogs")).unwrap();
        std::fs::create_dir_all(dir.join("Chatlogs")).unwrap();
        let hdr = |n: &str| format!("---\r\n  Gamelog\r\n  Listener: {n}\r\n  Session Started: x\r\n---\r\n");
        std::fs::write(dir.join("Gamelogs/20261107_010000_1.txt"), hdr("Alpha")).unwrap();
        std::fs::write(dir.join("Gamelogs/20261107_020000_1.txt"), hdr("Alpha")).unwrap();
        std::fs::write(dir.join("Gamelogs/20261107_020000.txt"), "---\r\n  Gamelog\r\n  Session Started: x\r\n---\r\n").unwrap();
        std::fs::write(dir.join("Chatlogs/Local_20261107_020000_1.txt"), utf16("          Listener:        Alpha\r\n")).unwrap();
        std::fs::write(dir.join("Chatlogs/Corp_20261107_020000_1.txt"), utf16("          Listener:        Alpha\r\n")).unwrap();
        let now = SystemTime::now();
        let s = scan(&dir, now);
        assert_eq!(s.game.get("Alpha"), Some(&dir.join("Gamelogs/20261107_020000_1.txt")));
        assert_eq!(s.local.get("Alpha"), Some(&dir.join("Chatlogs/Local_20261107_020000_1.txt")));
        assert_eq!(s.game.len(), 1);
        let later = now + Duration::from_secs(2 * 3600);
        assert!(scan(&dir, later).game.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reader_backfills_system_then_streams_events() {
        let dir = tmp("reader");
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
        assert!(r.rescan().is_empty(), "a second scan must not re-backfill");
        let mut f = std::fs::OpenOptions::new().append(true).open(&game).unwrap();
        f.write_all(b"[ 2026.11.07 09:17:00 ] (combat) You're jammed by Guristas Saboteur - Guristas Saboteur\r\n").unwrap();
        let ev = r.poll();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, EventKind::Jammed("Guristas Saboteur".into()));
        assert_eq!(r.characters(), vec!["Alpha".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
