# Handoff: Yutani — panel applet, settings window, and icon set

## Overview

Yutani is a COSMIC-desktop companion for EVE Online multiboxing: live thumbnails of every EVE
client as a layer-shell overlay, global hotkeys to focus clients, a panel applet, an optional
WireGuard tunnel that carries EVE's traffic only, and a settings window.

This package covers three redesigned surfaces:

1. **Panel applet popover** — the dropdown from the tray icon. Service state, connected accounts,
   live tunnel throughput, one primary action.
2. **Settings window** — six panes (Display, Behaviour, Layouts, Characters, Tunnel, Steam),
   rebuilt from a cramped six-tab strip into a sidebar dialog.
3. **Icon set** — a symbolic tray icon (with state variants) and a full-colour launcher icon.

Target environment: **Rust + libcosmic/iced** on CachyOS. Nothing here assumes web tech.

## About the design files

The `.dc.html` files in this bundle are **design references created in HTML** — prototypes that
show intended look, copy, and behaviour. They are **not production code to port**. The task is to
recreate them in the Yutani codebase using libcosmic's own widgets (`cosmic::widget::{settings,
list_column, button, toggler, slider, spin_button, dropdown, text_input}`), the COSMIC theme
system, and the existing applet/layer-shell plumbing.

Two hard rules when translating:

- **Take colours from the COSMIC theme, not from the hex values below.** The hexes document intent
  and contrast relationships. In code use `cosmic::theme` roles — `Container::Card`,
  `Container::Background`, `Button::Suggested`, `Button::Destructive`, `Text::Accent` — so the
  design follows the user's accent colour and light/dark preference. Where this document says
  "accent blue", read "the user's accent colour".
- **The tray icon must be a symbolic SVG** using `currentColor`, installed in the icon theme, not a
  bitmap and not a fixed-colour SVG.

## Fidelity

**High fidelity.** Layout, spacing, type sizes, copy, and every interaction state are final and
intentional. Recreate them faithfully. The only deliberate abstraction is colour (see above).
Sample data — character names, IPs, layout names, timestamps — is placeholder; wire it to real
sources.

---

## Design tokens

### Colour

| Role | Value used in mock | Hex ≈ | COSMIC equivalent |
|---|---|---|---|
| Window / popover background | `oklch(0.185 0.008 265)` | `#1b1f29` | `Container::Background` |
| Sidebar background | `oklch(0.165 0.008 265)` | `#181b24` | `Container::Primary` |
| Card / list surface | `oklch(0.215 0.008 265)` | `#212530` | `Container::Card` |
| Sunken surface (code block) | `oklch(0.155 0.008 265)` | `#15181f` | `Container::Secondary` |
| Border, card edge | `oklch(0.28 0.01 265)` | `#2d323e` | theme divider |
| Border, row divider | `oklch(0.25 0.01 265)` | `#272b36` | theme divider, 50% |
| Control background | `oklch(0.25 0.01 265)` | `#272b36` | `Button::Standard` |
| Control border | `oklch(0.33 0.01 265)` | `#383e4c` | — |
| Row hover | `oklch(0.29 0.012 265)` | `#313744` | theme hover |
| Text primary | `oklch(0.95 0.004 265)` | `#eef0f3` | `Text::Default` |
| Text secondary | `oklch(0.66 0.01 265)` | `#9aa0ad` | `Text::Default` @ 70% |
| Text tertiary / captions | `oklch(0.58 0.01 265)` | `#848a95` | `Text::Default` @ 55% |
| Accent (primary buttons, selection) | `oklch(0.78 0.08 255)` | `#9ab8ec` | **user accent** |
| Accent on-colour (text on accent) | `oklch(0.18 0.03 255)` | `#1a2030` | accent foreground |
| Accent selected surface | `oklch(0.34 0.06 255)` | `#33405e` | accent @ 20% |
| Accent selected border | `oklch(0.55 0.09 255)` | `#5e7bb0` | accent @ 55% |
| Success / connected | `oklch(0.76 0.13 155)` | `#3fcf8e` | `Text::Success` |
| Upload series | `oklch(0.72 0.12 155)` | `#37c184` | success |
| Download series | `oklch(0.66 0.11 240)` | `#4f92d8` | accent-adjacent |
| Attention / amber | `oklch(0.76 0.14 75)` | `#e0af68` | `Text::Warning` |
| Destructive surface | `oklch(0.40 0.12 27)` | `#6e2f2a` | `Button::Destructive` |
| Destructive border | `oklch(0.52 0.14 27)` | `#97453c` | — |
| Destructive text | `oklch(0.85 0.12 27)` | `#f39a8c` | `Text::Danger` |
| Yutani violet (applet badge) | `oklch(0.84 0.09 285)` | `#b9a6f5` | — |

