# Steam launch-options check

**Date:** 2026-09-14
**Status:** approved design, not yet planned

## The problem

On 2026-09-14 Yutani moved from a hand-installed `/usr/local/bin/yutani`
to the pacman package's `/usr/bin/yutani`. The old binaries were deleted,
as the package spec's migration steps say to. Steam's launch options for
EVE Online (app 8500) still read

    PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%

so every press of Play failed inside `/bin/sh` before Proton ran, with
nothing on screen. Steam's `console-linux.txt` had the only evidence:

    /bin/sh: line 1: /usr/local/bin/yutani: No such file or directory

Nothing in Yutani knew the line was stale. The user found out by clicking
Play. This design makes Yutani notice first and say so where the user
already looks: the panel icon, its popover, and the settings window's
Steam page. Yutani never edits Steam's files.

## Scope

In: reading Steam's per-account `localconfig.vdf`, judging the EVE launch
line, carrying the verdict in the daemon's `status` reply, the applet's
warning dot and popover line, the settings Steam page's banner, and the
documentation fixes that the incident exposed.

Out: `yutani doctor`, rewriting Steam's config, detecting a launch line
that names a different but valid yutani build (a dev build in `target/`
is a legitimate choice), and any check of the `PROTON_*` / `WINE_*`
variables.

## 1. The check: `src/steam.rs`

A new module of pure functions plus one function that touches the
filesystem. Nothing here depends on the UI, the daemon, or the applet.

### Finding the launch line

`launch_options(localconfig: &str) -> Option<String>`

Steam's `localconfig.vdf` nests the line as
`UserLocalConfigStore → Software → Valve → Steam → apps → "8500" →
"LaunchOptions"`. The scanner does not parse VDF in general. It walks
lines the way `eve_settings::steam_libraries` does:

1. Find the first line whose trimmed text is `"8500"` **and** whose next
   non-blank line is `{`. (`apptickets` also has an `"8500"` key, but as a
   one-line key/value pair, so this rule skips it.)
2. Track brace depth from that `{` to its matching `}`.
3. Inside, the first line whose first quoted token is `LaunchOptions`
   yields its second quoted token, with VDF's `\"` and `\\` escapes undone.

Returns `None` when there is no EVE block or no `LaunchOptions` in it.

### Judging the line

```rust
pub enum Verdict {
    /// `yutani launch --` is there and the binary it names resolves.
    Ok,
    /// The line names a yutani that does not resolve: an absolute path
    /// that does not exist, or a bare `yutani` not on PATH.
    Broken { path: String },
    /// The line has no `yutani launch --` at all: EVE would start
    /// outside the tunnel.
    NoWrapper,
}

pub fn wrapper_path(launch_options: &str) -> Option<&str>
pub fn judge(launch_options: &str, resolves: &dyn Fn(&str) -> bool) -> Verdict
```

`wrapper_path` returns the token immediately before ` launch -- ` when
that token ends in `yutani` (bare name or any path), else `None`. Tokens
are split on whitespace; a quoted path with spaces is not supported, and
the README never suggests one.

`judge` maps: no wrapper → `NoWrapper`; wrapper found and `resolves(path)`
true → `Ok`; otherwise `Broken { path }`. `resolves` is injected so every
verdict has a unit test with no filesystem. The production closure:
an absolute or relative path with a `/` is checked with `Path::is_file`;
a bare name is searched on this process's `PATH`.

### Scanning every account

```rust
pub struct Finding {
    pub verdict: Verdict,
    /// The `localconfig.vdf` the verdict came from.
    pub file: PathBuf,
}

pub fn steam_roots(home: &Path) -> [PathBuf; 3]
pub fn scan(home: &Path, resolves: &dyn Fn(&str) -> bool) -> Vec<Finding>
```

`steam_roots` mirrors `eve_settings::steam_vdf_candidates` one level up:
`~/.local/share/Steam`, `~/.steam/steam`, and the Flatpak's
`~/.var/app/com.valvesoftware.Steam/.local/share/Steam`. The legacy
`~/.steam/steam` is normally a symlink to the first; `scan` canonicalises
each root and skips duplicates so one account is not reported twice.

`scan` reads `<root>/userdata/<id>/config/localconfig.vdf` for every
numeric `<id>`, skips files it cannot read and accounts with no EVE block,
and returns one `Finding` per account that has one. Only findings whose
verdict is not `Ok` matter to the callers, but `scan` returns all of them
so a test can assert an `Ok` was actually seen.

### Serialised shape

