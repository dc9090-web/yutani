# Plan — redesign part 3: settings window

Spec: `docs/superpowers/specs/2026-09-14-yutani-redesign-design.md` §3;
layout, copy and sizes in `…-redesign-handoff.md` "Screen 2". TDD on the
pure parts; never `cargo fmt`. Checkpoint by install at the end.

## Stage A — foundations
- `model::date`: local-time `short_date` (`12 Sep`) and `date_time`
  (`13 Sep 2026, 05:38`) via `libc::localtime_r`.
- `layout::summaries_in(dir)` → name, distinct monitors, thumbnail count,
  saved time; `Layout.last_applied` (`serde(default)`) remembered on
  Apply / Save.
- `shortcuts::desired` adds `<prefix>+T` → `yutani toggle`.
- `ui::settings_ui`: the window's building blocks on COSMIC theme roles —
  text helpers, cards, section labels, rows (label / help / control),
  choice pills, toggles, steppers, swatches, chips, tinted panels, info
  note, sidebar items and footer; the handoff's sizes as consts.
- `settings::State` gains the transient UI state (layout menu / rename /
  delete-confirm, applied layout, copy phase, uninstall confirm, Steam
  copied-at and PATH toggle) and the layout summaries; `Msg` gains the
  messages for them plus `ResetFrame`, `ChangeProfileDir`,
  `ProfileDirChosen`. Prefix changes reinstall the shortcuts; the key
  fields and the Install/Uninstall buttons go.
- Daemon handlers in `ui/mod.rs` for the new messages (duplicate, rename
  by draft, delete after confirm, folder chooser, copied-at reset,
  applied layout persisted).

## Stage B — window chrome + Display, Behaviour, Layouts, Steam
- Header bar with the quiet `saved` (or the note) at its end; 204 px
  sidebar with live sublabels and the footer card; scrolling content
  pane; 900×724, min 760×560.
- Panes as the handoff describes them, opacity as a fourth Thumbnails row,
  Accent as a fifth focused-border swatch.

## Stage C — Tunnel + Characters
- Tunnel: state card (facts: location, private key, installed date),
  configuration card (drop zone + path + Browse… + Install/Replace, the
  two notes), DNS card, danger card with inline Uninstall confirm.
- Characters: three numbered steps, Safety net (humanised last backup +
  Restore, profile folder + Change…).

## Ship
`cargo test`, release build, install (pkexec), restart the daemon;
checkpoint with Daniel.