### Typography

Mock uses **Fira Sans** for UI and **JetBrains Mono** for all numerals, paths, commands, and key
combos. In the app, use the COSMIC interface font for UI and the COSMIC monospace font for the
mono role — do not bundle fonts.

| Role | Size / weight | Where |
|---|---|---|
| Window title | 13.5px / 600 | header bar |
| Pane heading | 17px / 600 | each settings pane |
| Pane subheading | 12.5px / 400, secondary | under each heading |
| Section label | 11px / 600, `letter-spacing: 0.07em`, uppercase, secondary | "FRAME", "ARRANGEMENT" |
| Row label | 13px / 400 | settings rows |
| Row help text | 11.5px / 400, tertiary | under row labels |
| Body / menu item | 12.5–13px / 400 | menus, buttons |
| Big metric | 20px / 500 mono | throughput numbers |
| Metric unit | 11px mono, tertiary | "KB/s" |
| Data value | 11.5–12px mono | IPs, paths, timestamps |
| Micro caption | 10.5px mono, tertiary | "live preview · 320 px" |

Minimum type size anywhere: **10.5px**, and only for mono captions.

### Geometry

- Radii: window `14`, card `11`, inner card `9–10`, control / button `7–8`, menu item `6`,
  toggle pill `11`, dots `50%`.
- Borders: `1px` everywhere; `1.5px dashed` for the drop zone; `2px` for colour-swatch selection rings.
- Spacing scale: `2, 4, 6, 8, 10, 12, 14, 16, 22, 26, 28` px. Card padding `11–16`. Section gap `22`.
- Control heights: toggle `40×22` (knob `16`), stepper `28`, small button `30–32`, primary button
  `36–38`, settings row ≈ `44` (12px vertical padding).
- Shadows: popover / window `0 26px 60px -24px rgba(0,0,0,0.95)`; menu popover
  `0 16px 34px -14px rgba(0,0,0,0.95)`; launcher icon `0 10px 18px rgba(0,0,0,0.55)`.
  In libcosmic, prefer the platform's own surface shadow.

---

## Screen 1 — Panel applet popover

File: `WireGuard Applet 1b.dc.html`. Width **340px**, height content-driven, radius 13.

Order top to bottom:

### 1. Yutani header row
`13px 16px 11px` padding. Left: a 26×26 rounded-square (radius 8) violet badge holding the letter
"Y" in 13px/600 mono, then "Yutani" 14px/600. Right: state word ("Running" / "Stopped") 12px/500
in success or muted, then a 40×22 toggle.

Toggling it stops/starts the whole service. **This is the master switch — everything below reacts.**

### 2. Accounts card
Card, `0 12px 12px` margins. Header row: "ACCOUNTS CONNECTED" section label; right side count
(`4 of 4`) in mono, coloured success when running.

Running state — one 30px row per client:
- 16px-wide mono index chip (`1`–`9`), radius 4. Focused client's chip uses accent surface/text.
- Character name, 12.5px, ellipsised.
- Right: the word `focused` in 10.5px mono violet, on the focused row only.
- Whole row is a button; clicking focuses that EVE client (same action as the hotkey).
- Focused row background: accent-violet tinted surface. Hover on others: row hover colour.

Stopped state — replaces the list with "Service stopped" 12px plus "Thumbnails, hotkeys and tunnel
routing are inactive." 11.5px tertiary.