`Verdict` and `Finding` derive `Serialize`/`Deserialize`. `Verdict` uses
`#[serde(tag = "problem", rename_all = "snake_case")]`, so the status JSON
reads

    "steam": [{"problem": "broken", "path": "/usr/local/bin/yutani",
               "file": "/home/user/.local/share/Steam/userdata/424242/config/localconfig.vdf"}]

## 2. The daemon carries it

`tunnel::status::Status` gains

```rust
/// Steam accounts whose EVE launch line is not `Verdict::Ok`; empty when
/// every account is fine or Steam was not found. `serde(default)`: the
/// applet can be older or newer than the daemon.
#[serde(default)]
pub steam: Vec<steam::Finding>,
```

Only non-`Ok` findings are sent. An empty list means "nothing wrong",
which is also what an old daemon's reply deserialises to.

The daemon runs the scan through a new `steam::subscription()` batched
into `App::subscription` next to `config_watch` and `adopt`. It emits
once at startup and then every 30 s. Between emissions it re-reads a file
only when its mtime changed, so a tick normally costs one `stat` per
account. The result is kept on `App` and copied into the `Status` built
in `ui/mod.rs` for the IPC reply.

Steam rewrites `localconfig.vdf` when it exits and when Properties is
closed, so a fix made in Steam's UI shows up within 30 s. No `notify`
watch: the file is replaced rather than edited, and the polling cost is
negligible.

## 3. The applet shows it

### Icon

`applet::icon::icon_state` gains a `steam_problem: bool` argument. When
true it yields `IconState::Attention` unless the state would otherwise be
`Dim` (no daemon, or no tunnel installed): the warning dot means "Yutani
is running and something you set up is wrong", and a stopped daemon
already says all it needs to. `Attention` from the tunnel rules and from
Steam are indistinguishable on the icon; the popover explains which.

### Popover

`display::Popover` gains

```rust
/// A one-line warning above the primary action, `None` when there is
/// nothing to say.
pub notice: Option<String>,
```

built from the first non-`Ok` finding:

- `Broken`: `Steam launches EVE through <path>, which is missing. Open
  Settings → Steam.`
- `NoWrapper`: `Steam launches EVE without yutani, so it runs outside the
  tunnel. Open Settings → Steam.`

The view renders it in the theme's warning colour, in the same clipped
single-line style as `clip_note`. `popover_height` adds the row's height
when `notice` is `Some`, so the short-screen rule that drops the graph
card still measures correctly.

## 4. The settings Steam page and the docs

### Banner

`settings::State` gains `steam_findings: Vec<steam::Finding>`, refreshed
by running `steam::scan` whenever the Steam page is shown (the settings
window lives in the daemon process, so it reads the files directly and
needs no IPC). When any finding is not `Ok`, `steam_page` puts a banner
above the launch line, in the existing `ui::Role::Warning` treatment,
with the same sentence the popover uses minus the "Open Settings" tail.
The three numbered steps below it are the fix.

The bare `yutani` form stays the default line. The full-path toggle keeps
resolving `current_exe()`, which is `/usr/bin/yutani` on a packaged
install.

### Documentation and constants

- `STEAM_LAUNCH_ARGS_ABSOLUTE` in `src/lib.rs` and its doc comment name
  `/usr/bin/yutani`. The settings test that pins the old path follows.
- README: the "spell the path out" example becomes `/usr/bin/yutani`, and
  the manual-install section stops installing into `/usr/local/bin` now
  that the PKGBUILD exists (a source build is `makepkg -si`).
- `docs/superpowers/specs/2026-09-14-yutani-package-design.md`, "Migrating
  a source install": add the step "re-check each EVE account's Steam launch
  options; a line spelling `/usr/local/bin/yutani` must change to
  `/usr/bin/yutani` or bare `yutani`".

## Testing

- `steam.rs`: `launch_options` against a fixture cut from the real file
  (the `apptickets` decoy included), an escaped quote, no EVE block, an
  EVE block without `LaunchOptions`. `wrapper_path` for bare, absolute,
  and absent forms. `judge` for all three verdicts with closure fakes.
  `scan` against a temp home with two accounts, one fine and one broken,
  plus the symlinked legacy root, asserting one finding each and no
  duplicate.
- `Status` round-trips with and without the `steam` field.
- `icon_state` with `steam_problem` in every existing state.
- `display::popover` produces the two notices and `None`, and
  `popover_height` grows by the row.
- `settings`: the banner appears for a `Broken` finding and not for an
  empty list; `launch_command(true, …)` uses the resolved path.

## Error handling

Every filesystem failure in `scan` is a skipped account, never an error:
a missing Steam is not a problem Yutani should report. The subscription
never panics on a malformed file; the scanner's worst case on garbage is
`None`.
