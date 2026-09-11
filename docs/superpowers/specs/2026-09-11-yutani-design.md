# Yutani — design

**Date:** 2026-09-11
**Status:** approved

Yutani is a native Rust application for the COSMIC desktop that shows live
thumbnails of running EVE Online clients and switches between them. It exists
because every existing Linux tool (PodSight, EVE Preview Manager) can only see
X11/XWayland windows, and EVE under Proton with `PROTON_ENABLE_WAYLAND=1` is a
native Wayland window.

## 1. Goals and non-goals

Goals

- Live, low-latency thumbnails of each EVE client with zero-copy rendering.
- Click a thumbnail (or press a hotkey) to focus that client.
- Two layouts: free-floating draggable thumbnails, and a fixed dock strip.
- Multi-monitor: thumbnails may live on any output; layouts remember which.
- Look and behave like a native COSMIC app (libcosmic).
- Sized for 2–3 clients; must not be a burden at 6.

Non-goals

- Support for compositors other than cosmic-comp (no abstraction layer).
- Sending input to EVE. Yutani only ever changes focus.
- Windows/X11 support.

## 2. Platform facts (verified on the target machine, 2026-09-11)

CachyOS, COSMIC 1.7 (cosmic-comp 1.7.0), Wayland session, two outputs.
cosmic-comp advertises:

| Protocol | Used for |
| --- | --- |
| `ext_foreign_toplevel_list_v1` + `zcosmic_toplevel_info_v1` v3 | window list with title, app_id, state, geometry, outputs |
| `zcosmic_toplevel_manager_v1` v4 | `activate`, `set_minimized` |
| `ext_image_copy_capture_manager_v1` + `ext_foreign_toplevel_image_capture_source_manager_v1` | per-window capture into dmabuf |
| `zwlr_layer_shell_v1` v5 | overlay surfaces above fullscreen windows |
| `zcosmic_overlap_notify_v1` | (reserved for later use) |

There is **no** GlobalShortcuts portal in xdg-desktop-portal-cosmic 1.7.
Global hotkeys go through COSMIC's custom-shortcut config instead (§7).

EVE under Steam/GE-Proton appears as `app_id = "steam_app_8500"`. The launcher's
title is `EVE Launcher`; the client's title is `EVE` before login and
`EVE - <Character Name>` after.

Rust toolchain: stable via rustup (1.98 at time of writing).

## 3. Architecture

One binary, `yutani`, with three roles:

- `yutani` — the app (daemon + UI). Single instance via libcosmic
  `run_single_instance` (D-Bus).
- `yutani focus <n> | next | prev | show | hide | toggle | layout <name> | settings | quit`
  — thin CLI: sends one command over the IPC socket and exits. Invoked by COSMIC
  shortcuts.
- `yutani doctor` — reports which protocols the compositor advertises and
  whether a test capture succeeds.

Inside the app, two threads with two Wayland connections:

```
┌─────────────────────────────┐   channel: WindowEvent / Frame    ┌────────────────────────────┐
│  Compositor thread          │ ────────────────────────────────▶ │  UI thread (libcosmic)     │
│  own wayland-client conn    │                                   │  iced runtime, wgpu        │
│  • toplevel_info  (list)    │ ◀──────────────────────────────── │  • settings window         │
│  • toplevel_mgmt  (focus)   │   channel: Command                │  • N thumbnail layer       │
│  • screencopy     (capture) │   (Activate, Minimize,            │    surfaces (floating)     │
│  • gbm buffer pools         │    StartCapture, SetFps…)         │  • 1 dock layer surface    │
│  • IPC socket listener      │                                   │    per output              │
└─────────────────────────────┘                                   │  • tray icon               │
                                                                  └────────────────────────────┘
```

libcosmic owns its own connection and event loop. `cosmic-client-toolkit`
must own the toplevel handles and seat on *its* connection for `activate()`
to be valid, hence the second connection. dmabuf file descriptors cross the
boundary freely. This mirrors libcosmic's `sctk_subsurface_gst` example.

### Crate layout

Single crate, modules not sub-crates:

```
yutani/
  Cargo.toml
  src/
    main.rs          — arg parsing; dispatch to app / cli / doctor
    backend/
      mod.rs         — thread entry, event loop, channel plumbing
      toplevels.rs   — EVE window detection + tracking
      capture.rs     — per-window capture sessions, buffer pools
      ipc.rs         — unix socket listener → Command
    ui/
      mod.rs         — cosmic::Application impl, message routing
      thumbnail.rs   — one thumbnail (widget tree shared by both modes)
      floating.rs    — per-client layer surfaces, drag/snap/pin
      dock.rs        — per-output strip layer surface
      settings.rs    — settings window pages
      tray.rs        — StatusNotifierItem (ksni)
    model/
      client.rs      — EveClient, title parsing
      layout.rs      — Layout, ThumbPos, ordering, snapping maths
      config.rs      — Settings struct, load/save/watch
    cli.rs
    doctor.rs
  docs/superpowers/specs/
  reference/         — third-party repos for reading (gitignored)
```

