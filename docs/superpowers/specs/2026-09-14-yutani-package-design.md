# Yutani — the Arch / CachyOS package

**Status:** asked by Daniel 2026-09-14 ("make this into a CachyOS app that
you can install with a package and some dependencies"); decisions the same
day: a `PKGBUILD` in the repo (not the AUR — the repo is private), and the
package owns everything system-wide that is not per user.

## What the package owns

`packaging/PKGBUILD` builds from the checkout (`source=git+file://` on the
parent directory, so what is committed is what is packaged; `pkgver` is
`0.1.0.r<commits>.g<sha>`), with `cargo fetch --locked` in `prepare` and
`cargo build --frozen --release` in `build`; `check` runs the test suite.

| Path | From |
|---|---|
| `/usr/bin/yutani`, `/usr/bin/yutani-applet` | release build |
| `/usr/share/icons/hicolor/…` (five files) | `assets/icons`, laid out as `assets::ICONS` |
| `/usr/share/applications/com.yutani.{Applet,Yutani}.desktop` | `packaging/*.desktop`, pinned by test to `desktop_entry("/usr/bin/yutani-applet")` / `launcher_entry("/usr/bin/yutani")` |
| `/usr/lib/systemd/user/yutani.service` | `packaging/yutani.service`, pinned by test to `service::unit_text("/usr/bin/yutani")` |
| licence, README | repo |

Dependencies name the tools the binaries exec (`wg`, `nft`, `ip`,
`systemctl`/`resolvectl`/`busctl`/`systemd-run`, `pkexec`, `curl`), the
desktop pieces (cosmic-comp, cosmic-panel, the COSMIC portal for the file
and folder choosers) and the linked/loaded libraries (mesa for libgbm and
EGL, vulkan-icd-loader for wgpu, libdrm, libxkbcommon, expat, gcc-libs).

## What stays per user, and why

- `systemctl --user enable --now yutani` — enabling a user unit is the
  user's.
- `yutani shortcuts install` — writes the user's COSMIC shortcut file.
- `yutani tunnel install <conf>` — writes `/etc/yutani/tunnel.conf`, the
  system unit and the polkit rule, as today. The unit's `ExecStart` is the
  installing binary (`/usr/bin/yutani` with the package) and the rule names
  the user, so neither can be a static package file; the conf is the user's
  secret. `installed` therefore still means "the unit `tunnel install`
  wrote exists".
- Adding the applet to the panel.

`service::installed()` and `service::install` know the packaged unit: with
it in place `yutani start` goes through systemd and `service install`
writes nothing (a per-user copy would only shadow it).

## Migrating a source install (this machine)

`pacman -U` the package, remove the two `/usr/local/bin` binaries (they
come first on `PATH`), `yutani applet uninstall` and `yutani service
uninstall` to drop the per-user copies that would shadow the packaged
files, re-run `yutani tunnel install <conf>` so the tunnel unit's
`ExecStart` is `/usr/bin/yutani`, then `systemctl --user daemon-reload &&
systemctl --user restart yutani` and `pkill -x cosmic-panel`.
