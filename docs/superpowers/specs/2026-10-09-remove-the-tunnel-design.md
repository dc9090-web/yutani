# Removing the tunnel, the ping and the network monitor

2026-10-09. Daniel no longer needs the VPN, so the EVE-only WireGuard tunnel
goes, and with it everything that only existed to show or serve it.

## Decision

Delete it all rather than hide it:

- **Machine first.** `yutani tunnel uninstall` was run on the dev machine
  before the code went (unit, `/etc/yutani/tunnel.conf`, polkit rule).
- **Code.** `src/tunnel/` (worker, rules, install, probe), `src/adopt.rs`,
  the `yutani tunnel …` CLI, the IPC `tunnel connect|disconnect`, the
  settings window's Tunnel page and its drag-and-drop, the config's
  `tunnel` section.
- **Applet.** The WireGuard row, the whole 03 NETWORK section (rates, sand
  scope, session totals, ping, endpoint and peer) and the green tunnel dot.
  The popover is now 01 CONTROL (service only), 02 ACCOUNTS, 03 HOST,
  Launch EVE. With the scope gone nothing can drop on a short screen, so the
  height estimate, `available` and the `outputs` list in `status` went too.
- **Packaging.** `wireguard-tools`, `nftables`, `iproute2` and `polkit`
  dropped from the PKGBUILD and the Ansible role (its `tunnel.yml` deleted).

## What stays

- `yutani launch` keeps setting the game's present mode and frame cap; it
  no longer wraps the game in `yutani-eve.slice`, it `exec`s it directly.
  The Steam launch line is unchanged.
- The `/proc` name match moved from `adopt.rs` to `src/eve_process.rs`: the
  Launch EVE log looks for the EVE Launcher, and the Characters page will
  not copy settings under a running client.
- `status` lives in `src/status.rs`. Old replies with `tunnel`/`direct_ping`
  fields and old `config.ron` files with a `tunnel: (…)` section still
  parse; the extra fields are ignored.
- The IPC `watch` request is still answered `ok` for older applets.