Key dependencies: `libcosmic` (wayland, wgpu, tokio features),
`cosmic-client-toolkit`, `cosmic-protocols`, `wayland-client`,
`wayland-protocols`, `gbm`, `ksni`, `ron`, `serde`, `notify`, `tracing`,
`clap`.

## 4. Client detection

The compositor thread subscribes to `toplevel_info`. A toplevel is an EVE
client when its `app_id` is in `config.app_ids` (default
`["steam_app_8500"]`) **and** its title is not `EVE Launcher`.

Title parsing (`model::client::parse_title`):

| Title | character | state |
| --- | --- | --- |
| `EVE - Aria Vex` | `Some("Aria Vex")` | LoggedIn |
| `EVE` | `None` | LoggingIn |

Every title / state / geometry / output change is forwarded to the UI as a
`WindowEvent::{Added, Updated, Removed}`. Nothing polls. The `activated`
state flag identifies the focused client; it drives the active border and the
visibility rules.

Identity across sessions is the **character name**. Layout positions are
keyed by it so a saved position follows the character. A client whose name
has not resolved yet uses the layout's `new_client_anchor`, stacking
successive unknown clients 24 px down-right of each other.

## 5. Capture pipeline

Per EVE client, in the compositor thread:

1. `Capturer::create_session(CaptureSource::Toplevel(handle), paint_cursors = false)`.
2. On `formats { buffer_size, dmabuf_device, dmabuf_formats }`: open the gbm
   device for `dmabuf_device`, allocate a pool of **3** buffer objects at
   `buffer_size` in the first mutually supported format (prefer
   `Argb8888`/`Xrgb8888` with an advertised modifier), wrap each as a
   `wl_buffer` via `zwp_linux_dmabuf_v1` params.
3. Loop: take a free BO, `session.capture(buffer, damage)`. On `ready`, send
   `Frame { client_id, dmabuf: {planes(fd, offset, stride), format, modifier, w, h} }`
   to the UI. The UI wraps it in `SubsurfaceBuffer`; the returned
   `SubsurfaceBufferRelease` is sent back to the compositor thread, which marks
   the BO free when it fires.
4. Throttle: do not submit the next capture until `1 / config.fps` seconds
   have elapsed since the last submit. Capture is also damage-driven by the
   protocol, so idle windows cost nothing.
5. On new `formats` (window resized): drop the pool and reallocate.
6. On `failed(reason)`: `Stopped` → tear down, wait for next `WindowEvent`;
   `BufferConstraints` → reallocate once; `Unknown` → retry with backoff
   250 ms → 2 s, after two consecutive `Unknown` mark the client
   `CaptureState::Unavailable` (thumbnail greys out) until its next state
   change.

Capture continues for the active client (it is shown in the dock) unless
`hide_active` is on, in which case its session is paused (no captures
submitted), not destroyed.

## 6. UI

### Thumbnail widget (shared by both modes)

```
container (border: border_px, colour = active ? active_border : inactive_border, radius 4)
  └ stack
      ├ Subsurface(frame, content_fit: Contain, alpha: opacity)
      ├ text(character_name | "Logging in…")  bottom-left pill, hidden if !show_names
      └ pin glyph                              top-right, only when pinned
```

Width is `config.thumb_width`; height follows the captured window's aspect.
`CaptureState::Unavailable` replaces the subsurface with a grey placeholder
and the name.

### Floating mode

One `zwlr_layer_surface` per client: layer **Overlay**, anchor top-left,
exclusive zone 0, keyboard interactivity **None**, positioned with margins,
on the output the layout names (fallback: the output the client was first
seen on; then the primary output).

### Dock mode

One layer surface per output that has ≥ 1 EVE client, anchored to
`config.dock_edge` (default Bottom), exclusive zone 0, containing a
`row` (Top/Bottom) or `column` (Left/Right) of thumbnail widgets in layout
order. No dragging.

### Interaction

| Input | Action |
| --- | --- |
| Left click, no drag | `Command::Activate(client)` |
| Left or right drag > 4 px | Move (floating, unpinned only). Snap to 32 px grid if `snap_grid`; snap to other thumbnails' edges within 12 px if `snap_edges`. Save `current` layout on release. |
| Ctrl + left click | `Command::Minimize(client)` |
| Middle click | Toggle pin |
| Hover enter / leave | Animate size to `thumb_width × zoom_factor` and back over 120 ms, growing away from the anchored corner so the thumbnail stays on screen. Applies in both modes. |

### Visibility (re-evaluated on every focus change)

- `visibility = Always` (default): shown.
- `visibility = EveFocusedOnly`: shown iff the activated toplevel is an EVE
  client or Yutani itself.
