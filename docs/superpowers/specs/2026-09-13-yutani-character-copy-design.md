# Yutani — copy one character's EVE interface settings to every other character

**Status:** approved by Daniel 2026-09-13 ("yup lets do it"); implementation plan
`docs/superpowers/plans/2026-09-13-yutani-character-copy.md`.

## 1. Goal

A "Characters" page in the settings window with one job: pick a character,
press one button, and every other character in the EVE profile gets that
character's overview presets, window positions and sizes, chat setup and
UI layout. Optionally the account-level settings (keyboard shortcuts,
general, graphics, audio) are copied from one account to the others too.

## 2. What EVE stores, and where

EVE keeps per-profile settings under its Wine prefix. For the Steam install
(app id 8500) that is

```
<steam library>/steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE/<server dir>/<profile dir>/
```

where `<server dir>` is `c_ccp_eve_tq_tranquility` (Tranquility) and
`<profile dir>` is `settings_Default` unless the launcher was told to use
another profile. Steam libraries are listed in
`~/.local/share/Steam/config/libraryfolders.vdf` (`"path"` keys). Daniel's
is `/mnt/games/SteamLibrary`.

Inside the profile directory:

| File | Holds | Copied |
|---|---|---|
| `core_char_<characterID>.dat` | everything per character: overview, window positions/sizes, chat, UI layout | yes — this is the feature |
| `core_user_<accountID>.dat` | per account: shortcuts, general, graphics, audio | optional |
| `core_char__.dat`, `core_user__.dat`, `core_char_('char', None, 'dat').dat` | junk EVE writes on the login screen | never touched |
| `core_public__.yaml`, `prefs.ini`, `Browser/` | not settings | never touched |

The `.dat` files are CCP's binary "blue" marshal format (first byte `0x7e`).
They are opaque: copied whole, never merged or edited. Which account a
character belongs to is not recorded anywhere on disk (the ids are not in
the account files as text, and file times are ambiguous when two clients
log out together), so the account copy needs its own source picker.

## 3. Behaviour

- **Discovery.** `config.ron` may set `eve_settings_dir` (absolute path) to
  point straight at a profile directory. Otherwise every Steam library from
  `libraryfolders.vdf` (in `~/.local/share/Steam`, `~/.steam/steam`, or the
  Flatpak Steam) is checked for the EVE prefix; the first `*_tranquility`
  server directory wins, and inside it `settings_Default`, else the first
  `settings_*`. No profile found → the page says so and names the config key.
- **Listing.** Only files matching exactly `core_char_<digits>.dat` and
  `core_user_<digits>.dat` count. Sorted by last-modified, newest first.
- **Names.** Character ids are resolved to names through ESI's public
  `POST https://esi.evetech.net/latest/universe/names/` (no auth), via
  `curl` on the blocking pool, and cached in
  `~/.config/yutani/characters.ron` so the page is instant on every later
  open and works offline. An id ESI cannot name is shown as its number.
  Account ids have no public name; they are shown as `account <id>` with
  a relative last-used time.
- **Copy.** "Copy to all characters" copies the chosen `core_char` file
  over every other `core_char_<digits>.dat`; with "Also copy account
  settings" on, the chosen `core_user` file over every other
  `core_user_<digits>.dat`. Each overwrite is a write-to-temp-and-rename in
  the same directory. The source files are never modified. The copy runs in
  two phases: *every* target is first copied into the backup directory, and
  only when all of them are safely there is *any* target overwritten.
  Interleaving the two would mean a backup that fails on the fourth target
  leaves the first three already replaced with no way back.
- **Safety.** The button is disabled while any EVE client toplevel exists
  (login screen included): the client rewrites these files on logout and
  would clobber the copy. EVE writes these files *as it exits*, so wait a
  moment after closing the last client before pressing Copy — otherwise the
  copy is of a file the client is still finishing. Before anything is
  overwritten, every target file is copied to
  `~/.local/share/yutani/backups/<UTC timestamp>/` under its own name; the
  directory must not already exist, so two presses inside one second are
  refused rather than letting the second run overwrite the first run's
  originals. "Restore last backup" copies them back — and, because a
  restore is otherwise the one step with no way back, it first backs up the
  live files it is about to replace into a new timestamped directory, and
  skips any backup entry whose file is no longer in the profile (a restore
  reverts files; it does not resurrect deleted ones). Fewer than two
  characters → nothing to copy, button disabled with the reason.
- **Feedback.** The note line says what happened: `copied KestrelVance to 6
  characters and 2 accounts; backup in …/backups/20260913T024100Z`, or
  `restored 6 files from …/20260913T024100Z; the files it replaced are in
  …/20260913T031500Z`. A failure says how far it got, honestly: nothing was
  touched during the backup phase, so that note is `copy not started: …`,
  while a failure while overwriting is `copy failed after replacing 3 of 7
  files: …; the originals are in …`. Reasons a button is disabled are shown
  as captions under it, and repeated in the note line (lowercased) when the
  press is refused after the fact.

## 4. Out of scope

Merging parts of a file (overview only, windows only); non-Steam installs
without the config override; the launcher's own profile switching; renaming
or deleting characters' files.