Card footer (`8px 13px 10px`, top border): hotkey hint `Ctrl+Alt+1…4 · Ctrl+Alt+←/→` in 10.5px
mono (reads "hotkeys inactive" when stopped), and a small "Hide thumbnails" / "Show thumbnails"
button (radius 6, 11px) which goes dim and inert when the service is stopped.

The hint must be generated from the real bound prefix and the real client count, not hardcoded.

### 3. Divider + tunnel section label
1px divider at `0 16px 11px`. Then a row: "WIREGUARD TUNNEL" section label, right-aligned
"EVE traffic only" 11px tertiary.

### 4. Tunnel status line
Single row: 8px status dot with a 4px glow ring (`box-shadow: 0 0 0 4px <colour @ 50%>`), tunnel
name 14px/600, city 12.5px secondary, then uptime right-aligned in 11px mono, coloured by state.

### 5. Throughput graph card
The centrepiece. Card with:
- Header: left "↑ UPLOAD" (10.5px/600, upload green) with a 20px mono rate + 11px "KB/s"; right,
  mirrored and right-aligned, "↓ DOWNLOAD" in download blue.
- Graph: **34 columns**, `1.5px` gaps, total height 64px, padded `0 13px 10px`. Each column is a
  flex column: top half grows upward from a centre axis (upload, green, radius `1px 1px 0 0`),
  a 1px axis line, bottom half grows downward (download, blue). Bar height = value %, minimum 1px
  so an idle tunnel still shows a baseline.
- Footer: `<total> sent` / `60 s` / `<total> received`, 10.5px mono tertiary.

Sampling: 1 Hz, 34 samples ≈ 60 s of history (relabel if you change the rate). Scale each
direction independently against a rolling max.

### 6. Fact tiles
Two tiles in a `1fr 1fr` grid, gap 8. Each: 10.5px/600 label (`ENDPOINT`, `PEER`) over an
11.5px mono value, ellipsised. Values become `—` when idle.

### 7. Action row
Primary button (flex 1, height 38, radius 10, 13px/600) plus a 38×38 `⋯` overflow button.

Primary button states:
- service stopped → label "Start Yutani to route traffic", muted surface, dim text, **inert**
- connected → "Disconnect tunnel", standard surface
- installed but idle → "Connect tunnel", accent surface

### 8. Menu
Top border, `6px 6px 8px` padding. 31px rows, radius 8, label left 13px, optional hint right in
11px mono tertiary: **Switch peer…** (`3 available`), **Layouts & characters…**, **Quit**.

### Popover invariants

- **Stopping the service takes the tunnel idle with it.** Dot → muted, uptime → "idle", both rates
  → 0, graph flatlines, totals stop accumulating, tiles → `—`. No part of the panel may claim
  traffic is flowing while the overlay is down.
- Exactly one primary action visible at a time.
- Every number is monospace so digits don't jitter between ticks.
- Popover never exceeds the panel work area; the graph card is the first thing to drop on short
  screens.

---

## Screen 2 — Settings window

File: `Yutani Settings.dc.html`. Window **900×724**, radius 14.

### Chrome
46px header bar: title "Yutani Settings" 13.5px/600 left, then `saved` in 11px mono tertiary, then
a 26px close button whose hover state is destructive-tinted. In the real app use the COSMIC header
bar; keep the quiet "saved" indicator.

### Sidebar — 204px
`10px 8px` padding, 2px gaps. Each item is a two-line button (radius 8, `8px 10px`): name
13px/500 over a live sublabel 11px:

| Pane | Sublabel |
|---|---|
| Display | "Size, names, frame" |
| Behaviour | "Placement, keys" |
| Layouts | "*n* saved" |
| Characters | "Copy EVE settings" |
| Tunnel | "Connected · London" / "Installed · idle" / "Not installed" |
| Steam | "Launch options" |

Selected item: accent surface, accent text, accent-tinted sublabel. The sublabels are live state —
they are the reason the sidebar beats the old tab strip; keep them wired.

Sidebar footer card: a 6px success dot plus "Every change saves as you make it." 11px. **No
"open config file" affordance anywhere** — persistence is silent and immediate, on every control
change (debounce writes ~250ms).

