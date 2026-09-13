# Yutani — EVE-only WireGuard tunnel (design)

**Status:** approved (Daniel, 2026-09-12)
**Depends on:** plan 4 (IPC socket, CLI)
**Companion:** `2026-09-12-yutani-applet-design.md` (the panel applet that shows and controls this)

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

**Known gap (2026-09-12), closed 2026-09-13: the NSS/resolved DNS path.**

**Status: closed on 2026-09-13** — EVE-domain lookups go via resolved's
per-link DNS on `yutani0` to 1.1.1.1/9.9.9.9 inside the tunnel; other
lookups are unchanged; the stub-resolver DNAT exclusion stays. The
description of the gap below is kept because it is why the remedy has the
shape it has.
 `/etc/nsswitch.conf`
has `hosts: … resolve [!UNAVAIL=return] …`, so glibc's `getaddrinfo` does
not send a DNS packet at all: it talks to `systemd-resolved` over a unix
socket. No IP packet leaves the EVE cgroup, so neither the DNAT in the `dns`
chain nor the kill-switch in the `killswitch` chain ever sees it. `resolved`
then queries upstream from *its own* cgroup, unmarked, over the normal
route — outside the tunnel. So while the tunnel is up, EVE's name lookups
still leak to the machine's configured resolver, and while the tunnel is
meant to be up but is broken, they still succeed instead of being blocked.
Programs that speak DNS directly (`dig`, `nslookup`, Wine's own resolver if
it bypasses NSS) do go through the DNAT and are unaffected by this gap —
which is exactly why the acceptance check in §10 must not use `dig`.

DNS sent to the local stub resolver (`127.0.0.53`, `/etc/resolv.conf`'s
target under systemd-resolved) is deliberately *not* redirected by the
`dns` chain: it reaches `systemd-resolved` over loopback and is answered
outside the tunnel, exactly like the NSS path above, because a
loopback-destined query is built with a loopback source address
(`127.0.0.1`), and a loopback-sourced packet cannot be re-routed out of a
non-loopback interface such as `yutani0` — the kernel's route lookup
refuses it (`__mkroute_output` returns `EINVAL`), so DNAT-ing its
destination would only get it dropped. Only DNS aimed at a real resolver
address is redirected to `10.2.0.1`. This matters because the Steam
runtime container the EVE launcher runs in (`pressure-vessel`) ships a
glibc whose NSS stack lacks the `resolve` module, so unlike a normal
CachyOS process it does not take the unix-socket path above — it sends raw
DNS packets straight to the stub resolver, `127.0.0.53`, which is the path
this exclusion is about.

**The remedy (2026-09-13).** The second of the two options below was
taken: `yutani0` gets a per-link DNS in `systemd-resolved`, with the EVE
domains routed to it. While the tunnel is up the worker runs

```
resolvectl dns yutani0 1.1.1.1 9.9.9.9
resolvectl domain yutani0 ~eveonline.com ~ccpgames.com ~evetech.net
resolvectl default-route yutani0 false
```

so resolved sends lookups for those domains — and only those — to
1.1.1.1/9.9.9.9, and one extra rule in `setmark` (§4) marks DNS aimed at
those two addresses for the tunnel, whoever sent it. That last part is the
point: the query that must be tunnelled is *resolved's*, sent from
resolved's own cgroup, which no `socket cgroupv2` rule of ours can match —
so the rule keys on the destination instead, which is safe precisely
because `resolvectl domain` limits what is ever sent there. The masquerade
in `postrouting` rewrites the source as usual. Every other lookup on the
machine still goes to the machine's normal resolver
(`default-route … false`), and the stub-resolver DNAT exclusion below
stays exactly as it is — the Steam runtime's raw queries to `127.0.0.53`
are answered by resolved, which now routes the EVE domains among them into
the tunnel too.

The per-link settings die with the interface, so teardown does not depend
on cleaning them up; `resolvectl revert yutani0` runs first in the down
sequence anyway, and its failure is ignored. A failure of any of the three
`resolvectl` commands (resolved not running, say) is a `warn!` and nothing
more: the tunnel is still up and correct, and DNS merely behaves as it did
before this change.