- `hide_active = true`: the activated client's own thumbnail is unmapped.
- Tray Hide / `yutani hide`: all unmapped until Show.

Unmapping never destroys layer surfaces or capture sessions (except the
`hide_active` pause in §5).

### Settings window

libcosmic window, opened from the tray or `yutani settings`. Pages:

- **Display** — thumb width, opacity, active/inactive border colour, border
  px, show names, zoom factor.
- **Behavior** — mode, dock edge, FPS (10/15/30/60), visibility, hide
  active, snap grid, snap edges, shortcut prefix/keys, *Install shortcuts* /
  *Uninstall shortcuts* buttons.
- **Layouts** — list of saved layouts; *Save current as…*, *Apply*, *Rename*,
  *Delete*.

All changes apply live and write `config.ron`.

### Tray

StatusNotifierItem via `ksni`: Show/Hide, Layouts submenu (apply), Settings,
Quit.

## 7. Hotkeys

COSMIC custom shortcuts file:
`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom` (RON map).
*Install shortcuts* writes/replaces only entries whose action is
`Spawn("yutani …")`:

```ron
(modifiers: [Ctrl, Alt], key: "1"): Spawn("yutani focus 1"),
…
(modifiers: [Ctrl, Alt], key: "9"): Spawn("yutani focus 9"),
(modifiers: [Ctrl, Alt], key: "Right"): Spawn("yutani next"),
(modifiers: [Ctrl, Alt], key: "Left"):  Spawn("yutani prev"),
```

cosmic-comp reloads this file on change. *Uninstall* removes those entries.
`focus <n>` selects the n-th client in **layout order**: dock order, or
floating thumbnails sorted by (output, y, x). `next`/`prev` cycle that order
from the currently active client (wrapping); if no client is active, `next`
goes to the first.

## 8. IPC

Unix socket `$XDG_RUNTIME_DIR/yutani.sock`, mode 0600, removed on exit.
Newline-delimited text; one command per connection.

Commands: `focus <n>`, `next`, `prev`, `show`, `hide`, `toggle` (show if hidden,
hide if shown),
`layout <name>`, `settings`, `quit`.
Reply: `ok\n` or `err <message>\n`. The CLI prints the error and exits 1;
if the socket is absent it prints "yutani is not running" and exits 1.

## 9. Config and layouts

`~/.config/yutani/config.ron`:

```ron
(
  app_ids: ["steam_app_8500"],
  mode: Floating,            // Floating | Dock
  dock_edge: Bottom,         // Top | Bottom | Left | Right
  thumb_width: 320,
  opacity: 0.9,
  fps: 30,                   // 10 | 15 | 30 | 60
  active_border: "#ff8800",
  inactive_border: "#404040",
  border_px: 2,
  show_names: true,
  zoom_factor: 1.5,
  visibility: Always,        // Always | EveFocusedOnly
  hide_active: false,
  snap_grid: true,
  snap_edges: true,
  shortcuts: ( focus_prefix: [Ctrl, Alt], next: "Right", prev: "Left" ),
)
```

Written by the settings UI; watched with `notify` so hand edits apply live.
Unparseable config → log a warning, run with defaults, never overwrite the
user's file.

`~/.config/yutani/layouts/<name>.ron`; `current.ron` is auto-saved on every
drag, pin, or order change:

```ron
(
  thumbs: {
    "Aria Vex":   ( output: "DP-1", x: 40,  y: 40, pinned: true ),
    "Kel Draven": ( output: "DP-1", x: 380, y: 40, pinned: false ),
  },
  order: ["Aria Vex", "Kel Draven"],
  new_client_anchor: ( output: "DP-1", x: 40, y: 40 ),
)
```

Outputs are referenced by connector name. If a layout names an output that
is not connected, those thumbnails use the primary output at the same x/y.
Applying a named layout copies it to `current.ron`.

## 10. Errors and logging

- `tracing` to stderr; `RUST_LOG` honoured. The desktop entry runs
  `yutani` with stdout/stderr appended to `~/.local/state/yutani/yutani.log`.
- Required protocol missing at startup → print which one and exit 2.
- Wayland connection lost → exit 1 (the session restarts autostart apps).
- Config/layout parse errors → warn and fall back to defaults; never
  overwrite the offending file.
- Capture failures → per-client, see §5; never fatal.

## 11. Testing

- `model/` is pure and unit-tested: title parsing, layout order, snapping
  (grid + edges), `next`/`prev` wrapping, config and layout RON round-trips,
  output fallback.
- `backend/ipc.rs` command parsing is unit-tested.
- Backend and UI are verified manually against the live session; `yutani
  doctor` checks the environment.

## 12. Out of scope for v1 (candidates for later)

Persistent per-character hotkeys, hide-when-overlapped via
`zcosmic_overlap_notify`, per-thumbnail opacity, scroll-to-resize, a COSMIC
panel applet, other compositors.
