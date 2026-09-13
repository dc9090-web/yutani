//! Pure generators for everything the root worker executes: the nftables
//! ruleset and the `ip`/`wg`/`sysctl` argv lists. Keeping these pure means
//! the exact commands are unit-tested and printable by `--dry-run`.

use std::net::Ipv4Addr;

use super::conf::WgConf;
use super::{FWMARK, IFACE, SLICE, TABLE, WG_FWMARK};

/// systemd nests `yutani-eve.slice` under `yutani.slice` because of the
/// dash in the name (a dash-separated slice name is automatically a child
/// of the slice named by the part before the last dash), so the real
/// cgroup path has five components, not four.
pub fn cgroup_path(uid: u32) -> String {
    format!("user.slice/user-{uid}.slice/user@{uid}.service/yutani.slice/{SLICE}")
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// `socket cgroupv2 level 5 "<path>"` — level = number of path components.
fn cgroup_match(uid: u32) -> String {
    format!(r#"socket cgroupv2 level 5 "{}""#, cgroup_path(uid))
}

/// `dns_servers` are the resolvers `systemd-resolved` is pointed at for
/// EVE's domains (see [`resolved_up_commands`]): their DNS traffic is
/// marked for the tunnel by `setmark`, whoever sends it.
pub fn nft_ruleset(uid: u32, dns: Option<Ipv4Addr>, dns_servers: &[Ipv4Addr]) -> String {
    let m = cgroup_match(uid);
    let mut s = String::from("table inet yutani {\n");
    s.push_str(
        "    chain setmark {\n        type route hook output priority mangle; policy accept;\n",
    );
    // First, before any cgroup match: WireGuard's encrypted outer packets
    // re-use the inner packet's `sk_buff` and so still carry the game's
    // socket. Re-marking one would route it back into `yutani0` to be
    // encrypted again — a loop that overflows the staged queue. WireGuard
    // stamps `WG_FWMARK` on them; `return` leaves that mark alone.
    s.push_str(&format!("        meta mark {WG_FWMARK:#x} return\n"));
    // Then, still ahead of the cgroup match: the queries `systemd-resolved`
    // sends to the tunnel's own resolvers. EVE's `getaddrinfo` never puts a
    // DNS packet on the wire itself (glibc's `resolve` NSS module talks to
    // resolved over a unix socket — the design doc's §2), so the packet that
    // must go through the tunnel is resolved's, and resolved runs in its own
    // cgroup: no `socket cgroupv2` rule of ours can match it. Keying on the
    // destination instead is what `resolvectl dns yutani0 …` makes safe —
    // only the EVE domains are routed to these addresses. The masquerade in
    // `postrouting` fixes up the source, exactly as for the game's traffic.
    if !dns_servers.is_empty() {
        let set = dns_servers.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ");
        s.push_str(&format!(
            "        ip daddr {{ {set} }} meta l4proto {{ tcp, udp }} th dport 53 meta mark set {FWMARK:#x}\n"
        ));
    }
    s.push_str(&format!("        {m} meta mark set {FWMARK:#x}\n    }}\n"));
    if let Some(dns) = dns {
        s.push_str(
            "    chain dns {\n        type nat hook output priority dstnat; policy accept;\n",
        );
        // `ip daddr != 127.0.0.0/8` first: this both guards against v6
        // packets (this is an `inet` table, so the chain also sees them, and
        // `dnat ip to` is an IPv4-only statement that `meta nfproto ipv4`
        // used to spell out) and excludes the loopback stub resolver
        // (127.0.0.53, `/etc/resolv.conf`'s target under systemd-resolved).
        // A packet aimed at 127.0.0.53 is built with a loopback source
        // (127.0.0.1); DNAT-ing its destination to `dns` would still leave
        // that source, and the kernel's route lookup for a loopback-sourced
        // packet refuses to send it out any non-loopback interface
        // (`__mkroute_output` returns EINVAL), so it would be silently
        // dropped instead of reaching the tunnel. Excluding it here lets
        // that query fall through unmodified to systemd-resolved over
        // loopback — where, since `resolved_up_commands` gives `yutani0` a
        // per-link DNS, the EVE domains among those queries are forwarded
        // to the tunnel's resolvers and do go through the tunnel after
        // all; see §2 of the design doc.
        s.push_str(&format!(
            "        {m} ip daddr != 127.0.0.0/8 meta l4proto {{ tcp, udp }} th dport 53 dnat ip to {dns}\n    }}\n"
        ));
    }
    // The application picks its source address when it connects, *before*
    // the `setmark` chain runs, so a socket in the slice binds the LAN
    // address (10.1.1.221); marking the packet reroutes it to `yutani0` but
    // leaves that source in place, and the peer's cryptokey routing drops
    // an inner source outside our AllowedIPs. Masquerading on the way out
    // rewrites it to the interface's own address (10.2.0.2); conntrack
    // un-NATs the replies.
    s.push_str(
        "    chain postrouting {\n        type nat hook postrouting priority srcnat; policy accept;\n",
    );
    s.push_str(&format!("        oifname \"{IFACE}\" masquerade\n    }}\n"));
    // The kill-switch hooks POSTROUTING, not OUTPUT. The OUTPUT hook state
    // captures its `out` device once, when the hook starts, and nothing
    // refreshes it after the route chain re-routes a freshly marked packet
    // to `yutani0`: every later chain in the same hook — including this one
    // — still reports `oifname "enp5s0"`, so an output-hook drop rule fires
    // on packets that are in fact leaving through the tunnel. POSTROUTING's
    // hook state is built after routing, so it sees the real interface.
    //
    // It keys on `FWMARK` because `socket cgroupv2` is not permitted in a
    // postrouting hook (nft_socket validates prerouting/input/output only),
    // and it does not need to: `setmark` sets that mark only on
    // cgroup-matched packets, so "marked for the tunnel but leaving
    // elsewhere" is exactly the leak we must stop. `WG_FWMARK` needs no
    // exemption either — the encrypted outer packets carry `0x5a`, never
    // `0x59`, so they cannot match the drop.
    s.push_str(
        "    chain killswitch {\n        type filter hook postrouting priority filter; policy accept;\n",
    );
    s.push_str("        oifname \"lo\" accept\n");
    s.push_str(&format!(
        "        meta mark {FWMARK:#x} oifname != \"{IFACE}\" counter drop\n    }}\n"
    ));
    s.push_str("}\n");
    s
}

/// Point `systemd-resolved` at the tunnel's resolvers for EVE's domains,
/// as per-link settings on `yutani0`:
///
/// - `resolvectl dns yutani0 <servers>` — the resolvers to use for this link;
/// - `resolvectl domain yutani0 ~<domain>…` — a *routing-only* domain (the
///   `~`), so resolved sends lookups for those names (and only those) to
///   this link's servers; it is not a search domain, so nothing is appended
///   to unqualified names;
/// - `resolvectl default-route yutani0 false` — and everything else keeps
///   going to the machine's normal resolver.
///
/// This is what closes the NSS gap in the design doc's §2: the lookup EVE
/// makes through `getaddrinfo` never leaves its cgroup as a packet, so the
/// only way to get it into the tunnel is to have resolved itself send it to
/// an address that `nft_ruleset` marks for the tunnel.
pub fn resolved_up_commands(servers: &[Ipv4Addr], domains: &[String]) -> Vec<Vec<String>> {
    let mut dns = argv(&["resolvectl", "dns", IFACE]);
    dns.extend(servers.iter().map(|s| s.to_string()));
    let mut domain = argv(&["resolvectl", "domain", IFACE]);
    domain.extend(domains.iter().map(|d| format!("~{d}")));
    vec![dns, domain, argv(&["resolvectl", "default-route", IFACE, "false"])]
}

/// Drop those per-link settings again. The kernel already forgets them when
/// the interface goes away, so this is belt and braces for the window
/// before `ip link del` — and for a resolved that outlives a link name it
/// has cached. Its failure (resolved not running at all) is ignored.
pub fn resolved_down_command() -> Vec<String> {
    argv(&["resolvectl", "revert", IFACE])
}

/// Re-assert the tunnel's default route. `ip link set yutani0 down` makes
/// the kernel delete `default dev yutani0 table 51820`, and bringing the
/// link back up does *not* restore it — without this the table stays empty,
/// marked packets fall through to the LAN route and the kill-switch drops
/// them forever. `replace` is idempotent, so the worker can run it on every
/// status tick; it simply fails while the link is down.
pub fn ensure_route_command() -> Vec<String> {
    argv(&[
        "ip",
        "route",
        "replace",
        "default",
        "dev",
        IFACE,
        "table",
        &TABLE.to_string(),
    ])
}

/// Everything before loading the nft ruleset, in order. `wg_conf_path` is
/// the transient 0600 file holding `WgConf::wg_native()`.
pub fn up_commands(conf: &WgConf, wg_conf_path: &str) -> Vec<Vec<String>> {
    let mtu = conf.mtu.unwrap_or(1420).to_string();
    let addr = format!("{}/{}", conf.address, conf.prefix_len);
    let from = conf.address.to_string();
    let table = TABLE.to_string();
    let mark = format!("{FWMARK:#x}");
    let wg_mark = format!("{WG_FWMARK:#x}");
    vec![
        argv(&["ip", "link", "add", IFACE, "type", "wireguard"]),
        argv(&["wg", "setconf", IFACE, wg_conf_path]),
        // After `setconf`, which rewrites the whole device configuration and
        // would clear a mark set before it. See `WG_FWMARK`.
        argv(&["wg", "set", IFACE, "fwmark", &wg_mark]),
        argv(&["ip", "address", "add", &addr, "dev", IFACE]),
        argv(&["ip", "link", "set", IFACE, "mtu", &mtu, "up"]),
        argv(&[
            "sysctl",
            "-q",
            "-w",
            &format!("net.ipv4.conf.{IFACE}.rp_filter=2"),
        ]),
        argv(&[
            "ip", "route", "add", "default", "dev", IFACE, "table", &table,
        ]),
        argv(&[
            "ip", "rule", "add", "fwmark", &mark, "lookup", &table, "priority", "1000",
        ]),
        argv(&[
            "ip", "rule", "add", "from", &from, "lookup", &table, "priority", "1001",
        ]),
    ]
}

/// Teardown. Every command is attempted regardless of earlier failures.
pub fn down_commands() -> Vec<Vec<String>> {
    let table = TABLE.to_string();
    let mark = format!("{FWMARK:#x}");
    vec![
        // First, while `yutani0` still exists for resolved to be asked about.
        resolved_down_command(),
        argv(&["nft", "delete", "table", "inet", "yutani"]),
        argv(&[
            "ip", "rule", "del", "fwmark", &mark, "lookup", &table, "priority", "1000",
        ]),
        argv(&["ip", "rule", "del", "lookup", &table, "priority", "1001"]),
        argv(&["ip", "route", "flush", "table", &table]),
        argv(&["ip", "link", "del", IFACE]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::conf::WgConf;

    fn conf() -> WgConf {
        WgConf::parse(
            "[Interface]\nPrivateKey = k=\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n",
            "x",
        )
        .unwrap()
    }

    #[test]
    fn cgroup_path_uses_the_uid_and_slice() {
        assert_eq!(
            cgroup_path(1000),
            "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice"
        );
    }

    fn servers() -> Vec<Ipv4Addr> {
        crate::tunnel::DEFAULT_DNS_SERVERS.to_vec()
    }

    fn domains() -> Vec<String> {
        crate::tunnel::DEFAULT_DNS_DOMAINS.iter().map(|d| (*d).to_string()).collect()
    }

    #[test]
    fn nft_ruleset_marks_dnats_dns_and_kill_switches_exact_text_with_dns() {
        let r = nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()), &servers());
        assert_eq!(
            r,
            "table inet yutani {\n\
             \x20   chain setmark {\n\
             \x20       type route hook output priority mangle; policy accept;\n\
             \x20       meta mark 0x5a return\n\
             \x20       ip daddr { 1.1.1.1, 9.9.9.9 } meta l4proto { tcp, udp } th dport 53 meta mark set 0x59\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" meta mark set 0x59\n\
             \x20   }\n\
             \x20   chain dns {\n\
             \x20       type nat hook output priority dstnat; policy accept;\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" ip daddr != 127.0.0.0/8 meta l4proto { tcp, udp } th dport 53 dnat ip to 10.2.0.1\n\
             \x20   }\n\
             \x20   chain postrouting {\n\
             \x20       type nat hook postrouting priority srcnat; policy accept;\n\
             \x20       oifname \"yutani0\" masquerade\n\
             \x20   }\n\
             \x20   chain killswitch {\n\
             \x20       type filter hook postrouting priority filter; policy accept;\n\
             \x20       oifname \"lo\" accept\n\
             \x20       meta mark 0x59 oifname != \"yutani0\" counter drop\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn nft_ruleset_without_dns_has_no_dns_chain_exact_text() {
        let r = nft_ruleset(1000, None, &servers());
        assert_eq!(
            r,
            "table inet yutani {\n\
             \x20   chain setmark {\n\
             \x20       type route hook output priority mangle; policy accept;\n\
             \x20       meta mark 0x5a return\n\
             \x20       ip daddr { 1.1.1.1, 9.9.9.9 } meta l4proto { tcp, udp } th dport 53 meta mark set 0x59\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" meta mark set 0x59\n\
             \x20   }\n\
             \x20   chain postrouting {\n\
             \x20       type nat hook postrouting priority srcnat; policy accept;\n\
             \x20       oifname \"yutani0\" masquerade\n\
             \x20   }\n\
             \x20   chain killswitch {\n\
             \x20       type filter hook postrouting priority filter; policy accept;\n\
             \x20       oifname \"lo\" accept\n\
             \x20       meta mark 0x59 oifname != \"yutani0\" counter drop\n\
             \x20   }\n\
             }\n"
        );
    }

    /// Lines of `chain`, trimmed, without its `type …` header or braces.
    fn chain_rules(ruleset: &str, chain: &str) -> Vec<String> {
        let lines: Vec<&str> = ruleset.lines().map(|l| l.trim()).collect();
        let i = lines.iter().position(|l| *l == format!("chain {chain} {{")).unwrap();
        lines[i + 2..]
            .iter()
            .take_while(|l| **l != "}")
            .map(|l| l.to_string())
            .collect()
    }

    /// The encrypted outer packet re-uses the inner packet's `sk_buff`, so
    /// it still carries the game's socket and matches our cgroup rules.
    /// WireGuard stamps `WG_FWMARK` on it; `setmark` must `return` before it
    /// can be re-marked (which would route it back into `yutani0` — an
    /// encrypt loop).
    #[test]
    fn the_wireguard_fwmark_is_exempt_before_any_cgroup_rule() {
        for r in [
            nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()), &servers()),
            nft_ruleset(1000, None, &servers()),
        ] {
            assert_eq!(chain_rules(&r, "setmark")[0], "meta mark 0x5a return");
        }
    }

    /// The kill-switch hooks POSTROUTING, not OUTPUT: the OUTPUT hook state
    /// captures `out` once, before the route chain re-routes the marked
    /// packet, so every later OUTPUT chain still sees the LAN interface and
    /// the drop rule would fire on a packet that is in fact going out of
    /// `yutani0`. Only POSTROUTING knows the real outgoing interface.
    ///
    /// Keying on `FWMARK` (not on `socket cgroupv2`, which nft rejects in a
    /// postrouting hook) is equivalent: `setmark` sets that mark only on
    /// cgroup-matched packets, so "marked for the tunnel but leaving
    /// elsewhere" is exactly the kill-switch condition. `WG_FWMARK` needs no
    /// exemption here — the outer packets carry `0x5a`, never `0x59`.
    #[test]
    fn the_kill_switch_hooks_postrouting_and_keys_on_the_mark() {
        for r in [
            nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()), &servers()),
            nft_ruleset(1000, None, &servers()),
        ] {
            assert!(
                r.contains(
                    "    chain killswitch {\n        type filter hook postrouting priority filter; policy accept;\n"
                ),
                "the kill-switch must hook postrouting:\n{r}"
            );
            assert_eq!(
                chain_rules(&r, "killswitch"),
                vec![
                    "oifname \"lo\" accept".to_string(),
                    "meta mark 0x59 oifname != \"yutani0\" counter drop".to_string(),
                ]
            );
            // `socket` is not permitted in a postrouting hook, and the
            // mark already implies the cgroup.
            assert!(!chain_rules(&r, "killswitch").iter().any(|l| l.contains("socket")));
        }
    }

    /// Syntax smoke test: pipe the generated ruleset through `nft -c -f -`.
    /// Skips (passes silently) if `nft` is not on PATH. An unprivileged run
    /// still exits non-zero and prints a `netlink: ... cache initialization
    /// failed: Operation not permitted` line even for valid syntax, so this
    /// only checks stderr for `syntax error` / `Error: syntax`, not the exit
    /// code.
    #[test]
    fn nft_ruleset_parses_with_nft_check() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let nft = if std::path::Path::new("/usr/bin/nft").exists() {
            "/usr/bin/nft"
        } else {
            "nft"
        };
        let mut child = match Command::new(nft)
            .args(["-c", "-f", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return, // nft not available: pass silently.
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()), &servers()).as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("syntax error") && !stderr.contains("Error: syntax"),
            "nft -c -f - reported a syntax error:\n{stderr}"
        );
    }

    #[test]
    fn up_commands_in_spec_order() {
        let cmds = up_commands(&conf(), "/run/yutani/wg.conf");
        let joined: Vec<String> = cmds.iter().map(|c| c.join(" ")).collect();
        assert_eq!(
            joined,
            vec![
                "ip link add yutani0 type wireguard",
                "wg setconf yutani0 /run/yutani/wg.conf",
                "wg set yutani0 fwmark 0x5a",
                "ip address add 10.2.0.2/32 dev yutani0",
                "ip link set yutani0 mtu 1420 up",
                "sysctl -q -w net.ipv4.conf.yutani0.rp_filter=2",
                "ip route add default dev yutani0 table 51820",
                "ip rule add fwmark 0x59 lookup 51820 priority 1000",
                "ip rule add from 10.2.0.2 lookup 51820 priority 1001",
            ]
        );
    }

    #[test]
    fn up_commands_honour_conf_mtu() {
        let mut c = conf();
        c.mtu = Some(1380);
        assert!(
            up_commands(&c, "/x")
                .iter()
                .any(|c| c.join(" ") == "ip link set yutani0 mtu 1380 up")
        );
    }

    /// The tunnel's resolvers are marked for the tunnel wherever the query
    /// comes from: `systemd-resolved` sends it from *its own* cgroup, which
    /// no `socket cgroupv2` rule of ours matches, so this rule must not be
    /// behind the cgroup match — and it must sit after the `WG_FWMARK`
    /// return like everything else in the chain.
    #[test]
    fn the_resolver_addresses_are_marked_for_the_tunnel_after_the_wireguard_exemption() {
        let r = nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()), &servers());
        assert_eq!(
            chain_rules(&r, "setmark")[..2],
            [
                "meta mark 0x5a return".to_string(),
                "ip daddr { 1.1.1.1, 9.9.9.9 } meta l4proto { tcp, udp } th dport 53 meta mark set 0x59".to_string(),
            ]
        );
    }

    /// A one-element nft set is still a set: `{ 1.1.1.1 }` is valid.
    /// No servers at all means no rule (an empty `{ }` is a syntax error).
    #[test]
    fn the_resolver_rule_follows_the_server_list() {
        let one = nft_ruleset(1000, None, &["8.8.8.8".parse().unwrap()]);
        assert!(
            one.contains("ip daddr { 8.8.8.8 } meta l4proto { tcp, udp } th dport 53 meta mark set 0x59"),
            "{one}"
        );
        let none = nft_ruleset(1000, None, &[]);
        assert!(!none.contains("th dport 53 meta mark set"), "{none}");
    }

    /// The three per-link settings, in the order the worker runs them.
    /// Domains are routing-only (`~`) so resolved sends *those* names to
    /// these servers and nothing else changes; `default-route false` keeps
    /// every other lookup on the machine's normal resolver.
    #[test]
    fn resolved_up_commands_set_the_link_dns_the_routing_domains_and_the_default_route() {
        let joined: Vec<String> =
            resolved_up_commands(&servers(), &domains()).iter().map(|c| c.join(" ")).collect();
        assert_eq!(
            joined,
            vec![
                "resolvectl dns yutani0 1.1.1.1 9.9.9.9",
                "resolvectl domain yutani0 ~eveonline.com ~ccpgames.com ~evetech.net",
                "resolvectl default-route yutani0 false",
            ]
        );
    }

    /// Per-link settings die with the interface, so this is belt and
    /// braces — but it must run before `ip link del yutani0`, while the
    /// link resolved is asked about still exists.
    #[test]
    fn resolved_down_command_reverts_the_link_and_runs_first() {
        assert_eq!(resolved_down_command().join(" "), "resolvectl revert yutani0");
        assert_eq!(down_commands()[0], resolved_down_command());
    }

    #[test]
    fn down_commands_reverse_everything_and_are_idempotent_shaped() {
        let joined: Vec<String> = down_commands().iter().map(|c| c.join(" ")).collect();
        assert_eq!(
            joined,
            vec![
                "resolvectl revert yutani0",
                "nft delete table inet yutani",
                "ip rule del fwmark 0x59 lookup 51820 priority 1000",
                "ip rule del lookup 51820 priority 1001",
                "ip route flush table 51820",
                "ip link del yutani0",
            ]
        );
    }

    /// `ip link set yutani0 down` makes the kernel drop
    /// `default dev yutani0 table 51820`, and bringing the link back up does
    /// not restore it — so the worker re-adds it every tick. `replace` is
    /// idempotent: it is a no-op when the route is already there.
    #[test]
    fn ensure_route_command_replaces_the_default_route_in_our_table() {
        assert_eq!(
            ensure_route_command().join(" "),
            "ip route replace default dev yutani0 table 51820"
        );
    }
}
