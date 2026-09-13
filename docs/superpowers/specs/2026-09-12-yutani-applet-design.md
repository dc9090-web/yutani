# Yutani — COSMIC panel applet (design)

**Status:** approved (Daniel, 2026-09-12)
**Depends on:** plan 4 (IPC), `2026-09-12-yutani-tunnel-design.md` (the `status` request and `tunnel connect|disconnect`)
**Visual source of truth:** `design/CachyOS Cosmic Wayland icon.zip` → `design_handoff_y_wireguard_applet/README.md` (copy, colours, spacing, states are final there; this spec only maps them onto our architecture). Icons in that bundle are production assets.

## 1. Goal

Replace the `ksni` StatusNotifierItem with a real COSMIC panel applet: the
Y mark in the panel, and on click the designed popup — tunnel status and
location, accounts (EVE clients) connected, tunnel IP and handshake age,
upload/download totals and rates, and the menu (Connect/Disconnect tunnel,
Accounts…, Preferences…, Quit). The applet holds no state of its own: it
renders what the Yutani daemon reports over IPC and sends actions back.

Non-goals: working without the daemon (it shows an offline state and a
"Start Yutani" item instead); non-COSMIC panels (the ksni path is removed,
not kept as a fallback — decided 2026-09-12).

## 2. Architecture

```
cosmic-panel ──spawns──▶ yutani-applet (libcosmic applet, own process)
                              │  IPC socket ($XDG_RUNTIME_DIR/yutani.sock)
                              ▼
                         yutani (daemon): status / focus / show / hide / toggle /
                                          tunnel connect|disconnect / quit
```

- `yutani-applet` is a second binary target of the same crate. The crate
  gains `src/lib.rs` exposing the modules both binaries need (`ipc`, the
  status types, `model::config`); `src/main.rs` becomes a thin `yutani`
  entry and `src/bin/yutani-applet/` holds the applet.
- The applet uses libcosmic's applet API (`cosmic::applet::run`,
  `Core::applet` helpers for the panel button and the popup surface). The
  popup is a shell-owned surface (position, blur, radius, shadow supplied by
  libcosmic — per the handoff, only the *contents* are ours).
- **Popup width is 360 px, not the handoff's 336.** `Core::applet
  ::popup_container` pins its autosize limits to `min_width(360) …
  max_width(360)`, so the width is libcosmic's to give, not ours to ask
  for. Every section fills that width; the handoff's paddings are unchanged.
  The same limits cap the height at 1000 px **by clipping**, which is why
  the expanded account list scrolls inside its own capped container rather
  than being allowed to push the rows below it off the bottom.
- Polling: `status` every 1 s while the popup is open, every 5 s while
  closed (icon state only). Rates are deltas of `rx_bytes`/`tx_bytes`
  between polls divided by elapsed seconds. Totals are cumulative and stay
  frozen at the last value while disconnected (they are counters).
- Actions map 1:1 to IPC requests. After an action the applet polls
  immediately.

## 3. Panel button and icon states

Icon name `y-symbolic`, tinted **pure white** on a dark panel theme and
with the theme's `on_bg` ink on a light one (`applet::theme::mark_class`;
the panel's default `icon_color` is a soft grey that read dull — changed
2026-09-13). `yutani applet install`
copies the five SVGs into `~/.local/share/icons/hicolor/` (the four
symbolic marks to `symbolic/apps/`, the full-colour `y-color.svg` to
`scalable/apps/`), runs `gtk-update-icon-cache` if present, and prints the
one manual step: *Settings → Desktop → Panel → Applets → add "Yutani"*. It
also writes two `.desktop` files into `~/.local/share/applications/`:

- `com.yutani.Applet.desktop` — `X-CosmicApplet=true`, `NoDisplay=true`,
  `Exec=<abs yutani-applet>`: what the panel needs to list the applet.
- `com.yutani.Yutani.desktop` — an ordinary visible launcher named
  "Yutani", `Icon=y-color` (the `scalable/apps` copy above),
  `Categories=Game;Utility;`, `Exec=<abs canonicalised yutani> start` (through
  the systemd user unit when `yutani service install` has written one, so a
  crash restarts the daemon; in-process otherwise): what puts
  Yutani in the Applications list, so the daemon can be started without a
  terminal. `update-desktop-database` is run afterwards if present, so it
  appears without a re-login.

`applet uninstall` removes both, and the icons.

| Daemon / tunnel state | Icon | Treatment (handoff table) |
|---|---|---|
| daemon running, tunnel connected & handshake < 180 s, **≥ 1 EVE client** | `y-symbolic` + dot | the tinted mark with a blue `#0A5CFF` dot over its bottom-right corner (drawn by the applet, not an icon file) |
| daemon running, tunnel connected & handshake < 180 s, no client running | `y-symbolic` | plain |
| daemon running, tunnel disconnected (deliberately) | `y-symbolic` | plain |
| tunnel connecting/disconnecting (unit activating/deactivating, ≤ 10 s) | `y-sync-symbolic` | spinner badge, 1.1 s rotation |
| tunnel up but no handshake for ≥ 180 s (including one that never handshaked at all), or unit failed | `y-attention-symbolic` | attention badge |
| daemon not running / tunnel not installed | `y-symbolic` | 38 % opacity |