Content pane: `20px 24px 28px`, scrolls independently. Each pane opens with a 17px/600 heading and
a 12.5px subhead, then 22px-gapped sections.

### Pane: Display

**Thumbnails card** with a live preview strip on top (`16px 16px 14px`, diagonal-stripe background,
bottom border): two mock thumbnails — first with the *focused* border colour, second with the
*inactive* colour — rendered at 42% of the real thumbnail width, 16:9, using the live radius and
border width, with the character-name caption below each when names are enabled. Right side:
`live preview · <width> px` in 10.5px mono.

Then three rows (12px 16px, dividers between):
1. **Thumbnail width** — help "Height follows the client's aspect ratio." Value `320 px` in 12px
   mono, 52px wide right-aligned; slider 200px, range **160–640 step 16**.
2. **Hover zoom** — help "Scale applied while the pointer is over a thumbnail." Value `1.0×`;
   slider range **1.0–2.5 step 0.1**.
3. **Show character names** — help "Caption under each thumbnail." Toggle.

Slider styling in the mock: 5px rail, accent fill to the left of the knob, 15px round knob with an
accent border. Use libcosmic's slider — it already looks like this.

**FRAME section** card:
- **Border width** — stepper, range **0–8**, step 1, displayed `N px` in a 40px mono field between
  − and + buttons.
- **Corner radius** — stepper, range **0–24**, step 2.
- **Focused client border** — help "Marks the client your keyboard is driving." Four 24px swatches
  (radius 6) + hex in 11.5px mono. Selected swatch gets a 2px light ring.
- **Other clients border** — help "Keep it dim so the focused one stands out."
- Palette: `#7aa2f7`, `#9ece6a`, `#e0af68`, `#1b2130`. Defaults: focused `#7aa2f7`,
  inactive `#1b2130`.
- Footer button, right-aligned: "Reset frame to defaults" → border 1, radius 8, the two defaults above.

The two colour rows replace the original's two identical `#000814` text fields; label them by
meaning, never "active/inactive colour".

### Pane: Behaviour

**ARRANGEMENT** card:
- Mode as two side-by-side described cards (radius 9, `10px 12px`): **Floating** — "Drag each
  thumbnail anywhere."; **Docked** — "Auto-arranged along one screen edge."
- **Dock edge** row (Top / Bottom / Left / Right segmented buttons) — **only rendered when Docked**.
  Do not show it disabled.
- **Capture frame rate** — `15 / 30 / 60` mono segmented buttons, and help text that changes with
  the value:
  - 60 → "Smoothest, and the most GPU per client. Drop to 30 when running many clients."
  - 30 → "A good balance for six or more clients."
  - 15 → "Cheapest — thumbnails update visibly in steps."

**VISIBILITY & DRAGGING** card:
- **Show thumbnails** — `Always` / `Only with EVE focused` / `Never`.
- **Hide the focused client's own thumbnail** — "Avoids a thumbnail of the window you are already
  looking at."
- **Snap to a 16 px grid while dragging** — "Keeps rows tidy without fiddling."
- **Snap flush against other thumbnails** — "Thumbnails stick edge to edge."

**KEYBOARD SHORTCUTS** card:
- **Modifier prefix** — `Ctrl + Alt` / `Super` / `Ctrl + Shift`, help "Applies to every Yutani
  shortcut below."
- A `1fr auto` grid of resulting bindings, each combo in a mono chip (radius 6, `4px 9px`, control
  surface + border):
  `Focus client 1 – 9` → `<prefix> + 1 … 9`; `Next client` → `<prefix> + →`;
  `Previous client` → `<prefix> + ←`; `Show / hide thumbnails` → `<prefix> + T`.

Combos are **derived from the prefix** — the user picks one thing and sees all four update. Replaces
the original's prefix-dropdown-plus-key-dropdown the user had to assemble mentally. If you add
per-action rebinding later, make the chips key-capture buttons; keep the derived display.

### Pane: Layouts