The other option — running each launched command in a mount namespace with
an `nsswitch.conf` that has no `resolve` entry, so glibc falls back to
`dns` and sends real packets the DNAT can catch — was not taken.

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

Constants: interface `yutani0`; fwmark `0x59` (on the cleartext packets
that must be routed into the tunnel) and `0x5a` (WireGuard's own mark, on
the encrypted packets that must be let out — see "Why the interface fwmark
exists" in §4); routing table `51820`;
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
3. `wg set yutani0 fwmark 0x5a` — WireGuard then stamps `0x5a` on every
   *encrypted* packet it sends. This must come after `setconf`, which
   rewrites the whole device configuration and would clear a mark set
   before it. See "Why the interface fwmark exists" below.
4. `ip address add 10.2.0.2/32 dev yutani0`; `ip link set yutani0 mtu 1420
   up` (MTU from the conf if given).
5. `sysctl -w net.ipv4.conf.yutani0.rp_filter=2`.
6. `ip route add default dev yutani0 table 51820`;
   `ip rule add fwmark 0x59 lookup 51820 priority 1000`;
   `ip rule add from 10.2.0.2 lookup 51820 priority 1001` (makes strict
   `rp_filter` accept tunnel replies).
7. `nft -f -` with:

(`mark` is a reserved nft keyword, hence `setmark`.)

```
table inet yutani {
    chain setmark {
        type route hook output priority mangle; policy accept;
        meta mark 0x5a return
        ip daddr { 1.1.1.1, 9.9.9.9 } meta l4proto { tcp, udp } th dport 53 meta mark set 0x59
        socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" meta mark set 0x59
    }
    chain dns {
        type nat hook output priority dstnat; policy accept;
        socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" ip daddr != 127.0.0.0/8 meta l4proto { tcp, udp } th dport 53 dnat ip to 10.2.0.1
    }
    chain postrouting {
        type nat hook postrouting priority srcnat; policy accept;
        oifname "yutani0" masquerade
    }
    chain killswitch {
        type filter hook postrouting priority filter; policy accept;
        oifname "lo" accept
        meta mark 0x59 oifname != "yutani0" counter drop
    }
}
```

   (The `ip daddr { 1.1.1.1, 9.9.9.9 }` rule is the tunnel's DNS: those
   are the resolvers `resolvectl dns yutani0 …` points `systemd-resolved`
   at in step 8, and this rule is what puts resolved's queries to them into
   the tunnel. It is deliberately not behind the cgroup match — resolved
   sends them from its own cgroup — and deliberately ahead of it, next to
   the other destination-keyed rule. The addresses come from the stored
   conf (`# yutani: dns_servers`, §5); with no servers configured the rule
   is omitted entirely (an empty nft set is a syntax error).

   `level 5` = number of path components; the path is built from the uid.
   Systemd nests `yutani-eve.slice` under `yutani.slice` because of the
   dash, hence five components. `meta nfproto ipv6` from the cgroup falls
   under the last rule since v6 never routes via `yutani0`. The DNS rule
   carries `ip daddr != 127.0.0.0/8` for two reasons: it guards against v6
   packets, since this is an `inet` table — the chain also sees them and
   `dnat ip to` is an IPv4-only statement — the same job `meta nfproto ipv4`
   used to do alone; and, being narrower than that, it also excludes
   loopback destinations, i.e. the local stub resolver `127.0.0.53` — see
   the DNS paragraph in §2's "Known gap".)

   **Why the interface fwmark exists.** The `meta mark 0x5a return` rule
   comes first in `setmark`, ahead of every cgroup match. It is tempting to
   assume the encrypted UDP to the endpoint is emitted by the kernel's wg
   device with no socket attached and so is unaffected by rules that match
   `socket cgroupv2` — that is wrong, and believing it cost a day. The
   kernel re-uses the *inner* packet's `sk_buff` for the encrypted outer
   datagram, so the outer datagram still carries `skb->sk`: the game's
   socket, whose cgroup is `yutani-eve.slice`. Without the exemption,
   `setmark` therefore also marks the encrypted packet `0x59`, `ip rule
   fwmark 0x59 lookup 51820` routes it back into `yutani0`, WireGuard
   encrypts it again, and the loop fills the per-peer staged queue. The
   symptom is total loss of every packet (TCP and ICMP alike) sent from
   inside the slice, with `yutani0`'s TX `dropped` counter climbing,
   `tx_errors` at 0 and nothing in the kernel log — WireGuard only bumps
   `tx_dropped` when that staged queue overflows. From *outside* the slice
   the tunnel looks perfectly healthy (`curl --interface 10.2.0.2` returns
   the London exit), because those packets carry a socket in a different
   cgroup and never match. `wg set yutani0 fwmark 0x5a` makes WireGuard
   write `0x5a` into `skb->mark` on every outer packet, which is exactly
   what `setmark`'s first rule keys on: it returns without re-marking, so
   the outer packet keeps `0x5a` and leaves by the LAN route to the peer's
   endpoint, as it is meant to. That mark also keeps it clear of the
   kill-switch, which drops only `0x59`. This is the same reason wg-quick
   sets a firewall mark on the interfaces it creates.

   **Why the kill-switch is in POSTROUTING.** It hooks postrouting and
   keys on `meta mark 0x59`, not on the cgroup. An `nft monitor trace` of a
   TCP SYN from the slice shows why: `setmark` sets `meta mark 0x59` in the
   route chain, the kernel's route hook then re-routes the packet to
   `yutani0` — and every later chain in the *same* OUTPUT hook (`ip mangle
   OUTPUT`, our `dns`, `ip nat OUTPUT`, and a kill-switch hooked there)
   still reports `oif "enp5s0"`. The hook state's `out` device is captured
   once when the OUTPUT hook starts and is not refreshed after
   `ip_route_me_harder`, so an output-hook `oifname != "yutani0" drop`
   matches on a stale interface and drops exactly the packets it is meant
   to let through (the drop counter climbs while the tunnel carries
   nothing). Only POSTROUTING, whose hook state is built after routing,
   sees the real outgoing interface. `socket cgroupv2` cannot come along:
   `nft_socket` allows that expression in prerouting/input/output hooks
   only. It is not needed either — `setmark` sets `0x59` solely on
   cgroup-matched packets, so "marked for the tunnel but leaving elsewhere"
   is precisely the kill-switch condition, and the encrypted outer packets
   (marked `0x5a`) can never match it. `oifname "lo" accept` comes first so
   loopback traffic inside the slice is untouched. The two postrouting
   chains sit at different priorities (`filter` 0 for the kill-switch,
   `srcnat` 100 for the masquerade), so the drop is evaluated before the
   source rewrite.

   The `postrouting` chain is what makes the slice's traffic usable at all.
   An application chooses its source address when the socket connects —
   *before* `setmark` runs — so the kernel's first route lookup uses the
   LAN route and binds the LAN address (`10.1.1.221`). Marking the packet
   reroutes it out of `yutani0` but does not revisit that choice, so the
   inner packet leaves with a source outside the peer's AllowedIPs
   (`10.2.0.2/32`) and WireGuard cryptokey routing on the far side drops it:
   every connection hangs in `SYN-SENT`. Masquerading on the way out of
   `yutani0` rewrites the source to the interface's own address, and
   conntrack un-NATs the replies.

8. `resolvectl dns yutani0 <dns_servers>`;
   `resolvectl domain yutani0 ~<dns_domain>…`;
   `resolvectl default-route yutani0 false` — the per-link DNS settings
   that close §2's known gap (see the remedy there for why). They run after
   the link exists and after the ruleset is loaded. Servers and domains
   come from the stored conf (`# yutani: dns_servers` /
   `# yutani: dns_domains`, §5), defaulting to 1.1.1.1/9.9.9.9 and
   `eveonline.com ccpgames.com evetech.net`. A failure here is a `warn!`,
   never a teardown: everything else about the tunnel is fine, and only the
   DNS improvement is lost.

**Loop**: every second run `ip route replace default dev yutani0 table
51820` and then write `/run/yutani/tunnel.json` (0644, atomic rename) with

```json
{ "up": true, "iface": "yutani0", "address": "10.2.0.2", "endpoint": "198.51.100.10:51820",
  "latest_handshake_unix": 1789180000, "rx_bytes": 123, "tx_bytes": 456,
  "since_unix": 1789179000 }
```

from `wg show yutani0 dump` (peer line; the private key column is never
written anywhere). Missing handshake → `latest_handshake_unix: 0`. The
`ip route replace` is there because the kernel deletes
`default dev yutani0 table 51820` whenever the link goes down and does not
restore it when the link comes back up; without the tick re-adding it,
table 51820 stays empty after any `ip link set yutani0 down` and marked
packets are dropped by the kill-switch for the rest of the session.
`replace` is idempotent, and its failure while the link is genuinely down
is expected: logged at debug, never a teardown.

**Down** (on SIGTERM, and on any start failure): `resolvectl revert
yutani0` (first, while the link still exists; failure ignored — the
per-link settings die with the interface anyway); `nft delete table inet
yutani`; both `ip rule del`; `ip route flush table 51820`; `ip link del
yutani0`; remove `tunnel.json`. Each step is attempted even if an earlier
one fails.

Behaviour: while the unit is active and the handshake stalls, marked
packets still go to `yutani0` and are lost — that *is* the kill-switch.
`systemctl stop` removes everything and EVE traffic flows normally.

## 5. Privileged install: `yutani tunnel install <conf>`

Runs `pkexec <abs yutani> tunnel install-root --conf <abs conf> --uid <uid>
--user <name> --exe <abs yutani> --dns-servers 1.1.1.1,9.9.9.9
--dns-domains eveonline.com,ccpgames.com,evetech.net` (one password
prompt; the two DNS options carry the user's validated
`tunnel.dns_servers` / `tunnel.dns_domains`, and are omitted when empty).
`install-root` (root):

1. Parses and validates the conf (exactly one `[Interface]` with
   `PrivateKey` and one IPv4 `Address`; one `[Peer]` with `PublicKey`,
   `Endpoint`, `AllowedIPs`); refuses anything else with a clear message.
2. Writes `/etc/yutani/tunnel.conf` (dir 0700, file 0600 root:root) — a
   verbatim copy plus a `# yutani: label = UK#455` line derived from the
   peer comment or the file name, a `# yutani: uid = 1000` line, and the
   two DNS lines the worker reads:
   ```
   # yutani: dns_servers = 1.1.1.1 9.9.9.9
   # yutani: dns_domains = eveonline.com ccpgames.com evetech.net
   ```
   This is how the user's `config.ron` values reach root without root ever
   reading a user-writable file (§9): they are validated on the way in and
   copied into the file root owns. Each domain must be a plain host name
   (letters, digits, `-`, `.`) or `install-root` refuses — a value with a
   newline in it would otherwise forge a second `# yutani: …` line. An
   omitted or empty option, and a conf written before this feature, both
   mean "use the built-in defaults", so an older install keeps working.
   **Changing `tunnel.dns_servers`/`tunnel.dns_domains` in `config.ron`
   therefore takes effect only after re-running `yutani tunnel install`**
   (and a `disconnect`/`connect`, like any other conf change). The directory's mode is set explicitly
   after creating it: `pkexec` does not reset the caller's umask, so
   `create_dir_all` alone would give whatever mode that umask allows. An
   *existing* `/etc/yutani` is verified root-owned and not group- or
   world-writable, and refused otherwise.
3. Writes `/etc/systemd/system/yutani-tunnel.service`:
   ```
   [Unit]
   Description=Yutani EVE tunnel (WireGuard, per-app routing)
   After=network-online.target
   Wants=network-online.target
   [Service]
   Type=simple
   ExecStart=<exe> tunnel run
   Environment=PATH=/usr/sbin:/usr/bin:/sbin:/bin
   RuntimeDirectory=yutani
   RuntimeDirectoryMode=0755
   KillSignal=SIGTERM
   TimeoutStopSec=10
   Restart=no
   [Install]
   WantedBy=multi-user.target
   ```
   (not enabled; started on demand. `RuntimeDirectory=yutani` gives the
   worker `/run/yutani` for `tunnel.json` and the transient `wg.conf`, and
   systemd removes it on stop. `Environment=PATH=…` is explicit because
   systemd's default PATH for system units does not include `/usr/sbin`,
   where `ip`, `wg`, `nft` and `sysctl` live on some distributions.)
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

**The `<exe>` must be root-owned and not writable by group or others**, and
so must every directory above it — `install-root` canonicalises the path and
refuses otherwise, before writing anything, and the user side checks the
same thing *before* the `pkexec` prompt so no password is wasted. The unit
runs that binary as root and the polkit rule below lets the user start it
without a password, so a binary the user can rewrite (`target/debug/yutani`,
owned by the developer) would be passwordless root for anything running as
that user.

Dev workflow, therefore:

```bash
cargo build --release
sudo install -o root -g root -m 0755 target/release/yutani /usr/local/bin/yutani
/usr/local/bin/yutani tunnel install ~/Downloads/EVE-UK-455.conf
```

Re-run both steps after rebuilding. `install` is idempotent. `--dry-run`
needs no root and reports the trust check (and the uid↔user check) alongside
everything it would write, so the refusal is visible before the prompt.

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
- Config additions: `tunnel: ( location: "London", auto_adopt: true,
  dns_servers: ["1.1.1.1", "9.9.9.9"], dns_domains: ["eveonline.com",
  "ccpgames.com", "evetech.net"] )`. The two DNS keys are the resolvers
  EVE's lookups are sent to inside the tunnel and the domains routed to
  them (§2's remedy, §4 step 8). `validate` trims each domain, strips a
  leading `~` or `.` (resolved's own spelling, and the `~` is ours to add),
  drops anything that is not a plain host name with a warning, and falls
  back to the defaults if either list ends up empty. They are read by the
  *user* side only: `yutani tunnel install` copies them into
  `/etc/yutani/tunnel.conf`, so editing them in `config.ron` requires
  re-running `yutani tunnel install` to have any effect.

### Settings page

The settings window's **Tunnel** page is the same thing without the
terminal: it takes the WireGuard `.conf` (Browse… through the XDG
file-chooser portal, a typed path, or the file dropped on the window) and
calls exactly the functions above — `tunnel::install::install` /
`uninstall` and `tunnel::control::connect` / `disconnect` — on the blocking
pool, so the thumbnails keep drawing while they run. `install` still goes
through `pkexec` from there, so the desktop's polkit agent asks for the
password once, as it does for the CLI; nothing about §5's privilege split
changes. The page shows the file *name* and the parsed `location` only —
never the key material. Specified in full in
`2026-09-13-yutani-tunnel-page-and-ansible-design.md` §1.

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
- **Invariant: nothing the user can write is executed or read by root.**
  The unit's `ExecStart` binary and every directory above it must be
  root-owned and not group/world-writable (§5), and `/etc/yutani` is held to
  the same standard. The one deliberate crossing is the conf the user hands
  to `install`: it is read once, at install time, parsed and validated, and
  copied under root's control — never executed, and never read again by the
  running worker, which reads only `/etc/yutani/tunnel.conf`.
- `install-root` also resolves `--uid` to a user name itself (`id -un`) and
  refuses a `--user` that disagrees: the polkit rule names a user while the
  nft rules key on a uid, and those must be the same person.
- Kill-switch blind spot: the `killswitch` chain drops what `setmark`
  marked, and `setmark` matches `socket cgroupv2`, which needs a socket to
  attribute the packet to. A few packets the kernel emits without one — a
  late RST, retransmissions from a TIME_WAIT socket after the process is
  gone — are therefore never marked, and can leave by the normal route. They carry no payload and reveal only that
  an already-known connection existed; accepted residual.
- DNS: while the tunnel is up, `1.1.1.1` and `9.9.9.9` are marked for the
  tunnel by destination, not by cgroup (§2's remedy) — so any program on
  the machine that queries those two addresses *directly* also goes through
  the tunnel for that query, not just EVE's lookups. Nothing else changes
  for other programs: resolved's own default route is left alone
  (`default-route yutani0 false`) and only the EVE domains are routed to
  the link. Accepted residual; the alternative (matching resolved's cgroup)
  would tunnel every lookup the machine makes.
- The `dns_servers`/`dns_domains` values originate in the user's
  `config.ron`, which root never reads: they cross into root's world once,
  as `install-root` arguments, are re-validated there (plain host names
  only), and are stored in the root-owned conf — the same crossing the
  wg-quick conf itself makes, and the same rules apply.
- DNS, historical: see the known gap in §2 — with `resolve` in `nsswitch.conf`, EVE's
  `getaddrinfo` lookups never become IP packets from its cgroup, so neither
  the DNAT nor the kill-switch applies to them.

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
slice must print the home IP. **DNS must not be checked with `dig`**: `dig`
speaks DNS directly and so goes through the DNAT, while the game's
`getaddrinfo` goes to `systemd-resolved` over a unix socket and does not
(§2). Check the path EVE actually uses instead —
`systemd-run --user --scope --quiet --slice=yutani-eve.slice getent hosts
whoami.akamai.net` — together with `resolvectl query whoami.akamai.net`, and
observe what resolved actually sent upstream with `sudo resolvectl monitor`
(or `journalctl -u systemd-resolved -f`) in another terminal. Since
2026-09-13 a lookup for an EVE domain must go to 1.1.1.1/9.9.9.9 through
`yutani0` (`resolvectl status yutani0` shows the per-link DNS and the
`~domain` routing; the `setmark` counter for the resolver rule moves), and
a lookup for anything else must still leave by the host's normal route —
that second half is by design, not a regression. Also inspect the live ruleset and policy routing with
`sudo nft list table inet yutani` and `ip rule show`, and confirm the
interface fwmark is in place with `sudo wg show yutani0 fwmark` (`0x5a`).
If the slice has no connectivity at all while the tunnel is healthy from
outside it, check `cat /sys/class/net/yutani0/statistics/tx_dropped`: a
counter climbing with `tx_errors` at 0 is the encrypt loop described in
§4, i.e. the fwmark or one of its two nft rules has gone missing. Then
`yutani tunnel disconnect`
→ the slice `curl` prints the home IP again; with the unit up and the
endpoint blocked (e.g. wrong endpoint in a scratch conf) the slice `curl`
times out. The same holds for `sudo ip link set yutani0 down`: the slice
`curl` times out while plain `curl` keeps working. Bringing the link back
up with `sudo ip link set yutani0 up` is enough — the kernel drops
`default dev yutani0 table 51820` when the link goes down and does not
restore it, but the worker's 1 s tick re-adds it, so the slice recovers
within a second. `yutani tunnel disconnect && yutani tunnel connect` is the
clean reset if anything else was disturbed by hand. Then EVE: launch through the wrapper, log in, check the applet
counters move.

## 11. Out of scope for this spec

IPv6 via the tunnel; enabling the unit at boot; multiple exits/switching
location (needs several confs + a selector); per-character routing.

*Status after plan A (2026-09-13): implemented and accepted on the target
machine.* Three defects surfaced only under live traffic and are fixed on
`master` (`c8c0843`, `c9743cc`, `fa834b1`): slice sockets bind the LAN
source before the mark (masquerade on `yutani0`); WireGuard's outer packet
inherits the inner socket's cgroup and was re-marked into a loop
(interface fwmark `0x5a`); and chains later in the OUTPUT hook see the
pre-reroute `oif` (kill-switch moved to POSTROUTING, keyed on the mark).
Observed: home exit `203.0.113.7`, slice exit `203.0.113.42` (UK#455);
with `yutani0` down the slice times out (exit 28) while home traffic
works, and the route is back within 2 s of `ip link set yutani0 up`;
`getent hosts whoami.akamai.net` from the slice answers with the *home*
address, confirming the §2 DNS gap (queries go out via systemd-resolved,
here Tailscale MagicDNS `100.100.100.100`); the kill-switch counter shows
exactly the packets dropped during the link-down test.
