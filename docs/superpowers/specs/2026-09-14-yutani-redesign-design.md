# Yutani — 2026-09-14 redesign: icons, applet popover, settings window

**Status:** Daniel's design handoff (`design/New.zip`, README preserved as
`2026-09-14-yutani-redesign-handoff.md` — the source of truth for layout,
copy, sizes and states; this document records how it maps onto the
codebase and the decisions taken on 2026-09-14). Three surfaces, built and
checkpointed in this order: **icons → applet popover → settings window**.
Each surface gets its own plan under `docs/superpowers/plans/`.

The handoff's two hard rules hold everywhere: colours come from the COSMIC
theme roles, never from the mock's hexes (the applet's dark-only palette in
`applet::theme` is retired with the popover redesign); the tray icon is a
`currentColor` symbolic SVG installed into the icon theme.

## Decisions (2026-09-14, with Daniel)

- **Switch peer… is left out.** Yutani stores one tunnel config; multi-peer
  is a later project. The applet menu is Layouts & characters…,
  Preferences…, Quit.
- **The thumbnail opacity slider stays**, as a fourth row of the Display
  pane's Thumbnails card (below Hover zoom), in the handoff's row style;
  the live preview strip renders at that opacity.
- **Sequencing:** icons, then applet, then settings, each installed and
  looked at before the next starts.

## Assumptions (not asked; easy to reverse)

- **Frame colours:** the handoff's four swatches plus a fifth, "Accent",
  for the focused border only — it maps to `active_border: None` (follow
  the COSMIC accent), today's default, and the handoff itself says to read
  "accent" as the user's accent. Its default stays Accent; the inactive
  default stays the current value rather than the mock's `#1b2130`.
- **Ranges:** the sliders and steppers take the handoff's ranges (width
  160–640 step 16, zoom 1.0–2.5, border 0–8, radius 0–24 step 2, fps
  15/30/60). `Config::validate` keeps its wider limits so a hand-edited
  file is not reset; a value outside the control's range shows clamped and
  is written back only when the control is moved.
- **Shortcuts:** the chips are derived from the prefix as designed; the
  `next`/`prev` keys stay configurable in `config.ron` and the chips show
  the real configured key (`→`/`←` by default). A fourth binding,
  Show / hide thumbnails → `<prefix> + T` (`yutani toggle`), is added to
  `shortcuts::desired` and installed with the others. Prefix choices are
  the handoff's three.
- **Sync state:** the handoff has no "settling" icon; a pending
  connect/disconnect is shown as a hollow ring dot (ink stroke, panel
  fill) in the same corner as the status dots, so the badge language stays
  one thing. The 1.1 s spinner glyph is retired.
