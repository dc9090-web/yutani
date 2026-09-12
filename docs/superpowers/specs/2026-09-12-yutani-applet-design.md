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
- Polling: `status` every 1 s while the popup is open, every 5 s while
  closed (icon state only). Rates are deltas of `rx_bytes`/`tx_bytes`
  between polls divided by elapsed seconds. Totals are cumulative and stay
  frozen at the last value while disconnected (they are counters).
- Actions map 1:1 to IPC requests. After an action the applet polls
  immediately.

## 3. Panel button and icon states

Icon name `y-symbolic` (tinted by the panel). `yutani applet install`
copies the five SVGs to `~/.local/share/icons/hicolor/symbolic/apps/`,
runs `gtk-update-icon-cache` if present, and prints the one manual step:
*Settings → Desktop → Panel → Applets → add "Yutani"*. It also writes the
applet's `.desktop` file (`~/.local/share/applications/
com.yutani.Applet.desktop`, `X-CosmicApplet=true`, `Exec=<abs
yutani-applet>`) that the panel needs to list it.

| Daemon / tunnel state | Icon | Treatment (handoff table) |
|---|---|---|
| daemon running, tunnel connected & handshake < 180 s | `y-symbolic` | plain |
| daemon running, tunnel disconnected (deliberately) | `y-symbolic` | plain |
| tunnel connecting/disconnecting (unit activating/deactivating, ≤ 10 s) | `y-sync-symbolic` | spinner badge, 1.1 s rotation |
| tunnel up but no handshake for ≥ 180 s, or unit failed | `y-attention-symbolic` | attention badge |
| daemon not running / tunnel not installed | `y-symbolic` | 38 % opacity |

Button 26×26, radius 7, hover/active fills per handoff; sibling gap is the
panel's.

## 4. Popup contents (336 px wide)

Vertical order and copy exactly as the handoff; data sources:

1. **Header** — Y mark 24 px; "WireGuard"; status dot + text: `Connected`
   (accent `#2FD6B0`, glow) when `tunnel.connected` and handshake age
   < 180 s, else `Disconnected` (`#8A8F98`); `·` ; pin glyph ;
   `tunnel.location`. Right chip: `tunnel.iface` (`yutani0`).
2. **Accounts band** — count = `clients.len()`; label "Account connected" /
   "Accounts connected"; right column: `tunnel.address` or `—`; `hs 21s ago`
   from `handshake_age_s`, `hs —` when null/disconnected.
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
5. **Quit** (`#FF8A7E`, hover `#FF6B5C1F`) → IPC `quit`; the applet stays
   in the panel and switches to the daemon-offline state, whose menu has a
   single `Start Yutani` item (spawns `yutani` detached).

Typography: COSMIC's UI font (the handoff allows substituting the
codebase's font); monospace (`JetBrains Mono` if installed, else the system
monospace) for numbers, IDs, rates. Colours are the handoff's tokens, used
as literals in a `theme.rs` table so they're in one place.

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