One card listing saved layouts, one row each (`12px 14px`, `position: relative`):
- 7px dot — success when applied, dim otherwise; applied row gets a success-tinted background.
- Name 13.5px/500, plus `applied` in 11px success when current.
- Meta line in 11px mono: `3 monitors · 5 thumbnails · saved 12 Sep`.
- Right: **Apply** (accent-tinted) or **Re-apply** (plain, for the current one), then a 30px `⋯`.

`⋯` opens a real popover (168px, radius 9, absolutely positioned `top: 42px; right: 14px`,
z-index above rows): **Rename…**, **Duplicate**, divider, **Delete…** in destructive text.
- Rename swaps the name block for a text input with accent border, and the actions for
  Cancel / Rename.
- Duplicate inserts `<name> copy` directly below with meta "saved just now".
- Delete… arms an inline confirm in the row: "Delete this layout?" + Cancel + a destructive Delete.

Never put a bare Delete button next to Apply.

Card footer row: text input "Name this arrangement" + **Save current arrangement** button. The
button is inert (muted surface, `cursor: not-allowed`) until the field is non-empty; saving inserts
the layout, marks it applied, and clears the field.

Below the card, an info note (radius 10, `12px 14px`, accent `i` glyph): "A layout that names a
monitor you no longer have is applied to the primary display instead. Nothing is lost — reconnect
the monitor and apply it again."

### Pane: Characters

This pane performs a **destructive, whole-file replacement**. It is structured as three numbered
steps inside one card; each step number is an accent chip (radius 4, `2px 6px`, 11px mono).

**1 Copy from** — a wrapping row of character cards (radius 9, `8px 12px`): name 12.5px/500 over
`last played 14 min ago` in 10.5px mono. Selected card uses the accent surface/border/text.

**2 What gets copied** — two inner rows (radius 9, `10px 12px`):
- A success `✓` glyph + "Interface — overview, window positions, chat, UI layout" with
  "Always included. This is the point of the operation." — not a toggle.
- "Account settings — shortcuts, general, graphics, audio" + toggle, help "Off by default: graphics
  settings rarely suit every machine." **Default off.**

**3 Apply** — three states:
- *idle*: accent button "Copy to N other characters" (N = character count − 1) plus "A backup is
  taken first." 11.5px.
- *confirm*: a destructive-tinted panel (radius 10, `12px 13px`) reading "This replaces the
  interface [and account] settings of N characters with <Name>'s. Their current files are backed up
  first, and can be restored below." with **Replace their settings** (destructive) and **Cancel**.
  The sentence must name the source character, the count, and whether account settings are included.
- *done*: success-tinted panel, `✓`, "<Name>'s settings copied to N characters. Restart any running
  client to see them." plus Dismiss.

**SAFETY NET** card:
- **Last backup** — humanised: `13 Sep 2026, 05:38 · before the last copy` in 11px mono, with a
  **Restore** button. Never show a raw `.../backups/20260913T053807Z` path as the primary label.
- **EVE profile folder** — label row with a **Change…** button, the full path in 11px mono
  `word-break: break-all`, then "Found automatically through Steam's library list." 11.5px.

### Pane: Tunnel

**State card** — border turns success-tinted while connected:
- 9px dot with glow ring; headline 14px/600 — "Connected — London" / "Installed — not connected" /
  "No tunnel installed"; sub 11.5px mono — "exit 203.0.113.42 · EVE traffic only" / "ready to
  connect · EVE traffic only" / "EVE uses your normal connection".
- Primary button: "Disconnect" (standard) / "Connect" (accent) / "Install first" (inert), plus a
  36px `↻` refresh.
- Fact strip (top border, 3-column grid): `CONFIG` → `EVE-UK-455.conf`, `PRIVATE KEY` →
  `/etc/yutani · root only`, `INSTALLED` → `13 Sep, 05:38`.

**CONFIGURATION** card:
- A dashed drop zone (`1.5px dashed`, radius 10, `18px 16px`, sunken background): "Drop a wg-quick
  `.conf` here" 13px, "From your VPN provider — one server, WireGuard format." 11.5px. Must accept
  a real file drop.
- Inside it, a row: path text input (mono 11.5px, placeholder "…or type a path"), **Browse…**, and
  the accent primary — "Install tunnel", or "Replace" when one is installed. Inert while the path
  is empty.
