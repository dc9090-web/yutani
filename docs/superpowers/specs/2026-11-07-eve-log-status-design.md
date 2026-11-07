# EVE log status: system and hostile-activity on the thumbnails

Date: 2026-11-07. Status: approved.

## Goal

Each thumbnail shows which solar system its character is in and lights up
when that character is being shot, tackled, jammed or neuted. The data
comes from the files the EVE client writes while it runs:
`Documents/EVE/logs/Gamelogs/*.txt` and `Documents/EVE/logs/Chatlogs/Local_*.txt`
inside the Proton prefix.

Shield, armour and hull levels are **not** in the logs and are out of
scope. The "under fire" signal is derived from incoming hits.

## What the logs contain (verified on this machine, 2026-11-07)

Game log: UTF-8, CRLF, a four-line header then one event per line:

```
------------------------------------------------------------
  Gamelog
  Listener: TrilliumONE
  Session Started: 2026.11.07 02:25:00
------------------------------------------------------------
[ 2026.11.07 09:09:53 ] (combat) <color=0xffcc0000><b>50</b> <color=0x77ffffff><font size=10>from</font> <b><color=0xffffffff>Pith Extinguisher</b><font size=10><color=0x77ffffff> - Scourge Torpedo - Hits
[ 2026.11.07 09:09:52 ] (combat) Pithum Abolisher misses you completely
[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora
[ 2026.11.07 03:40:12 ] (None) Undocking from Jita IV - Moon 4 - Caldari Navy Assembly Plant to Jita solar system.
[ 2026.11.07 04:01:00 ] (notify) Docking operation already in progress.Estimated time left: 9 seconds.
[ 2026.11.07 05:12:03 ] (combat) Warp scramble attempt from Emergent Preserver to you!
[ 2026.11.07 05:12:03 ] (combat) Warp disruption attempt from Blood Raiders Cruor to you!
[ 2026.11.07 05:13:10 ] (combat) You're jammed by Guristas Saboteur - Guristas Saboteur
[ 2026.11.07 05:13:11 ] (combat) 36 GJ energy neutralized Awakened Keeper - Awakened Keeper
[ 2026.11.07 05:13:12 ] (combat) -12 GJ energy drained to Blood Raiders Cruor - ♦ Cruor
```

The first game log of a session, written before login, has no `Listener`
line and no events. Timestamps are EVE time (UTC). The filename ends in the
character ID (`20261107_022500_669539978.txt`).

Local chat log: UTF-16LE, a header block, then lines each beginning with a
byte-order mark:

```
          Channel ID:      local
          Channel Name:    Local
          Listener:        TrilliumONE
          Session started: 2026.11.07 02:25:00
﻿[ 2026.11.07 09:20:47 ] EVE System > Channel changed to Local : Korsiki
```

Direction is carried by the leading colour tag, not the words:

| colour | meaning |
| --- | --- |
| `0xffcc0000` | damage **to you** (`N from <attacker> - <weapon> - <quality>`) |
| `0xff00ffff` | damage you dealt (ignored) |
| `0xffe57f7f` | energy **taken from you**: `N GJ energy neutralized <attacker> - ...` or `-N GJ energy drained to <attacker> - ...` |
| `0xff7fffff` | energy you took (ignored) |

Tackle lines: `<b>Warp scramble attempt</b> ... from ... <b>NAME</b> ... to ... you!`
(or `Warp disruption attempt`); a line ending in something other than
`you!` is tackle on someone else and is ignored. Jam: `You're jammed by
<attacker> - <attacker>`. Attacker names may carry nested markup (ship
type in `<font><color>(…)</color></font>`, corp and alliance tags); the
parser strips every `<…>` tag and HTML entity before keeping a name.
The dock line contains `<br>` in the raw file.

## Architecture (approach A)

A log-tailing subscription inside the daemon, built like `config_watch` and
`steam::subscription`. No new process. Three units:

1. `yutani::eve_logs::parse` — pure: one line in, `Option<LogEvent>` out.
2. `yutani::eve_logs::tail` — file discovery, per-file offsets, decoding.
3. `yutani::eve_logs::subscription` — iced `Subscription<LogEvent>` driving
   `tail` on the blocking pool, woken by `notify` on both folders with a
   500 ms fallback poll.

The UI keeps a per-client `Status` (`src/ui/status.rs`, pure) and the
thumbnail view reads it.

### Discovery

