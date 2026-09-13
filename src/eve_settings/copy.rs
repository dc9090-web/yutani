//! Copying one character's settings over the others, with a backup first.
//!
//! The `.dat` files are CCP's opaque binary format: copied whole, never
//! merged. Every overwrite is temp-file + rename in the target's directory,
//! and `execute` runs in two phases — *every* target is copied into the
//! backup directory under its own name before *any* target is overwritten
//! — so `restore` is a plain copy back and a failed backup has changed
//! nothing at all.

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

/// What went wrong and how far the copy got: `replaced` is 0 when the
/// failure came before any target was touched (backup phase, or the
/// backup directory already existing), so the note can say so honestly.
#[derive(Debug)]
pub struct Failure {
    pub error: std::io::Error,
    pub replaced: usize,
    pub planned: usize,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
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

/// Write `from`'s bytes over `to` via a sibling temporary and a rename;
/// the temporary never survives a failure of either step.
fn replace_via_tmp(from: &Path, to: &Path) -> std::io::Result<()> {
    let tmp = to.with_extension("tmp");
    let result = std::fs::copy(from, &tmp).and_then(|_| std::fs::rename(&tmp, to));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Copy each of `files` into a freshly created `backup` directory under
/// its own name; returns how many. The directory must not exist yet — a
/// second run into the same directory would overwrite the originals it
/// holds with the files that already replaced them.
pub fn backup_files(files: &[PathBuf], backup: &Path) -> std::io::Result<usize> {
    if let Some(parent) = backup.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(backup).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            std::io::Error::new(e.kind(), format!("backup directory {} already exists", backup.display()))
        } else {
            e
        }
    })?;
    let mut saved = 0;
    for file in files {
        let name = file.file_name().ok_or_else(|| std::io::Error::other("target has no file name"))?;
        // The backup itself goes through the temporary too: a copy that
        // dies part-way must not leave a truncated file under the real
        // name, or a later restore would write it over a good original.
        replace_via_tmp(file, &backup.join(name))?;
        saved += 1;
    }
    Ok(saved)
}

