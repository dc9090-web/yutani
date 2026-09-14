# Plan — applet services band, flat character list, thumbnail opacity

Specs: `docs/superpowers/specs/2026-09-14-yutani-applet-services-and-characters-design.md`
and `docs/superpowers/specs/2026-09-13-yutani-opacity-and-centre-design.md` §1.
TDD per task; one commit per task; `cargo test` green and the release build
warning-free before each commit. Never `cargo fmt`.

## Task 1 — config `thumb_opacity`
`src/model/config.rs`: field `thumb_opacity: u8` (doc: percent, 20..=100),
default 100, `validate` check `(20..=100)`. Tests: default; 0/19/101/255 →
100; 20 and 100 survive; the stale `opacity` key is still ignored.

## Task 2 — settings slider
`src/ui/settings.rs`: `Msg::Opacity(u8)`, live-only; `apply_config_field`
clamps to 20..=100; Display page slider `Thumbnail opacity: N%` (20..=100,
step 1, `on_release(Msg::Commit)`, width 260) between zoom and names.
Tests: apply (80 → true, 80 again → false, 5 → 20, 200 → 100); live-only.

## Task 3 — thumbnail rendering
`src/ui/thumbnail.rs`: `pub fn opacity(config, hovered) -> f32`,
`with_opacity(Color, f32) -> Color`; `view(client, config)` uses
`client.hovered`, sets `Subsurface::alpha`, multiplies the border colour,
label background/text, pin and placeholder box colours. Tests: opacity
not hovered = pct/100, hovered = 1.0; `with_opacity` touches alpha only.

## Task 4 — applet menu: flat characters
`src/applet/menu.rs`: `rows(status)` (no `accounts_open`), character rows
first, `No characters running` disabled row when empty, drop
`toggles_accounts`. Update tests per spec §4.
`src/bin/yutani-applet/app.rs`: remove `accounts_open`, `Msg::ToggleAccounts`.
`src/bin/yutani-applet/view.rs`: `menu_row(row)` (no `held`), rows go
straight into the group with the character run wrapped in `accounts_list`.
`src/applet/theme.rs`: remove `HELD_HOVER_FILL`, `hover_fill`, the `held`
parameter of `menu_row_class`, and their tests.

## Task 5 — services band
`src/applet/display.rs`: `Service { name: &'static str, up: bool, note:
&'static str }`, `Display.services: [Service; 2]`, `display(status, rates,
offline: OfflineTunnel { link_up, installed })`. Tests per spec §4.
`src/bin/yutani-applet/app.rs`: `display()` passes the offline pair from
`tunnel::control::read_tunnel_file().is_some_and(|f| f.up)` &&
`iface_present()`, and `installed()`.
`src/bin/yutani-applet/view.rs`: `services_band(&d)` under the header in
every state. `src/applet/theme.rs`: `SERVICES_PAD = pad(4,12,4,12)`,
`SERVICES_ROW_GAP = 6`, `SERVICE_UP = ACCENT_UP`, `SERVICE_DOWN =
DANGER_TEXT`; sizes test extended.
Docs: applet spec §4 gets a pointer to the new spec.

## Task 6 — ship
`cargo test`, `cargo build --release`; Daniel installs both binaries
(`sudo install …`), `pkill -x cosmic-panel` for the applet, `yutani quit`
+ relaunch for the daemon; visual check of the popup and the slider.
