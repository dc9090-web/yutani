//! EVE's per-profile settings files (spec: `docs/superpowers/specs/2026-09-13-yutani-character-copy-design.md`).
//!
//! Discovery of the profile directory, listing of the per-character and
//! per-account files, and nothing else: the copy lives in [`copy`], the
//! names in [`names`]. Everything takes explicit paths so it runs against
//! a temp directory in tests.

pub mod copy;
pub mod names;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// The settings files in `dir`, newest first. A symlinked core file is
/// skipped on purpose: the copy replaces a file by renaming a temporary
/// over it, which would replace the user's link with a plain file.
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
