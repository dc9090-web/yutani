# Yutani applet popover v6 "Nostromo" — design

Source: Daniel's handoff `temp/v2.zip`, copied verbatim to
`2026-10-07-yutani-nostromo-handoff.md` (the **handoff** below). The handoff
is final on layout, spacing, type, copy and states; this document records
only the decisions taken on top of it and where each piece lives in the code.
Approved 2026-10-07.

Environment at the time: COSMIC 1.9 (cosmic-comp 1:1.9.0), GE-Proton11-7,
AMD GPU (`/sys/class/drm/card1/device/gpu_busy_percent`, hwmon `amdgpu`,
`k10temp`).

## Decisions

1. **Replace, no Skin enum.** Nostromo *is* the popover. The COSMIC-roles
   popover code is deleted. The settings window and thumbnails keep their
   COSMIC theme. The tray badge follows the handoff (7×7 square, radius 1,
   ring in the panel colour); on a light panel it keeps the theme's
   success/warning colours.
2. **Phased, one spec.** Three phases, each installed and eyeballed before
   the next:
   1. skin + every datum we already have + host gauges;
   2. ping to Tranquility;
   3. Launch EVE.
3. **Fonts bundled.** B612 Mono 400/700, Michroma 400, and a Noto Sans JP
   subset holding only `ユタニ重工` (made with `pyftsubset`, from
   `python-fonttools`). The fonts and their OFL licences go under
   `assets/fonts/`, are `include_bytes!`'d, and are loaded with
   `cosmic::iced::font::load` at applet start. If a load fails, that face
   falls back to COSMIC's monospace and the popover still renders.
4. **Scanlines and vignette are kept if cheap.** They are one cached
   `canvas` layer stacked over the popover, which ignores every event. The
   geometry is rebuilt only on resize. If it eats clicks or costs visible
   frame time, drop it — the handoff allows that.
5. **Service rocker.**
   - Off does what Quit does today (`Msg::QuitAfterTunnel`): disconnect the
     tunnel, then quit the daemon.
   - On runs `systemctl --user start yutani`.
   - Quit stays in the ⋯ menu.
   - Until the next status poll confirms the change, the rocker shows the
     amber pending look (the existing `PENDING_S` mechanism).
6. **Ping only through the tunnel.** With the tunnel down, the ping row
   reads IDLE with "—" values, as the handoff says. There is no fallback
   probe over the home link.
7. **Launch EVE waits for the client.** Steam app 8500 opens the EVE
   Launcher, and Daniel clicks Play there. So step 3 waits for a *new* EVE
   client window, with no timeout while the launcher process is alive.
8. **Hotkey hint and Hide-thumbnails button are gone** (handoff).

## Phase 1 — skin, existing data, host

### Code layout

