# Yutani — EVE-only WireGuard tunnel (design)

**Status:** approved (Daniel, 2026-09-12)
**Depends on:** plan 4 (IPC socket, CLI)
**Companion:** `2026-09-12-yutani-applet-design.md` (the tray applet that shows and controls this)

## 1. Goal

All network traffic of the EVE Online client and its launcher — and only
that traffic — leaves the machine through a WireGuard tunnel to a London
exit (Proton VPN, `UK#455`). Everything else on the machine (browser, Steam
downloads, tailscale) is untouched. While the tunnel is meant to be up but
isn't working, EVE gets no network at all (kill-switch); a deliberate
disconnect returns EVE to the normal connection.

Non-goals: a general VPN client; IPv6 through the tunnel (the provided
config has no v6 address — EVE's v6 is dropped); multi-user machines;
distributions without systemd, nftables and iproute2.

## 2. Facts about the target machine (verified 2026-09-12)

- CachyOS, systemd, cgroup v2, `wg`/`wg-quick`/`nft`/`ip`/`systemd-run`/
  `pkexec`/`busctl` present; polkit 127; user is in `wheel`.
- System resolver is systemd-resolved's stub `127.0.0.53` — process DNS
  goes to loopback, so a per-process route alone would leak DNS.
- `net.ipv4.conf.all.rp_filter = 1` (strict).
- EVE started from the COSMIC app list runs in
  `user.slice/user-1000.slice/user@1000.service/app.slice/app-cosmic-…scope`;
  the user manager's subtree is delegated, so the user can create scopes and
  move their own processes without root.
- The WireGuard config is wg-quick format: `[Interface]` with `PrivateKey`,
  `Address = 10.2.0.2/32`, `DNS = 10.2.0.1`; `[Peer]` with `PublicKey`,
  `AllowedIPs = 0.0.0.0/0, ::/0`, `Endpoint = 198.51.100.10:51820`,
  `PersistentKeepalive = 25`. Comments carry the server label (`# UK#455`).

## 3. Architecture

```
 user session                                  system (root)
 ┌──────────────────────────────┐              ┌──────────────────────────────┐
 │ yutani (daemon)              │  systemctl   │ yutani-tunnel.service        │
 │  • IPC: tunnel connect/      │ start/stop ─▶│  ExecStart=yutani tunnel run │
 │    disconnect/status         │  (polkit     │  • yutani0 wg link + routes  │
 │  • auto-adopt exefile.exe    │   rule)      │  • nft table inet yutani     │
 │    into yutani-eve.slice     │              │  • writes /run/yutani/       │
 │  • reads /run/yutani/        │◀─ file ──────│      tunnel.json every 1 s   │
 │    tunnel.json + sysfs       │              └──────────────────────────────┘
 ├──────────────────────────────┤
 │ yutani launch -- %command%   │  systemd-run --user --scope --slice=yutani-eve.slice
 └──────────────────────────────┘
```

Three pieces, all in the `yutani` binary:

1. **`yutani tunnel run`** — the root worker (ExecStart of the unit). Sets
   the tunnel up, loops publishing status, tears down on SIGTERM.
2. **`yutani tunnel install|uninstall`** — one-time privileged setup via
   `pkexec`; also `connect|disconnect|status` for the user.
3. **`yutani launch`** and **auto-adopt** — put EVE's processes into the
   `yutani-eve.slice` cgroup that the firewall rules key on.

Constants: interface `yutani0`; fwmark `0x59`; routing table `51820`;
slice `yutani-eve.slice` (cgroup path
`user.slice/user-<uid>.slice/user@<uid>.service/yutani.slice/yutani-eve.slice`,
nft `level 5`; systemd nests `yutani-eve.slice` under `yutani.slice` because
of the dash, hence five components); config `/etc/yutani/tunnel.conf`;
status `/run/yutani/tunnel.json`.

## 4. Root worker: `yutani tunnel run`

Runs as root under `yutani-tunnel.service` (`Type=notify` is not needed;
`Type=simple`, `KillSignal=SIGTERM`, `TimeoutStopSec=10`). No shell scripts:
it execs `ip`, `wg`, `nft` with generated arguments; every command's
failure aborts the start with the command and stderr in the journal, after
running the teardown for whatever was already created.

**Up**, in order:

1. `ip link add yutani0 type wireguard`
2. `wg setconf yutani0 <tmpfile>` — the wg-native subset of
   `/etc/yutani/tunnel.conf` (`[Interface] PrivateKey`, `[Peer] PublicKey,
   PresharedKey?, AllowedIPs, Endpoint, PersistentKeepalive`); the tmpfile is
   0600 under `/run/yutani/` and deleted right after. `Address`/`DNS`/`MTU`
   are consumed by us.
3. `ip address add 10.2.0.2/32 dev yutani0`; `ip link set yutani0 mtu 1420
   up` (MTU from the conf if given).
4. `sysctl -w net.ipv4.conf.yutani0.rp_filter=2`.
5. `ip route add default dev yutani0 table 51820`;
   `ip rule add fwmark 0x59 lookup 51820 priority 1000`;
   `ip rule add from 10.2.0.2 lookup 51820 priority 1001` (makes strict
   `rp_filter` accept tunnel replies).
6. `nft -f -` with:

(`mark` is a reserved nft keyword, hence `setmark`.)

```
table inet yutani {
    chain setmark {
        type route hook output priority mangle; policy accept;
        socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" meta mark set 0x59
    }
    chain dns {
        type nat hook output priority dstnat; policy accept;
        socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" meta l4proto { tcp, udp } th dport 53 dnat ip to 10.2.0.1
    }
    chain killswitch {
        type filter hook output priority filter; policy accept;
        socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" oifname "lo" accept
        socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" oifname != "yutani0" counter drop
    }
}
```

   (`level 5` = number of path components; the path is built from the uid.
   Systemd nests `yutani-eve.slice` under `yutani.slice` because of the
   dash, hence five components. `meta nfproto ipv6` from the cgroup falls
   under the last rule since v6 never routes via `yutani0`.) The encrypted
   UDP to the endpoint is emitted
   by the kernel's wg device, not from a cgroup socket, so it is unaffected.

**Loop**: every second write `/run/yutani/tunnel.json` (0644, atomic
rename) with

```json
{ "up": true, "iface": "yutani0", "address": "10.2.0.2", "endpoint": "198.51.100.10:51820",
  "latest_handshake_unix": 1789180000, "rx_bytes": 123, "tx_bytes": 456,
  "since_unix": 1789179000 }
```

from `wg show yutani0 dump` (peer line; the private key column is never
written anywhere). Missing handshake → `latest_handshake_unix: 0`.

**Down** (on SIGTERM, and on any start failure): `nft delete table inet
yutani`; both `ip rule del`; `ip route flush table 51820`; `ip link del
yutani0`; remove `tunnel.json`. Each step is attempted even if an earlier
one fails.

Behaviour: while the unit is active and the handshake stalls, marked
packets still go to `yutani0` and are lost — that *is* the kill-switch.
`systemctl stop` removes everything and EVE traffic flows normally.

## 5. Privileged install: `yutani tunnel install <conf>`

Runs `pkexec <abs yutani> tunnel install-root --conf <abs conf> --uid <uid>
--exe <abs yutani>` (one password prompt). `install-root` (root):

1. Parses and validates the conf (exactly one `[Interface]` with
   `PrivateKey` and one IPv4 `Address`; one `[Peer]` with `PublicKey`,
   `Endpoint`, `AllowedIPs`); refuses anything else with a clear message.
2. Writes `/etc/yutani/tunnel.conf` (dir 0755, file 0600 root:root) — a
   verbatim copy plus a `# yutani: label = UK#455` line derived from the
   peer comment or the file name.
3. Writes `/etc/systemd/system/yutani-tunnel.service`:
   ```
   [Unit]
   Description=Yutani EVE tunnel (WireGuard, per-app routing)
   After=network-online.target
   Wants=network-online.target
   [Service]
   Type=simple
   ExecStart=<exe> tunnel run
   KillSignal=SIGTERM
   TimeoutStopSec=10
   Restart=no
   [Install]
   WantedBy=multi-user.target
   ```
   (not enabled; started on demand.)
4. Writes `/etc/polkit-1/rules.d/50-yutani-tunnel.rules`:
   ```js
   polkit.addRule(function(action, subject) {
       if (action.id == "org.freedesktop.systemd1.manage-units" &&
           action.lookup("unit") == "yutani-tunnel.service" &&
           (action.lookup("verb") == "start" || action.lookup("verb") == "stop" ||
            action.lookup("verb") == "restart") &&
           subject.user == "<username>") {
           return polkit.Result.YES;
       }
   });
   ```
5. `systemctl daemon-reload`; prints what it wrote and reminds the user to
   delete the original conf (world-readable private key).

`yutani tunnel uninstall` → `pkexec … tunnel uninstall-root`: stops the
unit if active, removes the three files, `daemon-reload`.

The `<exe>` path is whatever binary ran `install` (dev: `target/debug/
yutani`); re-run `install` after moving the binary. `install` is
idempotent.

## 6. User side

- `yutani tunnel connect` / `disconnect` → `systemctl start|stop
  yutani-tunnel.service` (no `sudo`; authorised by the polkit rule).
  Exit 0/1 with systemctl's message on failure. Also IPC requests
  `tunnel connect` / `tunnel disconnect` handled by the daemon the same way.
- `yutani tunnel status` (CLI) prints the same data the `status` IPC
  request returns.
- **IPC `status`** (new request; reply `ok <json>` on one line):
  ```json
  { "clients": [ { "name": "KestrelVance", "active": true } ],
    "hidden": false,
    "tunnel": { "installed": true, "connected": true, "iface": "yutani0",
                "location": "London", "address": "10.2.0.2",
                "endpoint": "198.51.100.10:51820",
                "handshake_age_s": 21, "rx_bytes": 413100000, "tx_bytes": 2790000000 } }
  ```
  `installed` = unit file exists; `connected` = `tunnel.json` exists with
  `up: true` and `/sys/class/net/yutani0` present; `handshake_age_s` null
  when no handshake yet; bytes from `/sys/class/net/yutani0/statistics/`
  (world-readable, no root needed) so rates keep working even if the status
  file is stale. `location` from config `tunnel.location` (default
  `"London"`), `iface`/`address`/`endpoint` from `tunnel.json`.
- Config additions: `tunnel: ( location: "London", auto_adopt: true )`.

## 7. Tagging EVE processes

**Launch wrapper.** Steam launch options:
`PROTON_ENABLE_WAYLAND=1 yutani launch -- %command%`. `yutani launch` execs
`systemd-run --user --scope --quiet --slice=yutani-eve.slice
--unit=yutani-eve-<pid> --collect -- <command…>` (environment inherited;
the launcher, Proton and the client are all descendants). If `systemd-run`
fails, it runs the command directly and prints a warning — the game must
never fail to start because of Yutani.

**Auto-adopt** (config `tunnel.auto_adopt`, default true). Every 2 s the
daemon scans `/proc/*/cmdline` for processes owned by the current uid whose
executable name is `exefile.exe` or `EVELauncher.exe`/`eve-online` and whose
`/proc/<pid>/cgroup` path is not under `yutani-eve.slice`; for each it
calls `org.freedesktop.systemd1.Manager.StartTransientUnit` on the user bus
(`yutani-eve-adopt-<pid>.scope`, properties `PIDs=[pid]`,
`Slice=yutani-eve.slice`, `CollectMode=inactive-or-failed`) via `busctl
--user call` (no new dependency). Success/failure is logged once per pid.
Adopted processes' existing TCP connections break once when the tunnel is
up (their source address changes); the wrapper path never has this
problem, and the applet's Preferences copy says so.

Adoption runs regardless of tunnel state, so that connecting later routes
the already-running client without a second adoption.

## 8. Errors and edge cases

- Unit fails to start (bad key, endpoint unresolvable, `nft` missing):
  `journalctl -u yutani-tunnel` has the failing command; `status` shows
  `connected: false`; the applet shows Attention. Teardown leaves no
  half-state.
- Tunnel up, no handshake (server down): traffic dropped by design; the
  applet shows Attention after 180 s without a handshake (spec B).
- Daemon not running: `yutani tunnel connect` still works (it is just
  `systemctl`); the applet shows the daemon-offline state.
- Reboot: the unit is not enabled; EVE is direct until `connect` (the
  applet makes that one click). Enabling at boot is a later option.
- Other WireGuard tunnels (tailscale is WireGuard-based but uses its own
  table/rules) are unaffected: we use our own interface, table and marks.
- `install` when the conf changed: overwrites; a running unit must be
  restarted (`disconnect`/`connect`) — `install` says so.

## 9. Security notes

- Private key lives only in `/etc/yutani/tunnel.conf` (0600 root) and the
  transient 0600 file handed to `wg setconf`. Never in logs, `tunnel.json`,
  IPC or the applet.
- The polkit rule authorises exactly one unit's start/stop/restart for one
  username; nothing else gains privilege. The root worker executes only
  `ip`, `wg`, `nft`, `sysctl` with arguments it generated.
- `install-root` refuses relative paths and confs it cannot fully parse;
  it never executes anything from the conf.

## 10. Testing

Pure, unit-tested: wg-quick conf parser and wg-native stripping; nft
ruleset text for a given uid/iface/mark/table/dns; `ip` command lists for
up/down; unit and polkit file text; `tunnel.json` (de)serialisation and the
`status` JSON assembly; `/proc` scan classification (given cmdline + cgroup
strings); `systemd-run` argv for `launch`; `busctl` argv for adoption.

Integration (Daniel, once): `yutani tunnel install ~/Downloads/EVE-UK-455.conf`
→ `yutani tunnel connect` → `curl --interface` is not enough (curl isn't in
the cgroup) — instead `systemd-run --user --scope --slice=yutani-eve.slice
curl -s https://ifconfig.me` must print the London IP and `curl` outside the
slice must print the home IP; `… --slice=yutani-eve.slice dig +short
whoami.akamai.net` must resolve via `10.2.0.1`; `yutani tunnel disconnect`
→ the slice `curl` prints the home IP again; with the unit up and the
endpoint blocked (e.g. wrong endpoint in a scratch conf) the slice `curl`
times out. Then EVE: launch through the wrapper, log in, check the applet
counters move.

## 11. Out of scope for this spec

IPv6 via the tunnel; enabling the unit at boot; multiple exits/switching
location (needs several confs + a selector); per-character routing.
