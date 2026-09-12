//! Pure generators for everything the root worker executes: the nftables
//! ruleset and the `ip`/`wg`/`sysctl` argv lists. Keeping these pure means
//! the exact commands are unit-tested and printable by `--dry-run`.

use std::net::Ipv4Addr;

use super::conf::WgConf;
use super::{FWMARK, IFACE, SLICE, TABLE};

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

pub fn nft_ruleset(uid: u32, dns: Option<Ipv4Addr>) -> String {
    let m = cgroup_match(uid);
    let mut s = String::from("table inet yutani {\n");
    s.push_str(
        "    chain setmark {\n        type route hook output priority mangle; policy accept;\n",
    );
    s.push_str(&format!("        {m} meta mark set {FWMARK:#x}\n    }}\n"));
    if let Some(dns) = dns {
        s.push_str(
            "    chain dns {\n        type nat hook output priority dstnat; policy accept;\n",
        );
        s.push_str(&format!(
            "        {m} meta l4proto {{ tcp, udp }} th dport 53 dnat ip to {dns}\n    }}\n"
        ));
    }
    s.push_str(
        "    chain killswitch {\n        type filter hook output priority filter; policy accept;\n",
    );
    s.push_str(&format!("        {m} oifname \"lo\" accept\n"));
    s.push_str(&format!(
        "        {m} oifname != \"{IFACE}\" counter drop\n    }}\n"
    ));
    s.push_str("}\n");
    s
}

/// Everything before loading the nft ruleset, in order. `wg_conf_path` is
/// the transient 0600 file holding `WgConf::wg_native()`.
pub fn up_commands(conf: &WgConf, wg_conf_path: &str) -> Vec<Vec<String>> {
    let mtu = conf.mtu.unwrap_or(1420).to_string();
    let addr = format!("{}/{}", conf.address, conf.prefix_len);
    let from = conf.address.to_string();
    let table = TABLE.to_string();
    let mark = format!("{FWMARK:#x}");
    vec![
        argv(&["ip", "link", "add", IFACE, "type", "wireguard"]),
        argv(&["wg", "setconf", IFACE, wg_conf_path]),
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

    #[test]
    fn nft_ruleset_marks_dnats_dns_and_kill_switches_exact_text_with_dns() {
        let r = nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()));
        assert_eq!(
            r,
            "table inet yutani {\n\
             \x20   chain setmark {\n\
             \x20       type route hook output priority mangle; policy accept;\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" meta mark set 0x59\n\
             \x20   }\n\
             \x20   chain dns {\n\
             \x20       type nat hook output priority dstnat; policy accept;\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" meta l4proto { tcp, udp } th dport 53 dnat ip to 10.2.0.1\n\
             \x20   }\n\
             \x20   chain killswitch {\n\
             \x20       type filter hook output priority filter; policy accept;\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" oifname \"lo\" accept\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" oifname != \"yutani0\" counter drop\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn nft_ruleset_without_dns_has_no_nat_chain_exact_text() {
        let r = nft_ruleset(1000, None);
        assert_eq!(
            r,
            "table inet yutani {\n\
             \x20   chain setmark {\n\
             \x20       type route hook output priority mangle; policy accept;\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" meta mark set 0x59\n\
             \x20   }\n\
             \x20   chain killswitch {\n\
             \x20       type filter hook output priority filter; policy accept;\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" oifname \"lo\" accept\n\
             \x20       socket cgroupv2 level 5 \"user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice\" oifname != \"yutani0\" counter drop\n\
             \x20   }\n\
             }\n"
        );
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
            .write_all(nft_ruleset(1000, Some("10.2.0.1".parse().unwrap())).as_bytes())
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

    #[test]
    fn down_commands_reverse_everything_and_are_idempotent_shaped() {
        let joined: Vec<String> = down_commands().iter().map(|c| c.join(" ")).collect();
        assert_eq!(
            joined,
            vec![
                "nft delete table inet yutani",
                "ip rule del fwmark 0x59 lookup 51820 priority 1000",
                "ip rule del lookup 51820 priority 1001",
                "ip route flush table 51820",
                "ip link del yutani0",
            ]
        );
    }
}
