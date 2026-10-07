# Handoff: Yutani applet popover — v6 "Nostromo"

## Overview
A full visual redesign of the **panel applet popover** only. Same information and actions as the
built popover (service, tunnel, accounts, throughput, one primary action) plus the three additions
agreed since the last handoff — **Launch EVE**, **ping to Tranquility**, and **host load
(CPU · GPU · RAM)** — restyled as a MU/TH/UR-class terminal: one phosphor green on black glass,
inverted blocks for emphasis, amber only when something wants attention.

Settings window, overlay and icon set are **unchanged** — the previous handoff
(`reference/previous-handoff.md` in your package) still governs them. The panel itself stays
COSMIC: the tray button, its hover and the symbolic mark are untouched; only the badge colours and
the popover content change.

Target: **Rust + libcosmic/iced** on CachyOS, as before. Everything below is written against
`src/applet/theme.rs` (your `reference/source/applet-theme.rs`) so the diff is mechanical.

## About the design files
`Yutani Applet v6.dc.html` is a **design reference built in HTML** — open it in a browser (it needs
`support.js` next to it) and click through every state: both rocker toggles, the account rows,
Launch EVE, the ⋯ menu. It is **not code to port**. Recreate it with libcosmic widgets and the
custom classes pattern already used in `theme.rs`.

The simulated data (traffic bursts, ping, host load, the 4-step launch log timing) is placeholder;
wire it to the real sources listed under *Data sources*.

## Fidelity
**High fidelity.** Layout, spacing, type, copy and every state are final. Two deliberate departures
from the package rules, both intentional and both to be kept:

1. **Fixed palette instead of COSMIC theme roles.** This is a skin, and its whole point is one
   phosphor on black regardless of the user's accent. Implement it as constants beside `VIOLET`
   in `theme.rs`, not by reading the theme.
2. **Two bundled fonts** (three files). See *Typography*.

Scanlines and vignette are the only *optional* elements — drop them if they cost more than a
cheap overlay (see *Implementation notes*).

---

## Design tokens — Nostromo palette
Also in `tokens-nostromo.css` in the same shape as your `tokens.css`.

| Token | Hex | Used for |
|---|---|---|
| `--bg` | `#050907` | popover ground; text on filled phosphor/amber |
| `--card` | `#081109` | every card |
| `--line` | `#173b22` | card borders, hairlines, unlit gauge cells, disabled outlines |
| `--line-2` | `#245233` | dotted section rules, resting toggle/chip borders, "·" separators |
| `--phosphor` | `#7ce38b` | primary ink, section labels, LEDs on, filled buttons, downlink |
| `--dim` | `#4f9c5c` | secondary ink: sublines, units, captions, footer |
| `--dimmer` | `#3c7a47` | idle/stopped ink, pending launch lines, inert buttons |
| `--white` | `#e9f5e6` | big readouts, wordmark, focused account name, counts |
| `--amber` | `#ffb000` | attention: uplink, focused account, pending toggle, Steam notice, ≥70 % gauges |
| `--red` | `#ff4a3d` | Quit, ping ≥100 ms, ≥90 % gauges |

Alpha variants used: `phosphor 7 %` (`#7ce38b12`) row hover · `phosphor 10 %` (`#7ce38b1a`)
open overflow fill · `phosphor 25 %` (`#7ce38b40`) launch glow · `phosphor 40 %` (`#7ce38b66`)
readout glow · `amber 8 %` (`#ffb00014`) focused row · `amber 12 %` (`#ffb0001f`) header plate
· `amber 50 %` (`#ffb00080`) Steam notice border · `red 10 %` (`#ff4a3d1a`) Quit hover.

No light variant: the popover is always dark glass. The **tray badge** is the one place the palette
meets a possibly light panel — on a light theme fall back to the theme's success/warning colours
for the badge (phosphor on `#d7d7d7` is under 2:1).

