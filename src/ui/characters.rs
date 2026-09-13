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