Button 26×26, radius 7, hover/active fills per handoff; sibling gap is the
panel's.

The Active row is the one departure from the handoff, which reserves
`y-color` for the launcher. It is deliberate (approved 2026-09-13; dot form
requested 2026-09-13 afternoon in place of the earlier all-blue mark): the
panel says at a glance whether Yutani is *doing* its job — tunnel up,
healthy, and EVE behind it — not merely ready to. It is drawn as the plain
`y-symbolic` mark, tinted by the panel like every other state, with a blue
dot (`#0A5CFF`, the launcher's blue, 1 px near-black ring, half the icon
height clamped to 7–12 px) laid over the bottom-right corner by the
applet's view (`IconState::badge`), so the Y itself still follows a light
panel theme. `y-color` stays a launcher-only icon under `scalable/apps`.
`icon_state` therefore takes the client count
(`icon_state(tunnel, clients, pending)`): pending, a failed unit and a
stale (or missing) handshake all outrank it, and 0 clients falls back to
the plain mark.

Two `TunnelStatus` fields exist for this table and nothing else, both
`#[serde(default)]` so an older daemon's `status` reply still parses in a
newer applet (they are separate binaries and upgrade independently):

- `up_for_s: Option<u64>` — seconds since the link came up
  (`TunnelFile::since_unix`), `None` while down. It bounds the sync badge: a
  link that is up with *no* handshake is settling only while `up_for_s <
  180`; past that it is not handshaking, it is broken, and the icon goes to
  attention. A reply without the field keeps the pre-`up_for_s` behaviour
  rather than raising a false alarm.
- `failed: bool` — the unit is installed, the interface is absent, and
  `systemctl --no-ask-password is-failed yutani-tunnel.service` exits 0
  (only a zero exit is the failed state; 3 is inactive, 4 is no such unit).
  Read-only and unprivileged, bounded by `proc::output_with_timeout` at 1 s
  because it runs on every `status` request, and short-circuited so the
  common paths never spawn it. It outranks every link check in the icon
  table. The popup's status text stays `Disconnected` — the handoff defines
  no failed colour, so nothing else in the popup changes.

## 4. Popup contents (360 px wide — see §2)

Vertical order and copy exactly as the handoff; data sources:

1. **Header** — Y mark 24 px; "WireGuard"; status dot + text: `Connected`
   (accent `#2FD6B0`, glow) when `tunnel.connected` and handshake age
   < 180 s, else `Disconnected` (`#8A8F98`); `·` ; pin glyph ;
   `tunnel.location`. Right chip: `tunnel.iface` (`yutani0`).
2. **Accounts band** — count = `clients.len()`; label "Account connected" /
   "Accounts connected"; right column: the **tunnel IP**, meaning the
   *public* address EVE is seen at — `tunnel.exit_address` when the worker
   has one, else the internal `tunnel.address` (10.2.0.2, all the applet
   has before the first lookup or against an older daemon), else `—`;
   `hs 21s ago` from `handshake_age_s`, `hs —` when null/disconnected.

   `exit_address` is `Option<String>` on both `TunnelFile` and
   `TunnelStatus` (`#[serde(default)]`, like `up_for_s`/`failed`), and
   `None` whenever the link is down. The root worker fills it with
   `curl -4 -sS -m 6 --interface <tunnel address> https://api.ipify.org`
   through `proc::output_with_timeout` (8 s cap): `--interface 10.2.0.2`
   gives the request the source address the policy rule `from 10.2.0.2`
   matches, so the query goes down `yutani0` and the exit node answers it.
   It runs when the link comes up, when the peer's first handshake lands,
   and every 300 s after; only a bare IPv4 answer is accepted, and any
   failure keeps the previous value (debug-logged) rather than blanking the
   band.
3. **Traffic tiles** — Upload (`tx_bytes`, accent `#2FD6B0`) and Download
   (`rx_bytes`, `#5B9BFF`): total + rate; formatting per the handoff
   (≥1e9 `X.XX GB`, ≥1e6 `X.X MB`, else `N KB`; rates `X.X MB/s` / `N KB/s`,
   `0 KB/s` while disconnected). Tabular numerals (monospace font for all
   numbers).
