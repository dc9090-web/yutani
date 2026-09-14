# Plan — redesign part 1: icons

Spec: `docs/superpowers/specs/2026-09-14-yutani-redesign-design.md` §1;
geometry in `…-redesign-handoff.md` "Screen 3". TDD; never `cargo fmt`.

1. **Assets.** Write the five SVGs in `assets/icons/`; delete the five
   `y-*.svg`. `src/assets.rs`: new consts, `ICONS` (5 entries with their
   theme dirs), `LEGACY_ICONS` (dir, name) for the old files. Tests: every
   symbolic file is `viewBox="0 0 32 32"` and uses `currentColor` with no
   fixed fill; the app icons are `0 0 128 128`; dirs are as the spec table.
2. **Icon state.** `src/applet/icon.rs`: `Badge { Connected, Attention,
   Sync }`, `IconState::badge() -> Option<Badge>`, `bytes(icon_px)` picks
   the solid variant under 22 px, `icon_name()` is `yutani-symbolic` for
   every state, `DIM_OPACITY` 0.40. Tests updated.
3. **Theme + view.** `theme::badge_px` = icon/3 clamped 7–10;
   `theme::badge_class(diameter, badge)` colours from the theme (success /
   warning / hollow); `view::panel_button` draws any badge. Remove
   `ACTIVE_BADGE*`. Popup header uses the new symbolic bytes for now.
4. **Install.** `Icon=yutani-symbolic` / `Icon=yutani`; install writes
   `ICONS` and removes `LEGACY_ICONS`; uninstall removes both sets. Tests
   updated (file counts, names, legacy cleanup).
5. **Docs + ship.** Applet spec §3 pointer; `cargo test`, release build;
   Daniel installs and runs `yutani applet install` + `pkill -x
   cosmic-panel`; checkpoint.
