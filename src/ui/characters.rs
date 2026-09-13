//! The settings window's Characters page: copy one character's EVE
//! interface files over every other character's (spec
//! `docs/superpowers/specs/2026-09-13-yutani-character-copy-design.md`).
//!
//! The page's own state and view live here; the messages are
//! `settings::Msg` variants, and the file work happens in `App`
//! (`settings_copy_characters`), which is the only place that knows
//! whether an EVE client is running.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget;

use yutani::eve_settings::copy::{Failure, Report};
use yutani::eve_settings::names::Names;
use yutani::eve_settings::{Entry, Listing, relative_age};

use super::settings::Msg;

pub const RUNNING_CLIENT: &str = "Close every EVE client first: the client rewrites these files when it logs out.";
pub const ONE_CHARACTER: &str = "Only one character has settings here; there is nothing to copy to.";
pub const NO_PROFILE: &str = "No EVE profile directory was found.";
pub const NO_SELECTION: &str = "Pick a character to copy from.";
pub const NO_ACCOUNT_SELECTION: &str = "Pick an account to copy from.";
/// ESI answered, but not about every id we asked for; the caption has to
/// explain the bare numbers left in the dropdown.
pub const SOME_UNNAMED: &str = "some characters could not be named";

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
    /// Ids already sent to ESI. An id ESI has no name for stays here so
    /// that every refresh does not ask for it again.
    pub asked: BTreeSet<u64>,
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

    /// Character ids with no name yet and not already asked about — what a
    /// names fetch asks ESI for.
    pub fn unnamed(&self) -> Vec<u64> {
        self.listing
            .as_ref()
            .map(|l| {
                l.characters
                    .iter()
                    .map(|e| e.id)
                    .filter(|id| !self.names.contains_key(id) && !self.asked.contains(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remember the ids a fetch was started for, so a reply that leaves
    /// some of them nameless does not start the same fetch again.
    pub fn asking(&mut self, ids: &[u64]) {
        self.asked.extend(ids.iter().copied());
        self.fetching = true;
    }

    /// A names reply landed. When ESI reported no error but some id we
    /// asked about is still nameless, say so: the dropdown shows numbers
    /// and the caption is the only place that can explain them.
    pub fn names_arrived(&mut self, names: Names, error: Option<String>) {
        self.names.extend(names);
        self.fetching = false;
        // A failed lookup (offline, ESI down) is worth asking again: the
        // next Refresh retries those ids. Only a reply that succeeded and
        // still left an id nameless is final.
        if error.is_some() {
            self.asked.retain(|id| self.names.contains_key(id));
        }
        let unanswered = self.listing.as_ref().is_some_and(|l| {
            l.characters.iter().any(|e| self.asked.contains(&e.id) && !self.names.contains_key(&e.id))
        });
        self.names_error = error.or_else(|| unanswered.then(|| SOME_UNNAMED.to_string()));
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
    // A dropdown index past the end of the listing (libcosmic publishes one
    // on ctrl+scroll, and a refresh can shrink the list under a queued
    // message): without this the copy would be a silent no-op.
    if state.selected_character().is_none() {
        return Some(NO_SELECTION);
    }
    if clients_running {
        return Some(RUNNING_CLIENT);
    }
    None
}

/// A settings file written this close to `now` is one a client may still
/// be writing: EVE flushes `core_char_*`/`core_user_*` on logout and again
/// while the process exits, after its window is already gone.
pub const RECENT_WRITE_WINDOW: Duration = Duration::from_secs(3);

/// EVE processes owned by `uid` under `proc_root` (`/proc`), as
/// `(pid, name)` sorted by pid. Matched on `comm` or argv[0] the way
/// `adopt::scan` does, but without its slice filter: a client launched
/// through `yutani launch` is in the slice and is exactly the one whose
/// exit we are waiting for.
pub fn running_eve_processes(proc_root: &Path, patterns: &[String], uid: u32) -> Vec<(u32, String)> {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir(proc_root) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != uid {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let argv0 = cmdline.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        let name = if crate::adopt::is_eve_process(comm.trim(), patterns) {
            comm.trim().to_string()
        } else if crate::adopt::is_eve_process(&argv0, patterns) {
            argv0.rsplit(['/', '\\']).next().unwrap_or(&argv0).to_string()
        } else {
            continue;
        };
        out.push((pid, name));
    }
    out.sort();
    out
}

/// The listed file (character or account) whose mtime is within
/// [`RECENT_WRITE_WINDOW`] of `now`, the newest when several are. Measured
/// in both directions: a file a second in the future is a write with a
/// slightly-ahead clock, a file minutes in the future is skew, not a write.
pub fn recent_write(listing: &Listing, now: SystemTime) -> Option<&Entry> {
    listing
        .characters
        .iter()
        .chain(&listing.accounts)
        .filter(|e| match now.duration_since(e.modified) {
            Ok(age) => age < RECENT_WRITE_WINDOW,
            Err(ahead) => ahead.duration() < RECENT_WRITE_WINDOW,
        })
        .max_by_key(|e| e.modified)
}

/// Why a copy or restore must not start *right now*, as a note-line
/// fragment. Checked on the button press, not per frame: the toplevel
/// list (`clients_running`) empties the moment a client's window closes,
/// and `exefile.exe` writes these files while it is still tearing down
/// after that. Two independent conditions, each named so the user knows
/// what to wait for: a matching process still alive (a `/proc` walk, a
/// few ms), and a listed file written within [`RECENT_WRITE_WINDOW`] — its
/// mtime re-read from disk, since the listing the page holds can be
/// minutes old.
pub fn write_blocker(listing: &Listing, patterns: &[String], proc_root: &Path, uid: u32, now: SystemTime) -> Option<String> {
    if let Some((pid, name)) = running_eve_processes(proc_root, patterns, uid).first() {
        return Some(format!("{name} (pid {pid}) is still running; wait for it to exit and press again"));
    }
    let fresh = yutani::eve_settings::list(&listing.dir).unwrap_or_else(|_| listing.clone());
    let entry = recent_write(&fresh, now)?;
    let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Some(format!("{name} was written a moment ago, a client may still be writing it; wait a few seconds and press again"))
}

/// [`write_blocker`] against the real `/proc`, our uid and the clock.
pub fn write_blocker_now(listing: &Listing, patterns: &[String]) -> Option<String> {
    write_blocker(listing, patterns, Path::new("/proc"), crate::ipc::uid(), SystemTime::now())
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

/// The note for a failed `copy::execute`. Everything that goes wrong
/// while the backup is being taken — including the refusal to reuse a
/// backup directory after two presses inside one second — happens before
/// any file is replaced, so `replaced == 0` and the "already replaced"
/// half of the usual note would be a lie.
pub fn copy_failure_note(failure: &Failure, backup: &Path) -> String {
    if failure.replaced == 0 {
        let mut note = format!("copy not started: {}", failure.error);
        if failure.error.kind() == std::io::ErrorKind::AlreadyExists {
            note.push_str(" — wait a second and press again");
        }
        return note;
    }
    format!(
        "copy failed after replacing {} of {} files: {}; the originals are in {}",
        failure.replaced,
        failure.planned,
        failure.error,
        backup.display()
    )
}

/// A blocker constant reworded as a note-line fragment. The constants are
/// captions under a button — capitalised, full sentences — and the note
/// line is a lowercase phrase, so pushing one through verbatim reads wrong.
pub fn blocker_note(blocker: &str) -> String {
    match blocker {
        RUNNING_CLIENT => "close every EVE client first",
        ONE_CHARACTER => "only one character has settings here",
        NO_PROFILE => "no EVE profile directory found",
        NO_SELECTION => "pick a character to copy from",
        NO_ACCOUNT_SELECTION => "pick an account to copy from",
        other => other,
    }
    .to_string()
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
        widget::button::suggested("Copy to all characters")
            .on_press_maybe(blocker.is_none().then_some(Msg::CopyCharacters))
            .into(),
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
            // The same reason the Copy button gives: a running client
            // would rewrite the restored files the moment it logs out.
            if clients_running {
                backups = backups.add(widget::text::caption(RUNNING_CLIENT));
            }
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

    /// An index past the end of the listing used to pass the blocker and
    /// then fall out of the copy with no note at all.
    #[test]
    fn a_stale_dropdown_index_blocks_the_copy_instead_of_doing_nothing() {
        let mut s = state_with(&[1, 2], &[10]);
        s.source_character = 5;
        assert_eq!(s.selected_character(), None);
        assert_eq!(copy_blocker(&s, false), Some(NO_SELECTION));
    }

    /// Index 0 into an empty listing is still nothing: no panic, no
    /// selection, and the copy is blocked by the character count.
    #[test]
    fn an_empty_listing_selects_nothing() {
        let s = state_with(&[], &[]);
        assert_eq!(s.selected_character(), None);
        assert_eq!(s.selected_account(), None);
        assert_eq!(s.unnamed(), Vec::<u64>::new());
        assert_eq!(copy_blocker(&s, false), Some(ONE_CHARACTER));
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
        // The guard behind the clamp: an index past the end names no
        // account, and the copy asks for one rather than skipping it.
        s.source_account = 7;
        assert_eq!(s.account_to_copy(), None, "out of range: no account");
    }

    #[test]
    fn the_note_counts_what_was_copied() {
        let report = Report { characters: 6, accounts: 2, backup: PathBuf::from("/b/20260913T024100Z") };
        assert_eq!(copy_note("KestrelVance", &report), "copied KestrelVance to 6 characters and 2 accounts; backup in /b/20260913T024100Z");
        let report = Report { characters: 1, accounts: 0, backup: PathBuf::from("/b/x") };
        assert_eq!(copy_note("KestrelVance", &report), "copied KestrelVance to 1 character; backup in /b/x");
        let report = Report { characters: 3, accounts: 1, backup: PathBuf::from("/b/x") };
        assert_eq!(copy_note("KestrelVance", &report), "copied KestrelVance to 3 characters and 1 account; backup in /b/x");
    }

    /// Everything that fails during the backup phase happens before any
    /// file is touched, so the note must not claim files were replaced.
    #[test]
    fn a_refused_copy_does_not_claim_files_were_replaced() {
        let backup = PathBuf::from("/b/20260913T024100Z");
        let failure = |kind, msg: &str, replaced| Failure {
            error: std::io::Error::new(kind, msg.to_string()),
            replaced,
            planned: 7,
        };
        assert_eq!(
            copy_failure_note(
                &failure(std::io::ErrorKind::AlreadyExists, "backup /b/20260913T024100Z exists", 0),
                &backup
            ),
            "copy not started: backup /b/20260913T024100Z exists — wait a second and press again"
        );
        // A backup that broke for any other reason: nothing was replaced
        // either, and there is no "wait and press again" to offer.
        assert_eq!(
            copy_failure_note(&failure(std::io::ErrorKind::PermissionDenied, "read-only", 0), &backup),
            "copy not started: read-only"
        );
        assert_eq!(
            copy_failure_note(&failure(std::io::ErrorKind::PermissionDenied, "read-only", 3), &backup),
            "copy failed after replacing 3 of 7 files: read-only; the originals are in /b/20260913T024100Z"
        );
    }

    /// The blocker constants are button captions; the note line wants a
    /// lowercase fragment, not a capitalised sentence mid-sentence.
    #[test]
    fn blocker_notes_read_as_note_line_fragments() {
        assert_eq!(blocker_note(RUNNING_CLIENT), "close every EVE client first");
        assert_eq!(blocker_note(ONE_CHARACTER), "only one character has settings here");
        assert_eq!(blocker_note(NO_PROFILE), "no EVE profile directory found");
        assert_eq!(blocker_note(NO_SELECTION), "pick a character to copy from");
        assert_eq!(blocker_note(NO_ACCOUNT_SELECTION), "pick an account to copy from");
        for constant in [RUNNING_CLIENT, ONE_CHARACTER, NO_PROFILE, NO_SELECTION, NO_ACCOUNT_SELECTION] {
            let note = blocker_note(constant);
            assert!(!note.starts_with(|c: char| c.is_uppercase()), "{note}");
            assert!(!note.ends_with('.'), "{note}");
        }
    }

    #[test]
    fn ids_still_unnamed_are_the_ones_to_fetch() {
        let mut s = state_with(&[1, 2, 3], &[]);
        s.names.insert(2, "Two".to_string());
        assert_eq!(s.unnamed(), vec![1, 3]);
        // ESI had no name for 1: asking again every refresh would be a
        // request per redraw, forever.
        s.asking(&[1, 3]);
        assert_eq!(s.unnamed(), Vec::<u64>::new());
        s.names_arrived(Names::from([(3, "Three".to_string())]), None);
        assert_eq!(s.names_error.as_deref(), Some(SOME_UNNAMED), "1 came back nameless");
        assert!(!s.fetching);
        s.names_arrived(Names::from([(1, "One".to_string())]), None);
        assert_eq!(s.names_error, None, "every asked id has a name now");
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-characters-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fake `/proc`: `<pid>/comm` and `<pid>/cmdline` under `root`.
    fn fake_process(root: &Path, pid: u32, comm: &str, argv: &[&str]) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        let mut cmdline = Vec::new();
        for a in argv {
            cmdline.extend_from_slice(a.as_bytes());
            cmdline.push(0);
        }
        std::fs::write(dir.join("cmdline"), cmdline).unwrap();
    }

    fn patterns() -> Vec<String> {
        vec!["exefile.exe".into(), "eve-online.exe".into()]
    }

    /// The window is gone but `exefile.exe` is not: the process walk sees
    /// it by `comm` or by argv[0], for our uid only, in or out of the slice.
    #[test]
    fn running_eve_processes_are_found_by_comm_or_argv0_for_our_uid() {
        let root = tmpdir("proc");
        let uid = crate::ipc::uid();
        fake_process(&root, 300, "wineserver", &["wineserver"]);
        fake_process(&root, 100, "exefile.exe", &["C:\\CCP\\EVE\\tq\\bin64\\exefile.exe"]);
        fake_process(&root, 200, "wine64-preloader", &["/pfx/drive_c/EVE/eve-online.exe", "--x"]);
        std::fs::create_dir_all(root.join("self")).unwrap();
        std::fs::write(root.join("self/comm"), "exefile.exe\n").unwrap();
        assert_eq!(
            running_eve_processes(&root, &patterns(), uid),
            vec![(100, "exefile.exe".to_string()), (200, "eve-online.exe".to_string())]
        );
        assert_eq!(running_eve_processes(&root, &patterns(), uid + 1), vec![], "another user's client is not ours");
        assert_eq!(running_eve_processes(&root.join("nowhere"), &patterns(), uid), vec![]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A file written inside the window is one a client may still be
    /// writing; a clock-skewed future mtime outside it is not.
    #[test]
    fn a_file_written_a_moment_ago_is_a_recent_write() {
        let now = SystemTime::now();
        let mut listing = Listing { dir: PathBuf::from("/p"), characters: vec![entry(1, 60, now), entry(2, 1, now)], accounts: vec![] };
        assert_eq!(recent_write(&listing, now).map(|e| e.id), Some(2));
        listing.characters[1].modified = now - Duration::from_secs(3);
        assert_eq!(recent_write(&listing, now), None, "the window is 3 s");
        listing.accounts.push(entry(10, 2, now));
        assert_eq!(recent_write(&listing, now).map(|e| e.id), Some(10), "accounts count too");
        listing.accounts[0].modified = now + Duration::from_secs(120);
        assert_eq!(recent_write(&listing, now), None, "far in the future: clock skew, not a write");
        listing.accounts[0].modified = now + Duration::from_secs(1);
        assert_eq!(recent_write(&listing, now).map(|e| e.id), Some(10), "just in the future: a write");
    }

    /// Both conditions on the button press, each named: the process first,
    /// then a fresh write — re-read from disk, since the listing the page
    /// holds can be minutes old.
    #[test]
    fn the_press_is_refused_while_a_client_runs_or_just_wrote_a_file() {
        let root = tmpdir("press");
        let (proc_root, profile) = (root.join("proc"), root.join("profile"));
        std::fs::create_dir_all(&proc_root).unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("core_char_1.dat"), b"~").unwrap();
        std::fs::write(profile.join("core_char_2.dat"), b"~").unwrap();
        let uid = crate::ipc::uid();
        let now = SystemTime::now();
        // The listing says both files are an hour old; the disk says now.
        let stale = Listing {
            dir: profile.clone(),
            characters: vec![entry(1, 3600, now), entry(2, 3600, now)],
            accounts: vec![],
        };
        let note = write_blocker(&stale, &patterns(), &proc_root, uid, now).expect("fresh on disk");
        assert!(note.contains("core_char_") && note.contains("a moment ago") && note.contains("press again"), "{note}");
        assert!(!note.starts_with(|c: char| c.is_uppercase()) && !note.ends_with('.'), "{note}");

        fake_process(&proc_root, 4242, "exefile.exe", &["exefile.exe"]);
        let note = write_blocker(&stale, &patterns(), &proc_root, uid, now).expect("process alive");
        assert!(note.contains("exefile.exe") && note.contains("4242") && note.contains("press again"), "{note}");
        assert!(!note.starts_with(|c: char| c.is_uppercase()) && !note.ends_with('.'), "{note}");

        std::fs::remove_dir_all(proc_root.join("4242")).unwrap();
        let old = now - Duration::from_secs(60);
        for name in ["core_char_1.dat", "core_char_2.dat"] {
            std::fs::File::open(profile.join(name)).unwrap().set_modified(old).unwrap();
        }
        assert_eq!(write_blocker(&stale, &patterns(), &proc_root, uid, now), None, "no process, nothing fresh");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Offline at first open must not mean numbers forever: a reply that
    /// carries an error releases the nameless ids so Refresh asks again.
    #[test]
    fn a_failed_lookup_is_retried_on_the_next_refresh() {
        let mut s = state_with(&[1, 2], &[]);
        s.asking(&[1, 2]);
        assert_eq!(s.unnamed(), Vec::<u64>::new());
        s.names_arrived(Names::from([(2, "Two".to_string())]), Some("curl failed".to_string()));
        assert_eq!(s.names_error.as_deref(), Some("curl failed"));
        assert_eq!(s.unnamed(), vec![1], "1 is asked again, 2 is named");
        assert!(!s.fetching);
    }
}