- **Popover width:** libcosmic's applet popup container pins its content
  to 360 (the previous handoff's 336 became 360 the same way); the content
  fills that and every inset is as designed, so nothing is drawn narrower
  than the surface.
- **Settings window:** opens at 900×724, resizable, minimum 760×560; the
  content pane scrolls, the sidebar does not.
- **`INSTALLED` / `CONFIG` facts on the Tunnel pane** come from the unit
  file's mtime and the `# yutani: label` line the daemon can read from the
  status reply (the conf itself is root-only); `—` when unknown.

## 1. Icons

Files in `assets/icons/`, all authored on the handoff's 32-unit grid:

| File | viewBox | Content | Installed as |
|---|---|---|---|
| `yutani-symbolic.svg` | 0 0 32 32 | two-piece mark, `fill="currentColor"` | `symbolic/status/yutani-symbolic.svg` |
| `yutani-symbolic-16.svg` | 0 0 32 32 | solid mark (no slice) | `16x16/status/yutani-symbolic.svg` |
| `yutani.svg` | 0 0 128 128 | launcher: plate, sheen, rim, extruded mark, drop shadow | `scalable/apps/yutani.svg` |
| `yutani-48.svg` | 0 0 128 128 | launcher without the slice and contour stroke | `48x48/apps/yutani.svg` |
| `yutani-32.svg` | 0 0 128 128 | as 48 | `32x32/apps/yutani.svg` |

The five old `y-*.svg` files are deleted; `applet install` and `uninstall`
also remove them from the theme (`LEGACY_ICONS`) so an upgraded install
leaves no stale files. Desktop entries: `Icon=yutani-symbolic` (applet),
`Icon=yutani` (launcher).

Panel button (`IconState`, `view::panel_button`): the applet draws the
symbolic bytes itself, the solid variant below 22 px. States, never
recolouring the glyph:

| State | Glyph | Badge (bottom-right, 8 px at 24, 1.5 px ring) |
|---|---|---|
| Plain (running; tunnel off, or on with nothing behind it) | ink, 100 % | none |
| Active (tunnel up, fresh handshake, ≥ 1 client) | ink | filled `success_color`, ring in the panel background |
| Attention (stale/no handshake past 180 s, unit failed) | ink | filled `warning_color`, same ring |
| Sync (connect/disconnect in flight, or settling) | ink | hollow: panel-background fill, ink ring |
| Dim (no daemon, or no tunnel installed) | ink, 40 % | none |

`icon_state` keeps its inputs and precedence (pending > failed > stale >
active > plain; missing daemon/tunnel = dim).

## 2. Applet popover

As handoff "Screen 1", less Switch peer…, on COSMIC theme roles
(`Container::Card` cards on the popup background, `Text::Default` at 70 %
/ 55 % for secondary / tertiary, `success_color` / `warning_color` /
`accent_color` / `destructive_color` from the theme). Data:

- **Header:** "Yutani" with the violet "Y" badge (the one fixed-colour
  token in the design, `#b9a6f5` on its tinted plate); state word and a
  toggler. On → `Action::StartDaemon`; off → `Action::Quit` (which already
  disconnects the tunnel first, giving the handoff's "stopping the service
  takes the tunnel idle with it"). While the daemon is off the popup has no
  `status`: accounts show "Service stopped", the tunnel section reads idle
  from `OfflineTunnel`, rates are 0, tiles `—`, the primary button is the
  inert "Start Yutani to route traffic".
- **Accounts card:** rows from `status.clients` (index chip = 1-based
  layout index = the hotkey digit), `focused` on the active row, click →
  `Focus(n)`. Footer hint from the real prefix — the daemon adds
  `shortcut_prefix` (display string, e.g. `Ctrl+Alt`) to the `status`
  reply — and the real count: `Ctrl+Alt+1…4 · Ctrl+Alt+←/→`; "hotkeys
  inactive" when stopped. Hide/Show thumbnails button → `Hide`/`Show`.
- **Tunnel status line:** dot + glow, "WireGuard", `tunnel.location`,
  uptime from `up_for_s` as `1h 12m 22s` / `idle`.
- **Throughput card:** rates from `Sampler` as today; a 34-sample history
  (`VecDeque`) pushed once per poll, each direction scaled against its own
  rolling max, bars drawn as fixed-height containers (no canvas), minimum
  1 px; totals from `tx_bytes`/`rx_bytes`; footer `60 s` (the popup polls
  at 1 Hz while open; while closed it polls at 5 s and the history is
  simply sparser — the label stays honest by pushing one sample per poll
  and the window being 34 polls).
- **Fact tiles:** `ENDPOINT` = exit address (fallback endpoint host), `PEER`
  = `tunnel.iface`.
- **Action row:** primary button per the handoff's three states; `⋯`
  opens the menu below (a popover inside the popup is not available in
  libcosmic's applet popup, so the menu is the always-visible list under a
  top border, as the mock's final section shows it).
- **Menu:** Layouts & characters… (→ `Settings` on the Layouts pane: the
  IPC `settings` request gains an optional page argument), Preferences…
  (→ Settings), Quit (`destructive` text).

The Sync/Attention icon logic, pending-action notes and the failed-poll
degrade behaviour are kept as they are.

## 3. Settings window

As handoff "Screen 2": header bar (COSMIC's, title "Yutani Settings", a
quiet `saved` in the header's end) → `nav_bar`-style sidebar (204 px, two
line items with live sublabels, footer "Every change saves as you make
it.") → scrolling content pane. `Page` and `PAGES` stay; the segmented
control goes. Sublabels: Layouts = `<n> saved`; Tunnel = `Connected ·
<location>` / `Installed · idle` / `Not installed`.

Per pane, what is new in the code:

- **Display:** live preview strip (two mock thumbnails at 42 % width, 16:9,
  live radius/border/colours/opacity/names); rows as designed plus the
  opacity row; steppers (`spin_button`) with the handoff's ranges; swatch
  rows (Accent + four) with hex readout; "Reset frame to defaults" → border
  1, radius 8, Accent, inactive default.
- **Behaviour:** mode as two described cards; Dock edge row rendered only
  when Docked; fps segmented 15/30/60 with the changing help text;
  visibility segmented; three toggles; prefix segmented (Ctrl + Alt / Super
  / Ctrl + Shift) and the derived chip grid; the `next`/`prev` inputs go.
- **Layouts:** rows with meta (`<n> monitors · <m> thumbnails · saved
  <date>` from the layout's distinct outputs, thumb count and file mtime),
  dot + `applied` (the last-applied name is remembered in `current.ron`'s
  sibling `last_layout` field — a new, optional field), Apply / Re-apply,
  `⋯` → Rename… (inline input) / Duplicate / Delete… (inline confirm);
  footer input + "Save current arrangement" (inert until named); info note.
- **Characters:** three numbered steps: source cards (`last played` from
  the entry's mtime via `relative_age`), what-gets-copied (interface always;
  account toggle default off), Apply (idle → confirm naming source, count,
  scope → done); Safety net: last backup humanised + Restore, profile
  folder + Change… (portal folder picker) + path.
- **Tunnel:** state card (headline/sub/primary/refresh, success-tinted
  border while connected, three facts); configuration card with the
  dashed drop zone (the existing whole-window drop keeps working; the zone
  is where it is explained), path input + Browse… + Install/Replace; the
  two notes; DNS card read-only; danger card with inline Uninstall confirm.
- **Steam:** three steps, one code block with Copy → "Copied" 1.6 s, the
  PATH toggle swapping the command to the absolute binary path
  (`current_exe`), closing note.

Transient UI state (`open_menu`, `renaming`, `confirm_delete`, `copy_phase`,
`uninstall_phase`, `copied_at`) lives in the settings `State`, is never
persisted, and is cleared on pane change. Persistence stays immediate;
sliders keep write-on-release.

## 3a. Deviations recorded while building part 3 (2026-09-14)

- **Tunnel facts:** the conf's file name is not known to the daemon (the
  installed conf is root-only), so the first fact is `LOCATION` (the
  conf's label, e.g. London) rather than `CONFIG`; `PRIVATE KEY` and
  `INSTALLED` (the unit file's mtime, local time) are as designed.
- **Show thumbnails** offers the two states the config has (Always / Only
  with EVE focused); the handoff's `Never` has no config field — hiding is
  the popover's Hide thumbnails action.
- **Layout `⋯` menu** opens inline under its row (libcosmic's applet-style
  popover is not available inside a toplevel window without an overlay
  manager); Rename… is inline in the row, Delete… arms the inline confirm.
- **Behaviour** has no Install / Uninstall shortcuts buttons: changing the
  prefix reinstalls the twelve bindings at once (the chips promise what
  the keys do). Custom next/prev keys stay honoured from `config.ron`.
- **Characters** copies the account file of the newest account when the
  toggle is on (the account picker is gone with the dropdowns).
- **Popover header** shows the Yutani symbolic mark, violet, centred in
  the plate (Daniel, 2026-09-14) rather than the letter Y.

## 4. Testing

Pure logic is unit-tested as the codebase does now: icon state → badge
mapping; the applet's history window and scaling; hotkey hint string;
uptime formatting; layout meta line; confirm sentences; command string for
the PATH toggle; prefix → chip derivation; install/uninstall file sets
including legacy cleanup. Views are checked by hand at each checkpoint.