### Typography
| Face | Weights | Role |
|---|---|---|
| **B612 Mono** | 400, 700 | everything — labels, numbers, buttons, menu, footer |
| **Michroma** | 400 | wordmark "YUTANI", section index chips 01–04 |
| **Noto Sans JP** | 500 | the one subline "ユタニ重工 · MULTIBOX SYSTEMS" (B612 has no kana) |

All three are SIL OFL on Google Fonts — vendor the `.ttf`s under `assets/fonts/` and load with
`include_bytes!` + `cosmic::iced::font::load` at startup. Fallback if a load fails: the COSMIC
monospace font. `font-smoothing: antialiased`; the mock's line-height is 1.35.

Sizes used (px / weight / tracking): wordmark 12 Michroma .24em · subline 8.5 .14em · state word
10/700 .14em · section index 7.5 Michroma .12em · section label 10/700 .16em uppercase · section
meta 8.5 .1em · row title 11.5/700 .06em · row sub 8.5 .06em · account name 11.5 uppercase .04em ·
index chip 9.5/700 · "◄ FOCUSED" 8.5/700 .16em · empty-state title 10.5/700 .08em, body 9.5 .04em ·
readout label 8.5/700 .16em · readout value 16/700 (ping 20/700) · unit 8.5 .08em · scope footer 8.5
.08em · ping stats 8 · facts line 8.5 .04em · gauge label 9/700 .12em, value 11/700, detail 8.5 ·
Steam notice 9.5 .04em · primary button 11.5/700 .14em uppercase, sub 8.5 .12em · launch log 9.5
.04em · menu 10.5 .08em uppercase · footer 8.5 .16em uppercase.

Minimum size is **8 px** (ping stats, by design — it is a terminal). Everything numeric is mono so
digits never jitter.

### Geometry
- Radii: popover `4`, cards and buttons `2`, chips/rows/LEDs/gauge cells `1`. **Nothing round.**
- Borders `1px` everywhere. Corner brackets `8×8`, `1px`, phosphor, offset `-1px` outside the
  card edge — top-left and bottom-right only, and **only on the two interactive cards** (01, 02).
- Gutters: popover padding `10` (header/section rows use `14`), `6` under a section header,
  `10` between sections, `8 12` inside rows, hairlines inset `12`.
- Heights: toggle `38×19` (knob 13), account row `28`, menu row `28`, primary button `36`,
  overflow `36×36`, LED `8` (header LED 6), scope `104`, ping sparkline `22`, gauge cell `7`.
