# Yutani — Applet: services band and a flat character list

**Status:** requested by Daniel 2026-09-14 ("show the current active
character without a drop down"; "show running services with green and red
dots: Yutani running and WireGuard service running"); design approved the
same day. Companion to the applet spec (`2026-09-12-yutani-applet-design.md`,
§4), which this amends. Plan: `docs/superpowers/plans/2026-09-14-yutani-applet-services-characters-opacity.md`.

The third request of the day — "I can't see the opacity slider" — is the
already-approved, never-built `2026-09-13-yutani-opacity-and-centre-design.md`
§1; the same plan implements it. §2 of that spec (Centre vertically) stays
pending.

## 1. Services band

A new section directly under the header, in every state (online or not),
above the accounts band. Two rows, each: a status dot on the left, the
service name, and a short mono note against the right edge.

| Row | Green when | Note (green) | Note (red) |
|---|---|---|---|
| **Yutani** | the daemon answered the last `status` poll | `running` | `not running` |
| **WireGuard** | the tunnel link is up *and* the handshake is fresh (< 180 s) — the header's own "Connected" | `connected` | `no handshake` (link up, handshake stale); `failed` (unit failed); `not installed`; otherwise `disconnected` |

Green is the accent `#2FD6B0` (`ACCENT_UP`), red the danger red `#FF8A7E`
(`DANGER_TEXT`), both existing tokens; no glow (the header's dot keeps it).

While the daemon is offline the applet has no `status`, so the WireGuard
row reads the worker's own state without root: `/run/yutani/tunnel.json`
(`up`) and whether `yutani0` exists — `connected` when both, else
`disconnected`; `not installed` when the unit file is absent. No `systemctl`
call in that path (a `systemctl is-failed` per poll while offline was a
known cost to avoid), so `failed` is only ever reported through the daemon.

Data lives in `applet::display::Display` as `services: [Service; 2]`
(`Service { name, up, note }`), computed in `display()` from the status
(or, offline, from a `link_up`/`installed` pair the applet reads), so the
view renders it and decides nothing.

## 2. Characters: a flat list

The `Accounts…` header, its expand/collapse (`accounts_open`,
`Msg::ToggleAccounts`, `toggles_accounts`, the held-row fill
`HELD_HOVER_FILL`/`hover_fill`) go away. The menu becomes:

1. one row per running character, always visible, in the daemon's layout
   order: dot on the left (`ACCENT_UP` for the focused character,
   `TEXT_MUTED` for the rest), name, click → `focus <n>`; kept in the
   existing scrolling list (`ACCOUNTS_LIST_MAX_PX`) so a long roster cannot
   push Preferences… and Quit off the popup. With no characters, one
   disabled row `No characters running`.
2. `Disconnect tunnel` / `Connect tunnel` (unchanged)
3. `Preferences…`, `Hide/Show thumbnails`, then the divider and `Quit`
   (unchanged).

Character rows keep their indent (`MENU_ACCOUNT_PAD`): the dot is what
distinguishes them from the action rows below. The accounts band above the
tiles is unchanged (count, exit IP, handshake age).

Offline: the single `Start Yutani` row, as before, under the services band.

## 3. Thumbnail opacity

As `2026-09-13-yutani-opacity-and-centre-design.md` §1, unchanged:
`thumb_opacity: u8` percent (20..=100, default 100), a live-only slider
between Hover zoom and Show character names, `Subsurface::alpha` for the
image (compositor `wp_alpha_modifier_v1`), and the border, name label, pin
and placeholders multiplied by the same factor; a hovered thumbnail is
fully opaque.

## 4. Tests

- `menu::rows`: online with 3 clients → the 3 character rows first (dot
  states, 1-based focus), then tunnel, Preferences…, thumbnails, Quit; no
  clients → one disabled `No characters running`; offline → `Start Yutani`.
- `display::display`: the services pair for online-connected,
  online-stale-handshake, online-failed, online-not-installed,
  online-disconnected, offline-link-up, offline-link-down,
  offline-not-installed.
- theme: the new paddings are on the handoff's spacing scale; the
  held-row tokens are gone.
- Opacity tests as in its own spec §1.5.

## 5. Out of scope

Centre vertically (§2 of the opacity spec); per-character opacity; any
change to the CLI or the daemon's IPC.