`eve_logs::discover(override_dir, home)` returns the `logs` directory:
`<library>/steamapps/compatdata/8500/pfx/drive_c/users/steamuser/Documents/EVE/logs`,
walking Steam libraries exactly as `eve_settings::discover` does. The
`eve_logs_dir` config field overrides it. No directory: the feature is off,
`yutani doctor` says where it looked.

### Which files

On start and on every directory event, scan `Gamelogs/*.txt` and
`Chatlogs/Local_*.txt`. Skip files not modified in the last hour. Read each
candidate's header for `Listener:`; skip files without one. Keep the newest
(by `Session started`) game log and Local log per character name.

### Tailing

Each tracked file holds a byte offset. First open: offset = end of file,
except the Local log is also scanned once backwards for its last
`Channel changed to Local :` line so the system is known immediately. The
same backfill applies to a game log: its last `Jumping from`/`Undocking
from` line, used only if the Local log gave nothing. Each pass reads the
bytes from the offset up to the last line feed (`0x0A` for UTF-8; `0A 00` at
an even offset for UTF-16LE), decodes, parses each line, advances the offset
by the consumed byte count. A partial final line waits for the next pass.
A file that shrinks (new session reusing a name never happens, but be safe)
resets its offset to 0.

### Events

```rust
pub struct LogEvent { pub character: String, pub at: EveTime, pub kind: EventKind }
pub enum EventKind {
    System(String),
    Docked,                       // "Docking operation already in progress"
    Undocked(String),             // system name; also sets System
    Damage { amount: u32, from: String, weapon: Option<String> },
    Scrambled(String),
    Disrupted(String),
    Jammed(String),
    Neuted { gj: u32, from: String },
}
```

`at` is the `[ YYYY.MM.DD HH:MM:SS ]` stamp parsed to seconds since epoch
(UTC). Lines that match nothing yield `None`. Parsing is plain string
matching, no regex crate.

### Status (UI)

```rust
pub struct Status {
    pub system: Option<String>,
    pub docked: bool,
    hits: VecDeque<(Instant, u32)>,   // last 5 s of incoming hits
    tackled_until: Option<(Instant, Tackle)>,  // Scram or Point, 10 s hold
    jammed_until: Option<Instant>,    // 5 s hold
    neuted_until: Option<Instant>,    // 5 s hold
}
```

`apply(event, now)`, `tick(now)` to expire, `dps(now) -> Option<u32>`
(sum of hits in window / 5, `None` when empty), `under_fire(now)`,
`tags(now) -> Vec<&'static str>` in the order SCRAM/POINT, JAM, NEUT.

Events route to the `Client` whose `Login::LoggedIn(name)` equals
`event.character`; others are dropped. A 1 Hz `StatusTick` message runs
while any client has a non-empty status (active hold or hit window), so an
idle desktop pays nothing.

### Thumbnail

Caption below the name, same label box:

```
TrilliumONE
Korsiki · 240 dps · SCRAM
```

Second line = system name (or `Docked · <system>` when docked), then
`<n> dps` while under fire, then the active tags. Absent entirely when the
system is unknown and nothing is active. Shown when `show_names` and the
new `show_status` are both true.

Border: when `under_fire || tackled`, colour `#e11d48` at the configured
width, overriding active/inactive. When the border is switched off
(`show_border = false`) the tags alone carry the signal.

### Config

- `show_status: bool`, default `true` — Thumbnails page toggle.
- `eve_logs_dir: Option<String>`, default `None`, must be absolute — path
  field on the Thumbnails page next to the existing override. A reload
  restarts the subscription with the new path.

### Errors

Missing or unreadable folder: feature off, one warning, doctor line. Line
that does not parse: dropped. File gone: forgotten on next scan. Decode
error: skip the chunk, move the offset past it. Nothing here may panic or
stall the UI runtime; all file I/O runs on the blocking pool.

### Doctor

`yutani doctor` prints `EVE logs: <dir>` with the count of tracked
characters, or the discovery error.

## Tests

- `parse`: one test per event kind with verbatim lines from this machine's
  logs, plus outgoing-damage, misses, your-own-neut, and a chat line that
  is not a system change all yielding `None`; timestamp parse.
- `tail`: UTF-16LE with per-line BOM and a trailing partial line; UTF-8
  partial line; append-between-reads returns only the new complete lines;
  backfill finds the last system line.
- discovery: header `Listener` parse; newest-per-character selection; the
  no-Listener stub skipped.
- `Status`: dps over the window; each hold expires; tags order; unknown
  character dropped at the routing layer.
- caption formatting for each combination.

Live verification: install the package while clients are running and
watch the thumbnails.