/// Two phases, in this order: *every* target is copied into `backup`,
/// and only then is any target overwritten. Interleaving the two would
/// mean a backup that fails on the fourth target leaves the first three
/// already replaced with no way back; this way a phase-1 failure has
/// touched nothing (`Failure::replaced` is 0), and a phase-2 failure
/// leaves every original in the backup for `restore`.
pub fn execute(plan: &Plan, backup: &Path) -> Result<Report, Failure> {
    let planned = plan.targets.len();
    let files: Vec<PathBuf> = plan.targets.iter().map(|t| t.to.clone()).collect();
    backup_files(&files, backup).map_err(|error| Failure { error, replaced: 0, planned })?;
    let mut report = Report { characters: 0, accounts: 0, backup: backup.to_path_buf() };
    for target in &plan.targets {
        if let Err(error) = replace_via_tmp(&target.from, &target.to) {
            return Err(Failure { error, replaced: report.characters + report.accounts, planned });
        }
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

/// The files in `dir` a `restore` from `backup` would replace, sorted.
/// Pass this to [`backup_files`] to save the live files before a restore
/// overwrites them.
pub fn restore_plan(backup: &Path, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut targets = Vec::new();
    for entry in std::fs::read_dir(backup)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().and_then(parse_file_name).is_none() || !entry.path().is_file() {
            continue;
        }
        let to = dir.join(&name);
        // A restore reverts files; it does not create files the profile
        // never had. A character deleted from the profile since the
        // backup would otherwise come back from the dead.
        if !to.is_file() {
            continue;
        }
        targets.push(to);
    }
    targets.sort();
    Ok(targets)
}

/// Copy every settings file in `backup` back over the file of the same
/// name in `dir`; returns how many. Backup entries with no counterpart in
/// `dir` are skipped (see [`restore_plan`]).
pub fn restore(backup: &Path, dir: &Path) -> std::io::Result<usize> {
    let mut restored = 0;
    for entry in std::fs::read_dir(backup)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().and_then(parse_file_name).is_none() || !entry.path().is_file() {
            continue;
        }
        let to = dir.join(&name);
        if !to.is_file() {
            continue;
        }
        replace_via_tmp(&entry.path(), &to)?;
        restored += 1;
    }
    Ok(restored)
}

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
    fn a_backup_directory_is_never_reused() {
        let dir = profile("reuse");
        let listing = super::super::list(&dir).unwrap();
        let backup = dir.join("backups").join("20260913T024100Z");
        execute(&plan(&listing, 1, Some(10)).unwrap(), &backup).unwrap();

        let backup_files_after_first: std::collections::BTreeMap<_, _> = std::fs::read_dir(&backup)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name(), std::fs::read(e.path()).unwrap())
            })
            .collect();
        let profile_files_after_first: Vec<_> = ["core_char_1.dat", "core_char_2.dat", "core_char_3.dat", "core_user_10.dat", "core_user_20.dat"]
            .iter()
            .map(|n| std::fs::read(dir.join(n)).unwrap())
            .collect();

        let err = execute(&plan(&listing, 2, Some(20)).unwrap(), &backup).unwrap_err();
        assert_eq!(err.error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(err.replaced, 0, "refused before any target was touched");

        let backup_files_after_second: std::collections::BTreeMap<_, _> = std::fs::read_dir(&backup)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name(), std::fs::read(e.path()).unwrap())
            })
            .collect();
        assert_eq!(backup_files_after_first, backup_files_after_second, "backup untouched by the rejected second run");
        let profile_files_after_second: Vec<_> = ["core_char_1.dat", "core_char_2.dat", "core_char_3.dat", "core_user_10.dat", "core_user_20.dat"]
            .iter()
            .map(|n| std::fs::read(dir.join(n)).unwrap())
            .collect();
        assert_eq!(profile_files_after_first, profile_files_after_second, "profile untouched by the rejected second run");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failure while backing up must leave the profile exactly as it
    /// was: the second target cannot be backed up (its file does not
    /// exist), so not even the first target may have been replaced.
    #[test]
    fn a_failure_backing_up_leaves_every_profile_file_untouched() {
        let dir = profile("backup-fails");
        let entry = |name: &str| Entry {
            id: 0,
            path: dir.join(name),
            size: 4,
            modified: SystemTime::UNIX_EPOCH,
        };
        let source = entry("core_char_1.dat");
        let plan = Plan {
            source_character: source.clone(),
            source_account: None,
            // The second target has no file behind it, so phase 1 fails
            // on it — after the first target was already backed up but
            // before anything was overwritten.
            targets: vec![
                Target { from: source.path.clone(), to: dir.join("core_char_2.dat"), kind: Kind::Character },
                Target { from: source.path.clone(), to: dir.join("core_char_404.dat"), kind: Kind::Character },
            ],
        };
        let backup = dir.join("backups").join("20260913T024100Z");
        let err = execute(&plan, &backup).unwrap_err();
        assert_eq!(err.error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!((err.replaced, err.planned), (0, 2));
        assert_eq!(std::fs::read(dir.join("core_char_2.dat")).unwrap(), b"BBBB", "not overwritten");
        assert_eq!(std::fs::read(dir.join("core_char_1.dat")).unwrap(), b"AAAA", "source untouched");
        assert!(!dir.join("core_char_404.dat").exists());
        // No temporary survives the failure, in the profile or the backup.
        for parent in [&dir, &backup] {
            let leftovers: Vec<_> = std::fs::read_dir(parent)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .filter(|n| n.ends_with(".tmp"))
                .collect();
            assert!(leftovers.is_empty(), "{} has {leftovers:?}", parent.display());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_ignores_anything_that_is_not_a_settings_file() {
        let dir = tmpdir("restore-junk");
        let backup = dir.join("b");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(dir.join("core_char_5.dat"), b"stale").unwrap();
        std::fs::write(backup.join("core_char_5.dat"), b"five").unwrap();
        std::fs::write(backup.join("notes.txt"), b"nope").unwrap();
        std::fs::write(backup.join("core_char__.dat"), b"nope").unwrap();
        assert_eq!(restore_plan(&backup, &dir).unwrap(), vec![dir.join("core_char_5.dat")]);
        assert_eq!(restore(&backup, &dir).unwrap(), 1);
        assert_eq!(std::fs::read(dir.join("core_char_5.dat")).unwrap(), b"five");
        assert!(!dir.join("notes.txt").exists());
        assert!(!dir.join("core_char__.dat").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A restore reverts files; a character whose file is gone from the
    /// profile is not resurrected by the backup that still holds it.
    #[test]
    fn restore_skips_backup_files_the_profile_no_longer_has() {
        let dir = tmpdir("restore-gone");
        let backup = dir.join("b");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(dir.join("core_char_1.dat"), b"live").unwrap();
        std::fs::write(backup.join("core_char_1.dat"), b"old").unwrap();
        std::fs::write(backup.join("core_char_2.dat"), b"deleted").unwrap();
        assert_eq!(restore_plan(&backup, &dir).unwrap(), vec![dir.join("core_char_1.dat")]);
        assert_eq!(restore(&backup, &dir).unwrap(), 1);
        assert_eq!(std::fs::read(dir.join("core_char_1.dat")).unwrap(), b"old");
        assert!(!dir.join("core_char_2.dat").exists(), "not brought back from the dead");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What the Restore path does before it restores: save the live files
    /// into a fresh directory, refusing to reuse one.
    #[test]
    fn backup_files_saves_each_file_and_never_reuses_a_directory() {
        let dir = tmpdir("backup-files");
        std::fs::write(dir.join("core_char_1.dat"), b"one").unwrap();
        std::fs::write(dir.join("core_char_2.dat"), b"two").unwrap();
        let backup = dir.join("backups").join("20260913T024100Z");
        let files = vec![dir.join("core_char_1.dat"), dir.join("core_char_2.dat")];
        assert_eq!(backup_files(&files, &backup).unwrap(), 2);
        assert_eq!(std::fs::read(backup.join("core_char_1.dat")).unwrap(), b"one");
        assert_eq!(std::fs::read(backup.join("core_char_2.dat")).unwrap(), b"two");
        let err = backup_files(&files, &backup).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
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