4. **Menu**
   - `Disconnect tunnel` / `Connect tunnel` (hint `wg-quick` in the handoff
     → we show `systemd`, the truthful hint) → `tunnel disconnect|connect`.
     Disabled with hint `not installed` when `tunnel.installed` is false.
   - `Accounts…` (trailing count) → expands in place to one row per client:
     name, active dot; click → `focus <n>` where `n` is the row's 1-based
     index in the daemon's layout order (the `status` reply lists clients in
     that order). A second click on the header collapses it.
   - `Preferences…` → `xdg-open ~/.config/yutani/config.ron` (creates the
     file with defaults first if absent) until plan 5's settings window
     exists, then `settings` over IPC.
   - Thumbnails row (addition to the handoff, needed to replace the old
     tray): `Hide thumbnails` / `Show thumbnails` → `hide`/`show`.
5. **Quit** (`#FF8A7E`, hover `#FF6B5C1F`) → IPC `quit`: the daemon
   disconnects the tunnel if it is up, then exits; the applet stays in the
   panel and switches to the daemon-offline state, whose menu has a single
   `Start Yutani` item (spawns `yutani` detached). Quit means "shut
   everything down" — leaving the tunnel up with nothing left to manage it
   would be a surprise, so the daemon runs `systemctl stop` first. It
   answers `ok` before doing so (the stop can take the unit's 10 s
   TimeoutStopSec and nothing may block on it), and the applet degrades its
   shown tunnel state to disconnected the moment the press succeeds rather
   than claiming a live link for those 10 s.

Typography — **decided**: the handoff asks for Space Grotesk and JetBrains
Mono and allows substituting the codebase's own fonts; we substitute, and
ship neither. UI text is COSMIC's UI font (`cosmic::font::default()`, with
`Weight::Medium` for the handoff's "500"); every number, id, rate and
interface name is `cosmic::font::mono()`, which resolves to whatever
monospace the system has. Nothing is bundled and nothing is downloaded, so
the applet inherits the user's font settings the way every other COSMIC
applet does. Line heights are set per helper: libcosmic's `monotext` preset
pins an *absolute* 20 px line box, which at 10.5–22 px would wreck every gap
in the popup, so each text helper passes its own factor.

Colours are the handoff's tokens, used as literals in a `theme.rs` table so
they're in one place. One token is *derived* rather than taken from the
handoff — `HELD_HOVER_FILL` `#FFFFFF29`, the hover fill of the expanded
`Accounts…` header, because the handoff has no row that is both held and
hovered and the ordinary hover fill would make such a row darken under the
pointer. The popup's corner radius follows the COSMIC theme's `radius_m`
(what libcosmic rounds the container behind it to), with the handoff's 14 as
the fallback.

## 5. IPC additions

Already specified in the tunnel design: `status` → `ok <json>` with
`clients[]` (in layout order, with `active`), `hidden`, and `tunnel{…}`.
`tunnel connect|disconnect`. The applet needs nothing else; `focus`,
`show`, `hide`, `quit` exist from plan 4. Parsing on the applet side uses
`serde_json` (new direct dependency; already in the lock via libcosmic —
verify) against a shared `Status` struct in the lib.

Daemon-offline detection: connect failure on the socket → offline state;
retried on the poll cadence.

## 6. Removal of the ksni tray

`src/ui/tray.rs`, the `ksni` dependency and `Msg::Tray` go away; visibility
toggling is already routed through `set_hidden` (plan 4 Task 3), so the
only tray-specific code left is the subscription. The main spec §6 "Tray"
paragraph is replaced by a pointer to this document.

## 7. Errors

- Socket errors → offline state, never a crash; a poll that fails after a
  success keeps the last totals and shows `Disconnected`.
- Action `err …` replies surface as a one-line note under the menu item
  for 3 s (e.g. `no client 3 (2 known)` — can't normally happen since the
  list came from the daemon).
- `xdg-open` missing → note `open ~/.config/yutani/config.ron manually`.

## 8. Testing

Pure, unit-tested in the lib: byte/rate formatting table (`413.1 MB`,
`2.79 GB`, `222 KB/s`, `0 KB/s`); `Status` JSON round-trip; icon-state
selection from a `Status` + age threshold; rate derivation from two samples
(incl. counter reset → rate 0, not negative).

Hands-on: applet appears after `yutani applet install` + panel add; icon
states cycle through disconnected → sync → connected when clicking Connect;
tiles move while EVE is in game; Accounts… lists characters and focuses
them; Quit switches to the offline state and Start Yutani brings it back;
a light COSMIC theme still tints the icon correctly (symbolic).

## 9. Out of scope

Traffic graphs; per-client traffic; multiple tunnel locations; notification
on handshake loss (candidate: a desktop notification after 180 s).
