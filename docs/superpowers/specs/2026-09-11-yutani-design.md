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
- Two layouts: free-floating draggable thumbnails, and a dock that
  auto-arranges them centred along a screen edge.
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

EVE under Steam/GE-Proton with `PROTON_ENABLE_WAYLAND=1` (measured live on
2026-09-11): the **client** window has `app_id = "exefile.exe"`, the launcher
`app_id = "eve-online.exe"`; `steam_app_8500` was only observed transiently on
the launcher during startup. Default detection therefore uses
`app_ids: ["exefile.exe", "steam_app_8500"]`. The launcher's title is
`EVE Launcher`; the client's title is `EVE` before login and
`EVE - <Character Name>` after.

Rust toolchain: stable via rustup (1.98 at time of writing).

## 2a. Supported launch configuration (primary target)

Yutani **must** work with EVE launched from **Steam** using **Proton**
(GE-Proton or Valve Proton), with `PROTON_ENABLE_WAYLAND=1 %command%` in the
game's launch options so the client is a native Wayland window. This is the
configuration on the target machine and the reason the project exists.

Concretely:

- The client window's `app_id` is `exefile.exe` (native Wayland via Wine's
  Wayland driver reports the executable name); `steam_app_8500` is kept as a
  secondary key. Both are in the default `app_ids`.
- The launcher (`EVE Launcher`) is excluded by title; each client window is
  `EVE` then `EVE - <Character Name>`.
- Multiple clients started from one launcher are separate toplevels with the
  same `app_id`; they are told apart by toplevel handle and, once logged in,
  by character name.
- The same Steam + Proton setup **without** `PROTON_ENABLE_WAYLAND` (an
  XWayland window) must also work: cosmic-comp exposes XWayland windows
  through the same toplevel and capture protocols, so no code path differs.
  This is verified, not assumed, in the acceptance checklist below.

Recommended in-game setting: **Window Mode = Fixed Window** (borderless).
In plain windowed mode Wine draws client-side decorations (title bar and
frame) that are part of the captured surface and therefore appear in the
thumbnail; Fixed Window removes them (verified 2026-09-11). A source-crop
option (`wp_viewport.set_source`) is a plan-2 candidate for users who keep
decorations — it needs a small patch to libcosmic's `Subsurface` widget.

Acceptance checklist (run before v1 is called done):

1. Steam + GE-Proton + `PROTON_ENABLE_WAYLAND=1`, one client → thumbnail
   appears at login screen, name resolves after character load, click
   focuses, hotkey focuses.
2. Same, two clients from the same launcher → two thumbnails, correct names,
   active border follows focus, `next`/`prev` cycle.
3. Same as 1 with the launch option removed (XWayland) → identical behaviour.
4. Client goes fullscreen → thumbnails still visible above it.
5. Client closes → its thumbnail disappears; relaunch → same saved position.

## 3. Architecture

One binary, `yutani`, with three roles:

- `yutani` — the app (daemon + UI). Single instance via libcosmic
  `run_single_instance` (D-Bus).
- `yutani focus <n> | next | prev | show | hide | toggle | layout <name> | settings | quit`
  — thin CLI: sends one command over the IPC socket and exits. Invoked by COSMIC
  shortcuts.
- `yutani doctor` — reports which protocols the compositor advertises and
  whether a test capture succeeds.

Inside the app, two threads sharing **one** Wayland connection with two
event queues (the pattern System76's own cosmic-workspaces uses):

```
┌─────────────────────────────┐   channel: Event (Client*/Frame)  ┌────────────────────────────┐
│  Compositor thread          │ ────────────────────────────────▶ │  UI thread (libcosmic)     │
│  2nd event queue on the     │                                   │  iced runtime, wgpu        │
│  same wl connection, driven │ ◀──────────────────────────────── │  • settings window         │
│  by calloop                 │   calloop channel: Cmd            │  • N thumbnail layer       │
│  • toplevel_info  (list)    │   (Activate, Minimize,            │    surfaces (both modes;   │
│  • toplevel_mgmt  (focus)   │    SetAppIds, SetFps)             │    dock = layout policy)   │
│  • screencopy     (capture) │                                   │                            │
│  • gbm buffer pools         │                                   │  • tray icon               │
└─────────────────────────────┘                                   └────────────────────────────┘
```

The UI recovers iced's `Connection` from the first `wl_output` event
(`Connection::from_backend`) and hands it to the backend, which calls
`registry_queue_init` on it to get its own queue and globals. The backend
deliberately does **not** bind `wl_output`, so every output handle in the
process is iced's and can be passed straight to `IcedOutput::Output`.
`cosmic-client-toolkit` is used through libcosmic's re-export `cosmic::cctk`,
which guarantees matching `wayland-client` versions.

The UI's Wayland event subscription must forward **only** the event variants
it handles (`Output`, later `Layer`). Forwarding `RequestResize` or `Frame`
as messages creates an update → redraw → event loop that pins a core
(measured: ~40 000 redraws/s).

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
      dock.rs        — dock layout policy (centred along an edge)
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
`["exefile.exe", "steam_app_8500"]`) **and** its title is not `EVE Launcher`.

Title parsing (`model::client::parse_title`):

| Title | character | state |
| --- | --- | --- |
| `EVE - Aria Vex` | `Some("Aria Vex")` | LoggedIn |
| `EVE` | `None` | LoggingIn |
| anything else with a matching app_id | `None` | LoggingIn |