- Popover: width **360** (= libcosmic's popup width, so no change to `POPOVER_WIDTH`), border
  `1px --line`, radius 4, shadow `0 30px 70px -28px #000` + `0 0 0 1px #00000080`.

### Mapping onto `theme.rs`
| Constant | Was | Now |
|---|---|---|
| `CARD_RADIUS` / `TILE_RADIUS` / `PRIMARY_RADIUS` | 11 / 9 / 10 | **2** |
| `ROW_RADIUS` / `MENU_RADIUS` / `CHIP_RADIUS` / `SMALL_BUTTON_RADIUS` | 7 / 8 / 4 / 6 | **1** |
| `BADGE_PX` / `BADGE_RADIUS` / `BADGE_MARK_PX` | 26 / 8 / 18 | 26 / **2** / **17** |
| `PRIMARY_HEIGHT` / `OVERFLOW_PX` | 38 / 38 | **36 / 36** |
| `ACCOUNT_ROW_HEIGHT` / `MENU_ROW_HEIGHT` | 30 / 31 | **28 / 28** |
| `TOGGLE_PX` | 22 (pill) | **19** (38×19 square rocker, custom class) |
| `STATUS_DOT_PX` / `STATUS_GLOW_PX` | 8 / 4 ring | 8 square / **glow = shadow blur 8 @ 67 %** |
| `GRAPH_HEIGHT` | 64 bars | **104** canvas (sand field) |
| `INDEX_CHIP_WIDTH` | 16 | **17×17**, bordered |
| `HEADER_PAD` | 13 16 11 | **12 14 10** |
| `CARD_MARGIN` | 0 12 12 | **0 10 10** |
| `MENU_PAD` / `MENU_ROW_PAD` | 6 6 8 / 0 10 | **5 5 5** / 0 10 |
| `VIOLET*` | header badge | **retire** in this skin — plate is amber-outlined |
| `ink / secondary / tertiary / success / warning / destructive` | theme roles | **phosphor / dim / dimmer / phosphor / amber / red** constants |
| `DIM_OPACITY` | 0.40 | 0.40 (unchanged, tray mark) |

Keep the existing COSMIC-roles implementation reachable behind a `Skin` enum if you want both;
the mock only shows Nostromo.

---

## Screen — the popover, top to bottom
Reference renders: `previews/01…05` (panel + popover at 2×). Width 360, height content-driven (≈ 640 with two accounts).

### Glass
Three non-interactive overlays on the popover, above content: a `1px` phosphor line along the top
edge at 70 %; scanlines `repeating-linear-gradient(0deg, transparent 0 2px, #00000059 2px 3px)`;
vignette `radial-gradient(ellipse at 50% 40%, transparent 55%, #00000066 100%)`.

### Header — `12 14 10`, gap 10
- Plate 26×26, radius 2, fill amber 12 %, border 1px amber, the symbolic Y mark 17 px in amber.
- "YUTANI" Michroma 12 .24em white, over "ユタニ重工 · MULTIBOX SYSTEMS" 8.5 .14em dim.
- Right, pushed to the edge: LED 6×6 (phosphor + glow when running, dimmer when stopped) then
  "RUNNING" / "STOPPED" 10/700 .14em in the same colour.

### Section header pattern — `0 14 6`, gap 8
Index chip (Michroma 7.5, padding `3 5 2`, phosphor fill, bg text) · label 10/700 uppercase ·
a 1px dotted rule that fills the row (`repeating-linear-gradient(90deg, --line-2 0 2px,
transparent 2px 5px)`) · optional right-hand meta 8.5 dim.

| # | Label | Right meta |
|---|---|---|
| 01 | CONTROL | — |
| 02 | ACCOUNTS (ACCOUNT when 1) | count `02` in 10.5/700 white — only when running and > 1 |
| 03 | NETWORK | "EVE TRAFFIC ONLY" |
| 04 | HOST | "32 GB · 1 HZ" (real RAM total, real sample rate) |

### 01 · Control card (brackets)
Two rows split by a hairline, each `8 12`, gap 10: LED 8×8 · title 11.5/700 · sub 8.5 · rocker.

**Yutani service** — sub "MULTIBOX · HOTKEYS · ROUTING ACTIVE" (dim) / "STOPPED · MULTIBOX,
HOTKEYS, ROUTING OFF" (dimmer). Master switch: everything below reacts.

**WireGuard tunnel** — sub by state (LED colour in brackets):
- stopped: "START YUTANI TO ROUTE TRAFFIC" (dimmer), rocker disabled
- not installed: "NO TUNNEL INSTALLED · PREFERENCES → TUNNEL" (dimmer), rocker disabled
- connecting: "CONNECTING · LONDON" (amber), rocker amber-filled, cursor progress
- stale handshake: "LONDON · HANDSHAKE STALE" (amber), rocker on
- connected: "CONNECTED · LONDON" (dim), rocker on
- idle: "IDLE · LONDON" (dimmer), rocker off

**Rocker** 38×19 radius 2, 1px border. Off: transparent, border `--line-2`, knob dim. On: fill
phosphor, border phosphor, knob `--bg`. Pending: same with amber. Disabled: opacity .45, border
`--line`, knob dimmer. Knob 13×13 radius 1 at `top 2`, `left 2` → `left 21`, 120 ms.

### 02 · Accounts card (brackets)
Running with clients — list padding 5, gap 1, one 28 px button row per client (`0 8`, gap 10):
index chip 17×17 (border `--line-2`, text dim; **focused: amber fill, bg text**) · name uppercase,
phosphor (white when focused), ellipsised · on the focused row only, right-aligned "◄ FOCUSED"
8.5/700 amber. Focused row fill amber 8 %; hover elsewhere phosphor 7 %. Clicking a row focuses that
EVE client (same as its hotkey).

Running, no clients — "NO EVE CLIENTS RUNNING" / "LAUNCH EVE BELOW — IT APPEARS HERE WITH THE NEXT
FREE HOTKEY." Stopped — "SERVICE STOPPED" (dim) / "THUMBNAILS, HOTKEYS AND TUNNEL ROUTING ARE
INACTIVE." (dimmer). Padding `10 12 11`, gap 3.

The hotkey hint and Hide-thumbnails button from the previous popover are **gone** (agreed earlier).

### 03 · Network card (no brackets)
One card, three bands split by hairlines.

**Scope (104 px)** — the sand field canvas fills it (algorithm below); 7×7 phosphor corner marks
inside each corner. Overlaid: top-left "▲ UPLINK" (amber) · rate 16/700 white with glow · "KB/S";
top-right mirrored "… KB/S DOWNLINK ▼" (phosphor). Bottom line, three-way justified:
"105.8 MB TX" · "SESSION" · "693.6 MB RX" (8.5 dim). Dropped first on short screens (your
`shortScreen` flag) — the rest of the card stays.

**Ping row** — `8 12`, gap 10. Left column 64 wide: "PING · TQ" over the value 20/700 white +
"MS". Middle: 240×22 sparkline — baseline at y 21.5 in `--line`, average as a white 30 % dashed
line (`1 4`), **34 squares** (2 px, newest 3 px) at opacity .3 → 1 oldest → newest, all in the
quality colour; under it "MIN 3 · AVG 5 · MAX 12" 8 dim. Right column 74 wide, right-aligned:
quality word 8.5/700 + 6×6 LED, then "JIT 1.2 · LOSS 0.0%" 8 dim.

Quality: **< 30 ms NOMINAL phosphor · < 100 DEGRADED amber · else POOR red · tunnel down IDLE
dimmer** (values "—", squares sit on the baseline).

**Facts line** — `7 12 8`, 8.5: "ENDPOINT 203.0.113.42 · PEER yutani0" left, uptime right
("T+ 1H 38M" phosphor; "IDLE" dimmer). Uptime format: `T+ {h}H {mm}M` when ≥ 1 h, else
`T+ {m}M {ss}S`, else `T+ {s}S`. Resets when a tunnel session starts.

### 04 · Host card (no brackets)
Padding `8 12 9`, three rows gap 6: label 26 wide ("CPU/GPU/RAM" 9/700) · 20 cells (flex, gap 2,
7 high, radius 1) · value 32 wide right-aligned 11/700 · detail 54 wide right-aligned 8.5 dim.
Lit cells = round(pct/100 × 20). Colour: phosphor, **amber ≥ 70 %, red ≥ 90 %**; while phosphor,
the last lit cell is white (the "cursor"). Value is white, or amber/red with the cells. Detail:
CPU and GPU temperature "61°C", RAM used "10.2 GB". RAM % is used ÷ total.

### Steam notice (only when Steam launch options are missing and the service runs)
Card margin `0 10 10`, border 1px amber 50 %, radius 2; a 4 px hazard stripe on top
(`repeating-linear-gradient(135deg, amber 0 6px, --bg 6px 12px)`); text `7 11 8` 9.5 amber:
"STEAM LAUNCHES EVE WITHOUT YUTANI — IT RUNS OUTSIDE THE TUNNEL. OPEN SETTINGS → STEAM."

### Action row — margin `0 10 10`, gap 8
**Launch EVE** (flex 1, 36 high, radius 2, 11.5/700 uppercase, label left / sub right, padding
`0 14`):
- ready: "▶ LAUNCH EVE", phosphor fill, bg text, glow `0 0 14px phosphor 25 %`
- launching: "LAUNCHING…" + sub "STEP 2 / 4", phosphor outline, phosphor text, cursor progress
- service stopped: "START YUTANI TO LAUNCH", `--line` outline, dimmer text, inert
- nine clients: "ALL 9 SLOTS IN USE", same inert look

**⋯** 36×36 radius 2, 15 px, border `--line-2`; open: border phosphor, fill phosphor 10 %.

### Launch log (while launching) — margin `0 10 10`
Outline card `7 12 8`, gap 3, 9.5: four lines, left text / right status:
"▸ STEAM · APPLAUNCH 8500", "▸ STEAM · WINDOW MINIMISED", "▸ EVE CLIENT · STARTING",
"▸ HOTKEY ASSIGNED · CTRL+ALT+3" (next free slot). Done lines phosphor + "OK", the current line
amber + "…", pending dimmer. Mock timing 850 ms per step; real timing follows the IPC. On
completion the new client is appended to 02 and **becomes focused**.

### Menu (⋯ open) — top hairline, padding 5, gap 1
28 px rows, radius 1, 10.5 uppercase .08em, hover phosphor 7 %: "LAYOUTS & CHARACTERS…",
"PREFERENCES…", "QUIT" (red, hover red 10 %).

### Footer — top hairline, `7 14 8`, 8.5 .16em uppercase dim
"YUTANI OS · BUILD 8A03B1F" (real build hash) left; "READY FOR INQUIRY" + a 6×9 phosphor block
cursor blinking at 1 Hz (step, not fade) right.

### Tray badge (the only panel change)
Still the symbolic mark; the badge becomes a **7×7 square, radius 1**, 1.5 px ring in the panel
colour: connected → phosphor with glow; attention → amber; sync → hollow amber outline; service
stopped / not installed → mark at 40 % with no badge. On light panels use theme success/warning.

---

## The sand field (scope background)
Reference implementation in `reference/sand-field.js` (lifted verbatim from the mock). Spec:

- Canvas = scope area, 338×104 CSS px, DPR capped at 2, redrawn every frame (`window::frames()`
  subscription; stop when the popover closes).
- **1 000 grains.** Each: `x` random, `off = (rand − .62) × 28` (most grains ride above the
  surface), `y = .64 H + off`, phase `ph`, size 1 px (12 % are 1.5 px), brightness `b ∈ [.3, 1]`,
  `k` random.
- **Dune surface** `ys(x,t) = .64 H + 11 sin(.011x + .25t) + 5 sin(.029x − .17t + 1.3) + 3 sin(.07x + .6t)`.
- **Wind** target `= live ? 10 + min(120, up + down) × .85 : 2` px/s, eased at 2.5 /s.
- Per grain per frame (`dt` capped at 50 ms): `gust = .5 + .5 sin(.02x + 1.7t + ph)`;
  `vx = wind × (.5 + gust) × (.6 + .8k) × (off < 0 ? 1.6 : .7)` (airborne grains run faster);
  `vy = (ys + off − y) × 2.5 + wind × .15 × sin(.05x + 2.3t + ph)`. Past `x > W + 2` the grain
  respawns at `x = −2` with fresh `off`.
- Alpha `= b × (.5 + .5 sin(2.2t + 3ph)) × (live ? 1 : .45)`. Quantise to 11 levels so fill
  styles are cached (3 colours × 11).
- Colour: `k > .975` → white spark; `k < up/(up+down)` → amber (the uplink share — 30 % when idle);
  else phosphor. So the dune's colour mix *is* the up/down ratio and its speed *is* throughput.
- Tunnel down: wind dies to 2, grains dim to 45 %. Never clear the canvas to a flat colour.

Budget: ~1 000 `fill_rectangle` calls per frame — fine on iced's canvas. If the panel is on battery,
cap at 30 fps.

## Data sources
- **Throughput / totals:** `wg show wg0 transfer` (or Netlink `WG_CMD_GET_DEVICE`) at 1 Hz; rate is
  the delta, totals are since the session started (not interface lifetime). Display KB/s integers,
  MB with one decimal.
- **Ping:** 1 Hz probe to the Tranquility endpoint the app already uses for server status, through
  the tunnel when it is up; keep 34 samples. MIN/AVG/MAX integers, JIT = mean absolute delta (1 dp),
  LOSS % over the window (1 dp).
- **Host:** `/proc/stat` (CPU %), `/proc/meminfo` (RAM), amdgpu sysfs `gpu_busy_percent` or
  `nvidia-smi` (GPU %), hwmon for temperatures; 1 Hz, smoothed (mock eases 20 % per tick).
- **Launch EVE:** new IPC action — `steam steam://rungameid/8500` (or `-applaunch 8500`), wait for
  Steam's window, minimise it, wait for the EVE window, assign the next free hotkey; report each
  step so the log can advance.
- **Endpoint / peer / location:** from the installed tunnel config; location is the configured exit
  label ("London").

## Interactions & behaviour
| Trigger | Result |
|---|---|
| Service rocker off | header LED + word STOPPED; 02 → stopped copy, count hidden; tunnel rocker disabled, sub "START YUTANI…"; rates 0, wind dies, totals freeze, ping → IDLE/—, facts → —, uptime IDLE; host gauges fall toward idle; Launch → inert "START YUTANI TO LAUNCH" |
| Tunnel rocker | connected → idle instantly; idle → connecting (amber, ~1.5 s) → connected, session timer resets |
| Account row click | focuses that EVE client; chip and row take the amber focused treatment |
| Launch EVE | log card appears, steps advance, button shows STEP n / 4; on finish the new account is appended and focused, log disappears |
| ⋯ | toggles the menu; button takes the open treatment |
| 1 Hz tick | push throughput + ping samples (34-window), accumulate totals, advance uptime, resample host |
| Short screen | scope band dropped first; nothing else collapses |

Motion: rocker knob 120 ms; cursor blink 1 Hz stepped; sand field continuous. No other transitions.

## State
`service_running`, `tunnel: Connected | Idle | NotInstalled | StaleHandshake | Connecting`,
`location`, `endpoint`, `peer`, `session_secs`, `clients: Vec<{name, index}>`, `focused`,
`throughput: {up_rate, down_rate, up_total, down_total, history: VecDeque<(u32,u32)>}` (34),
`ping: {history: VecDeque<f32>} ` (34) with derived min/avg/max/jitter/loss/quality,
`host: {cpu, gpu, ram_used, ram_total, cpu_temp, gpu_temp}`, `steam_launch_options_missing`,
`launch: Option<step 0..4>`, `menu_open`, `short_screen`. Transient (launch, menu) never persists.

## Decisions for you
1. **Replace or add?** The mock replaces the COSMIC-roles popover. Keeping both behind a Skin
   enum costs little and keeps the package's theme-roles rule intact as the default.
2. **Scanlines / vignette.** iced has no radial gradients; both need a canvas overlay or a 1×3 px
   repeating image. Keep if cheap, drop if not — the design survives without them.
3. **Fonts.** Bundling ~1.2 MB of OFL fonts (Noto Sans JP is the big one — subset it to the eight
   glyphs used, or render that subline with the system CJK font).

## Files
| File | Contains |
|---|---|
| `README.md` | this spec |
| `Yutani Applet v6.dc.html` + `support.js` | interactive design reference — every state reachable by clicking |
| `tokens-nostromo.css` | the palette in your `tokens.css` shape |
| `reference/sand-field.js` | the scope animation and ping-trace maths, verbatim from the mock |
| `previews/01…05.png` | 2× renders: running+connected · tunnel idle · launching (step 4, log card) · menu open with 3 clients · service stopped |
| `assets/icons/yutani-symbolic.svg`, `yutani-symbolic-16.svg` | clean symbolic marks (your package's copies carry ~7 KB of embedded metadata each — these are the same paths without it) |
| `assets/brand/logo-white.png` | the wordmark, for the header if you prefer it to Michroma |