| Unit | Change |
|---|---|
| `applet/theme.rs` | Nostromo palette as `const Color`s (`BG CARD LINE LINE_2 PHOSPHOR DIM DIMMER WHITE AMBER RED` plus the alpha variants); geometry per the handoff's mapping table; style classes for card, bracketed card, chip, row (resting/hover/focused), rocker, primary button (ready/launching/inert), overflow (closed/open), menu row and Quit row. The `VIOLET` constants and the theme-role lookups are removed. |
| `applet/fonts.rs` (new) | the `Font` handles (`MONO`, `MONO_BOLD`, `DISPLAY`, `JP`) and `load_all() -> Task`. |
| `applet/display.rs` | `Popover` is rebuilt for the new sections: `header`, `control` (service + tunnel rows with `TunnelState`), `accounts`, `network` (scope readouts, totals, ping summary, facts, uptime), `host` (three `Gauge`s), `notice`, `action` (`LaunchButton`), `menu`, `footer`. It stays a **pure function of `Status` + local samples** and is unit-tested like today. `popover_height`/`graph_fits` are recomputed for the new rows; the short-screen rule now drops the scope band. |
| `applet/rocker.rs` (new) | the 38×19 square rocker: a custom `Widget` with off/on/pending/disabled states and a 120 ms knob animation driven by redraw requests. |
| `applet/sand.rs` (new) | the sand field: a `canvas::Program` holding the 1 000 grains (the handoff's algorithm, ported from `reference/sand-field.js`), `step(dt, wind_target, up_share, live)`, and alpha quantised to 11 levels. The RNG is seeded, so tests are deterministic. |
| `applet/glass.rs` (new) | scanlines + vignette overlay (decision 4). |
| `applet/host.rs` (new) | `HostSampler`: CPU % from `/proc/stat` deltas, RAM used/total from `/proc/meminfo` (`MemTotal − MemAvailable`), GPU % from the first `/sys/class/drm/card*/device/gpu_busy_percent`, temperatures from hwmon (`k10temp` Tctl → CPU, `amdgpu` edge → GPU). Each reading is an `Option`; a missing source shows "—" and an empty gauge. Smoothing: ease 20 % per tick. The parsers take `&str`, so they are tested on fixtures. |
| `applet/ping.rs` (new, Phase 2) | the 34-sample window and its derived stats. |
| `applet/mod.rs` | new messages (`Frame(Instant)`, `HostTick`, `ServiceToggle`, `TunnelToggle`, `Launch`); the `window::frames()` subscription only while the popover is open; the host tick at 1 Hz only while open; fonts loaded in `init`. |
| `build.rs` | `YUTANI_BUILD` = `git rev-parse --short HEAD`, upper-cased, or `UNKNOWN` outside a checkout (the PKGBUILD builds from a git source, so the hash is present). |

### Data mapping (existing sources)

- **Service running:** a status reply arrived. **Stopped:** the poll failed
  (today's `None`).
- **TunnelState:**

  | State | Condition |
  |---|---|
  | NotInstalled | `!installed` |
  | Connecting | pending connect, or up with no handshake yet within `HANDSHAKE_STALE_S` |
  | StaleHandshake | `handshake_age_s ≥ HANDSHAKE_STALE_S` |
  | Connected | up with a fresh handshake |
  | Idle | otherwise |

  This is the same classification `icon.rs` already does; share it rather
  than duplicate it.
- **Location / endpoint / peer:**
  - Location: `tunnel.location`, upper-cased.
  - Endpoint: the host part of `tunnel.endpoint`.
  - Peer: `tunnel.iface`.
- **Uptime:** `up_for_s`, formatted `T+ {h}H {mm}M` / `T+ {m}M {ss}S` /
  `T+ {s}S`.
- **Throughput:**
  - The rate is the existing `rate.rs` delta at 1 Hz.
  - Session totals are `rx/tx − base`. The base is taken when `up_for_s`
    goes from `None` to `Some`, or decreases (a new session). If the applet
    starts mid-session, the base is 0, i.e. interface lifetime. That is
    accepted, because the interface is created per session.
  - KB/s are integers; MB have one decimal.
- **Accounts:** `clients` in order; `active` = focused. Clicking a row sends
  `focus <n>` (the existing request).
- **Steam notice:** `!status.steam.is_empty()`.
- **Host meta:** "{RAM total rounded} GB · 1 HZ".

### Visual verification

After install, take screenshots with `cosmic-screenshot` in these states,
each compared with `previews/01–05`:

- running + connected;
- tunnel idle;
- service stopped;
- menu open;
- a Steam-problem fixture.

## Phase 2 — ping to Tranquility

Revised 2026-10-07 after measuring. From a home far from London a TCP
connect to `tranquility.servers.eveonline.com:26000` (Cloudflare,
172.65.201.188) takes ≈16 ms over the home link — it ends at the local
Cloudflare edge — and ≈360 ms through the tunnel, whose exit is London.
The tunnel figure is the one EVE's traffic actually pays, so it is the one
shown.

- **Probe (root tunnel worker, `src/tunnel/probe.rs`):**
  - A task spawned beside the 1 s status loop, while the link is up.
  - Once a second it times a `tokio::net::TcpSocket` connect from the
    tunnel address (`bind(<conf.address>:0)`), to TQ port 26000. The
    source address is what routes it down `yutani0`, through the
    `from <address>` policy rule the exit-IP lookup already relies on, so
    no socket mark is needed.
  - Timeout 1.5 s. Success records the RTT; a timeout, refusal or failed
    resolution records a loss. The socket is dropped at once, before any
    data is sent.
  - The hostname is resolved with `tokio::net::lookup_host` and cached for
    5 min (resolved's `~eveonline.com` routing domain already sends that
    lookup through the tunnel).
- **Wire:**
  - `TunnelFile` and `TunnelStatus` gain `ping_us: Option<u32>` (the latest
    probe, whole microseconds, `None` = lost) and `ping_seq: u64`
    (incremented per probe, 0 = none yet). Both are `serde(default)`.
  - The value is an integer because `TunnelStatus` is `Eq`.
  - `assemble` passes them on only while connected.
- **Applet:**
  - `PingWindow::push_seq(seq, ms)` on every status reply while the tunnel
    is up; the window clears when the link goes down or the daemon goes
    away.
  - MIN/AVG/MAX, jitter and loss are as before.
- **Quality is relative to the session's own normal (Daniel's choice).**
  The baseline is the median of the window's successful samples. With
  `latest` the newest success:

  | Quality | Condition |
  |---|---|
  | POOR | loss > 5 %, or `latest > 1.6 × baseline + 20` ms, or no success at all |
  | DEGRADED | any loss, or `latest > 1.25 × baseline + 10` ms, or jitter > `0.15 × baseline + 5` ms |
  | NOMINAL | otherwise |
  | IDLE | tunnel down |

  The absolute thresholds in the handoff (< 30 / < 100 ms) would read POOR
  permanently from the far side of the world.
- **Tests:**
  - the probe against a local listener (success) and a closed port (loss);
  - the sequence increments and the latest value is kept;
  - serde back-compat for the new fields;
  - the quality table on fixed windows.
- **Live check:** after install, the popover's ping reads ≈360 ms NOMINAL.

## Phase 3 — Launch EVE

- **IPC:** `Request::Launch` (`launch\n`). The status reply gains
  `launch: Option<LaunchState>` (`serde(default)`).

  ```rust
  LaunchState {
      step: u8,
      steps: [StepState; 4],
      next_hotkey: String,
      failed: Option<String>,
  }
  ```

  `StepState` is `Pending | Running | Done | Failed`.
- **Daemon state machine (`launch_eve.rs`)**, advanced on the existing
  scan tick:
  1. **STEAM · APPLAUNCH 8500.** Spawn `steam steam://rungameid/8500`
     detached. Done when a Steam toplevel exists (it usually already does).
     Fail after 30 s.
  2. **STEAM · WINDOW MINIMISED.** Minimise Steam's toplevels through the
     toplevel-management protocol the backend already binds. If that is
     unsupported, mark the step Done anyway and log it, because this step is
     cosmetic.
  3. **EVE CLIENT · STARTING.** Done when a client that was not in the
     snapshot taken at step 1 is adopted. No timeout while an EVE Launcher
     process is alive. Fail if neither a launcher nor a new client appears
     within 60 s, or if the launcher exits with no new client.
  4. **HOTKEY ASSIGNED · {prefix}+{n}.** Done when the new client has its
     slot. The daemon then focuses it.
- **After the steps:**
  - The state stays visible for 2 s after the last step, then clears.
  - A failure shows on its line in red with the reason for 5 s, then
    clears.
  - Only one launch runs at a time. A second `launch` while one runs is a
    no-op that returns the current state.
- **Button:**
  - ready "▶ LAUNCH EVE";
  - launching "LAUNCHING…" / "STEP n / 4";
  - inert "START YUTANI TO LAUNCH" when the service is stopped;
  - inert "ALL 9 SLOTS IN USE" when there are 9 clients.
- **Tests:** a state-machine unit test driven with fake observations
  (toplevels, adopted clients, launcher pid alive/dead, elapsed time). The
  real run is verified live.

## Error handling

Every new reading is optional and degrades to the handoff's "—"/IDLE
copy. No panics on missing sysfs, fonts, DNS or protocols. New wire fields
are `serde(default)`, so a mismatched applet and daemon still parse each
other.

## Out of scope

The settings window, the overlay/thumbnails, multi-peer, and battery-aware
fps capping (desktop machine; the frames subscription stops when the
popover closes).