The last row makes any non-launcher window of a matching app_id a client.
Besides covering EVE's transient titles, it lets the whole pipeline be tested
without EVE by temporarily setting `app_ids: ["firefox"]`.

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
   device for `dmabuf_device`, allocate a swapchain of **2** buffer objects at
   `buffer_size` in `Abgr8888` with the advertised modifiers, wrap each as a
   `wl_buffer` via `zwp_linux_dmabuf_v1` params. Fall back to `wl_shm` if
   gbm allocation fails (cosmic-workspaces does the same for some Intel GPUs).
3. Loop: take a free BO, `session.capture(buffer, damage)`. On `ready`, send
   `Frame { client_id, dmabuf: {planes(fd, offset, stride), format, modifier, w, h} }`
   to the UI as a `SubsurfaceBuffer` created from the buffer's shared
   `Arc<BufferSource>` (so the widget reuses its `wl_buffer`). The
   `SubsurfaceBufferRelease` future stays in the compositor thread; the next
   capture into that buffer is only submitted after it resolves.
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
submitted), not destroyed. *Implemented in plan 3* (`Cmd::PauseCapture` /
`ResumeCapture`, edge-triggered from surface reconciliation).

## 6. UI

### Thumbnail widget (shared by both modes)

```
container (border: border_px, colour = active ? active_border : inactive_border, radius corner_radius)
  └ stack
      ├ Subsurface(frame, content_fit: Contain, alpha: opacity)
      ├ text(character_name | "Logging in…")  bottom-left pill, hidden if !show_names
      └ pin glyph                              top-right, only when pinned (floating mode)
```

Width is `config.thumb_width`; height follows the captured window's aspect.
`CaptureState::Unavailable` replaces the subsurface with a grey placeholder
and the name.

### Floating mode

One `zwlr_layer_surface` per client: layer **Overlay**, anchor top-left,
exclusive zone 0, keyboard interactivity **None**, positioned with margins,
on the output the layout names (fallback: the output the client was first
seen on; then the primary output). Every thumbnail surface asks cosmic-comp
for rounded corners (`cosmic_corner_radius_layer_v1`, `corner_radius`), in
both modes — this is why the dock is not one wide strip: the compositor
rounds a whole layer surface, subsurfaces included, so per-thumbnail
rounding needs per-thumbnail surfaces. The radius is requested only after
the surface has presented its first frame: cosmic-comp 1.7 validates it
against the surface's current (pre-commit) bounding box, which is 0×0
before the first buffer, and a too-large radius is a fatal protocol error.

### Dock mode (default)

The same per-client surfaces as floating mode, but their positions are a
layout policy rather than the user's: on each output, the shown clients (in
layout order — by character name) form a row (Top/Bottom) or column
(Left/Right) centred along `config.dock_edge` (default Top), 8 px in from
the edge, 8 px apart; Bottom/Right align each thumbnail's far side with the
edge. The layout is recomputed and surfaces moved (`set_margin`) whenever
the set of shown clients, a thumbnail's size (first frame, hover zoom —
neighbours shift to make room), the edge or an output changes. No dragging,
no pins, positions are not persisted; clicks still activate/minimise.

### Interaction

| Input | Action |
| --- | --- |
| Left click, no drag | `Command::Activate(client)` |
| Left or right drag > 4 px | Move (floating, unpinned only). Snap to 32 px grid if `snap_grid`; snap to other thumbnails' edges within 12 px if `snap_edges`. Save `current` layout on release. |
| Right click (no drag) | `Command::Minimize(client)` — layer surfaces never receive modifier state, so Ctrl-click is not possible |
| Middle click | Toggle pin (floating mode; ignored in dock mode) |
| Hover enter / leave | resize immediately (no animation) to `thumb_width × zoom_factor` and back (no-op at the default `zoom_factor: 1.0`), growing away from the anchored corner so the thumbnail stays on screen. Applies in both modes; in dock mode the row/column is re-laid out so neighbours move aside. |

### Visibility (re-evaluated on every focus change)

- `visibility = Always` (default): shown.
- `visibility = EveFocusedOnly`: shown iff the activated toplevel is an EVE
  client or Yutani itself.
- `hide_active = true`: the activated client's own thumbnail is unmapped.
- Tray Hide / `yutani hide`: all unmapped until Show.

Unmapping never destroys layer surfaces or capture sessions (except the
`hide_active` pause in §5). Implementation note: layer-shell surfaces have no
hide/show primitive, so "unmapping" is implemented as destroy + recreate;
the client's position and pinned state are retained across the cycle and the
thumbnail reappears in the same place.

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
  app_ids: ["exefile.exe", "steam_app_8500"],
  mode: Dock,                // Floating | Dock
  dock_edge: Top,            // Top | Bottom | Left | Right
  thumb_width: 480,
  opacity: 1.0,
  fps: 30,                   // 10 | 15 | 30 | 60
  active_border: None,        // None = COSMIC theme accent (focused-window outline colour)
  inactive_border: "#404040",
  border_px: 2,
  show_names: true,
  zoom_factor: 1.0,          // 1.0 = no hover zoom
  corner_radius: 8,
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
*Status after plan 2:* `output` is saved but not yet used for placement (the
client's own output is used); `order` and `new_client_anchor` are not yet
written. Both land with named layouts in plan 4.
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
`zcosmic_overlap_notify`, per-thumbnail opacity, scroll-to-resize, source
crop insets (needs libcosmic `Subsurface` source-rect support), a COSMIC
panel applet, other compositors.