- Two bullet notes, 11.5px:
  - "Installing asks for your password once. The private key is written to /etc/yutani, readable
    only by root, and never shown here."
  - "Delete the downloaded .conf afterwards — it holds that same key and is world-readable in your
    Downloads folder."

**DNS INSIDE THE TUNNEL** card: `Resolvers` → `1.1.1.1, 9.9.9.9`; `Domains routed` →
`eveonline.com, ccpgames.com, evetech.net`; footer note "Written into the tunnel when it is
installed — replace the configuration to change them." Read-only, and with **no reference to editing
a config file**.

**Danger row** (separate card, destructive-tinted border and background): "Uninstall tunnel" 13px/500
over "Removes the interface and the stored private key. EVE goes back to your normal connection."
Right: **Uninstall…** outline button → Cancel + destructive **Uninstall**. Uninstalling sets
not-installed and disconnected.

Uninstall must never sit adjacent to Connect, as it did originally.

### Pane: Steam

One card:
- Three numbered steps (accent chips): "In Steam, right-click EVE Online → Properties → General.",
  "Paste the line below into Launch Options.", "Close Properties and launch each account as usual."
- A sunken code block (radius 9, `12px 13px`) with the command in 12px mono,
  `word-break: break-all`, and a **Copy** button that becomes **Copied** on a success-green
  surface for 1.6s.
- Default command:
  `PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 yutani launch -- %command%`
- A toggle row: "Steam can't find `yutani`" + "Flatpak Steam, or a session without your shell's
  PATH. Uses the full binary path instead." Enabling it swaps the command to
  `… /usr/local/bin/yutani launch -- %command%` (resolve the real binary path at runtime).
  One command block, not two.
- Closing info note: "Launch each EVE account as usual afterwards — Yutani picks up every client
  automatically and gives it the next free hotkey."

---

## Screen 3 — Icons

File: `Yutani Tray Icon.dc.html`. Redrawn from the supplied raster mark on a **32-unit grid**, only
45° and orthogonal edges.

### Geometry

Two-piece variant (**22px and above**), `viewBox="0 0 32 32"`:

```
<path d="M5.5 7 H10.8 L16 13.3 L21.2 7 H26.5 V8.6 L18.6 18.2 V20.9 L13.4 22.6 V18.2 L5.5 8.6 Z"/>
<path d="M13.4 24.2 L18.6 22.5 V25.1 L16.8 26.9 H13.4 Z"/>
```

Solid variant (**16px**, diagonal slice omitted — at 16px it lands on half a pixel):

```
<path d="M5.5 7 H10.8 L16 13.3 L21.2 7 H26.5 V8.6 L18.6 18.2 V25.1 L16.8 26.9 H13.4 V18.2 L5.5 8.6 Z"/>
```

Stem is 5 of 32 units (2.5px at tray size — the thinnest stroke that survives). Arms meet at 45°
so both diagonals antialias evenly. Every edge lands on a whole pixel at 16px.

### Tray icon

