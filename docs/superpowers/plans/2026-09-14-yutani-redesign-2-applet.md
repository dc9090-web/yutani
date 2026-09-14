# Plan — redesign part 2: applet popover

Spec: `docs/superpowers/specs/2026-09-14-yutani-redesign-design.md` §2;
layout, copy and sizes in `…-redesign-handoff.md` "Screen 1". TDD; never
`cargo fmt`. Built 2026-09-14 in one pass, checkpointed by install.

1. **Daemon:** `Status.shortcuts: Option<ShortcutHint>` (`prefix`, `next`,
   `prev` as printed) from `config.shortcuts`; `Request::SettingsPage(name)`
   (`settings layouts`) → `open_settings_at(Page)`; `Page::from_name`.
2. **Applet model:** `display::Popover` (header state, account rows, count,
   hotkey hint, thumbs button, tunnel line, throughput, tiles, primary
   action); `history::History` (34 samples, per-direction scaling);
   `format::{rate_parts, uptime}`; `menu::rows(running)` = Layouts &
   characters… / Preferences… / Quit; `Action::LayoutsAndCharacters`.
3. **Theme:** `applet::theme` rewritten — the handoff's geometry as consts,
   colours as functions of the COSMIC theme (ink 100/70/55 %, success,
   accent, destructive, card surface, control/hover fills), the Yutani
   violet as the one fixed token; container and button classes per element.
4. **View:** `view::popup` = header (Y plate, title, state word, toggler) →
   accounts card → tunnel section → throughput card (34-column graph as
   fixed-height containers) → fact tiles → action row (primary + `⋯`) →
   note → menu (shown by `⋯`). Popup width 340 by overriding libcosmic's
   360 px limits.
5. **App:** `history` pushed per poll (zeros while down/failed, cleared
   offline), `ToggleService` → Start/Quit, `ToggleMenu`, menu closes with
   the popup.
6. **Ship:** tests, release build, install both binaries (pkexec), restart
   the panel and the daemon (ASan rebuild for the hint field); checkpoint.