- `fill="currentColor"`, no gradients, no fixed colours. Ship as
  `yutani-symbolic.svg` in `hicolor/scalable/status/` (or the app's icon dir) and load with
  `cosmic::widget::icon::from_name("yutani-symbolic")`.
- Sizes to provide: 16 (solid variant), 22, 24, 32 (two-piece).
- States — **never recolour the whole glyph**:
  - Running: full-strength panel foreground.
  - Stopped: same shape at **40% opacity**.
  - Tunnel connected: 8px success dot, bottom-right, with a 1.5px cut-out ring in the panel colour.
  - Needs attention: same dot in amber `#e0af68`.

### Launcher icon

- 116×116 plate inside a 128 box (6px margin), radius **26**.
- Plate gradient: `#303a4f → #1b2130 → #0d1017` along `(0.1,0) → (0.8,1)`. Derived from a single
  base `#1b2130`: top stop = base mixed 16% toward white, bottom = base darkened 42%.
- Top sheen: a quarter-round highlight over the top-left, white 16% → 0%.
- Rim: 1.5px inset stroke, white 80% at the top fading to transparent, black 50% at the bottom.
- Mark: drawn at `translate(24 24) scale(2.5)`; a dark copy (`#0a0d14`, 55%) offset
  `(0.55, 0.75)` as the side wall, then the face filled `#ffffff → #dfe5ef → #a7b0c2`, plus a
  0.35-unit white 85% stroke along the top-left contour.
- Drop shadow `0 10px 18px rgba(0,0,0,0.55)`.
- Provide 128 / 64 / 40; below 64 drop the slice and the top-contour stroke. No bevel detail finer
  than one pixel at 40px.

---

## Interactions & behaviour summary

| Trigger | Result |
|---|---|
| Applet service toggle off | Accounts list → stopped copy; hotkey hint → "hotkeys inactive"; thumbnails button dim + inert; tunnel forced idle (dot, uptime, rates, graph, totals, tiles); primary button → "Start Yutani to route traffic", inert |
| Click an account row | Focuses that EVE client; row + index chip take the focused treatment |
| Applet primary button | Connect / disconnect the tunnel; resets session timer |
| Throughput tick | 1 Hz: push a sample, shift the 34-sample window, accumulate totals |
| Settings sidebar change | Switch pane; cancel any armed confirm or open menu |
| Any settings control change | Persist immediately (debounce ~250ms). No Apply button, no save dialog |
| Mode → Floating | Dock edge row disappears entirely |
| Frame rate change | Help text swaps to the matching sentence |
| Prefix change | All four shortcut chips recompute |
| Layout `⋯` | Popover: Rename… / Duplicate / Delete… |
| Layout Delete… | Inline confirm in the row; only then does it delete |
| Layout Apply | Marks it applied, clears applied from the others |
| Characters step 3 | idle → confirm (naming source + count + scope) → done |
| Tunnel install | Requires a non-empty path; polkit prompt; sets installed + connected |
| Tunnel Uninstall… | Cancel / Uninstall confirm; clears installed + connected |
| Steam Copy | Copies to clipboard; button reads "Copied" on green for 1.6s |
| Steam PATH toggle | Swaps the single command block to the absolute-path variant |

Transitions in the mock are instant. If you add motion, keep it ≤150ms and only on hover, selection,
and popover open.

## State

**Applet:** `service_running`, `focused_client`, `thumbnails_visible`, `clients: Vec<{name,
index}>`, `tunnel: {installed, connected, endpoint, peer, session_secs}`,
`throughput: {up_rate, down_rate, up_total, down_total, history: VecDeque<(f32,f32)>}` (34 entries).

**Settings:** `thumbnail_width`, `hover_zoom`, `show_names`, `border_width`, `corner_radius`,
`active_border`, `inactive_border`, `mode`, `dock_edge`, `capture_fps`, `visibility`,
`hide_own_thumbnail`, `snap_grid`, `snap_flush`, `shortcut_prefix`, `layouts: Vec<{name, meta,
applied}>`, `copy_source`, `copy_account_settings`, `tunnel_conf_path`, plus transient UI state
(`open_menu`, `renaming`, `confirm_delete`, `copy_phase`, `uninstall_phase`, `copied_at`).

Everything persistent writes through to the config on change. Transient UI state is never persisted
and is cleared on pane change.

## Assets

No bitmaps. The icons are the SVG paths above; the only external inputs were the user's original
tray-icon raster (redrawn, not traced) and the six settings screenshots (replaced). Fonts are
system fonts — bundle nothing.

## Files

| File | Contains |
|---|---|
| `WireGuard Applet 1b.dc.html` | Panel applet popover, live throughput, all service/tunnel states |
| `Yutani Settings.dc.html` | Settings window, all six panes, all confirm/menu/rename states |
| `Yutani Tray Icon.dc.html` | Symbolic tray icon at 16/22/32/64, state variants, launcher icon at 128/64/40, panel mock in dark and light |
| `support.js` | Runtime for the three files above — needed only to open them in a browser |

Open each file in a browser and interact with it; every state described here is reachable by
clicking (service toggle, layout `⋯`, the Characters confirm, Uninstall…, the Steam PATH toggle,
dark/light panel and running/stopped in the icon sheet).
