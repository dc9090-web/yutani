//! EVE-only WireGuard tunnel: root worker, install, control, status.
//! See docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md.

pub mod conf;
pub mod control;
pub mod install;
pub mod rules;
pub mod status;
pub mod worker;

pub const IFACE: &str = "yutani0";
pub const FWMARK: u32 = 0x59;
/// WireGuard's own firewall mark, stamped by the kernel on every *encrypted*
/// outer packet (`wg set yutani0 fwmark 0x5a`). It exists because the kernel
/// re-uses the inner packet's `sk_buff` for the outer UDP datagram, so that
/// datagram still carries `skb->sk` — the game's socket, whose cgroup is
/// `yutani-eve.slice`. Our cgroup-matching rules would therefore also match
/// the encrypted packet: `setmark` would set `FWMARK` on it, `ip rule fwmark
/// 0x59 lookup 51820` would route it straight back into `yutani0` to be
/// encrypted again, and the loop would fill WireGuard's per-peer staged
/// queue (the interface's TX `dropped` counter climbs, `tx_errors` stays 0,
/// and the kernel logs nothing). The `killswitch` chain would then drop it
/// too, since a packet carrying `FWMARK` that leaves via the LAN interface
/// is exactly what it is there to stop. Marking the outer packet distinctly
/// keeps it out of both: `setmark` returns on `WG_FWMARK` before any cgroup
/// match, and the kill-switch only ever drops `FWMARK` — this is the same
/// reason wg-quick sets a firewall mark on its interfaces.
pub const WG_FWMARK: u32 = 0x5a;
pub const TABLE: u32 = 51820;
/// Resolvers EVE's name lookups are sent to *inside* the tunnel, and the
/// domains that go to them. `systemd-resolved` is told about both as
/// per-link settings on `yutani0` (`resolvectl dns|domain|default-route`)
/// while the tunnel is up, which is what closes the NSS gap in the design
/// doc's §2: `getaddrinfo` never emits a DNS packet from EVE's cgroup, so
/// only resolved itself can route those lookups — a `~domain` on the link
/// makes it do exactly that, and the `setmark` rule for these addresses
/// puts the resulting query into the tunnel. Overridable per user in
/// `config.ron` (`tunnel.dns_servers` / `tunnel.dns_domains`); the values
/// reach root only through `yutani tunnel install`, which stores them in
/// `/etc/yutani/tunnel.conf`.
pub const DEFAULT_DNS_SERVERS: [std::net::Ipv4Addr; 2] =
    [std::net::Ipv4Addr::new(1, 1, 1, 1), std::net::Ipv4Addr::new(9, 9, 9, 9)];
pub const DEFAULT_DNS_DOMAINS: [&str; 3] = ["eveonline.com", "ccpgames.com", "evetech.net"];

/// Whether `a` may be a `dns_servers` entry: an address the *exit node*
/// can reach. `rules::nft_ruleset` marks every packet to it on port 53 for
/// the tunnel, whoever sends it, and the masquerade hands it to the exit —
/// so a LAN resolver (`192.168.1.1`: the router, a Pi-hole, the obvious
/// thing to type), loopback, link-local or a placeholder address would no
/// longer be routed where it lives, and every lookup on the machine to it
/// would time out for as long as the tunnel is up, with the applet showing
/// a healthy tunnel. The same goes for shared address space (100.64.0.0/10,
/// RFC 6598 — Tailscale's MagicDNS resolver `100.100.100.100` is exactly
/// the kind of address a user would type) and multicast (224.0.0.0/4,
/// mDNS). Checked where the values are typed (`Config::validate`), where
/// root stores them (`install::dns_conf_lines`) and where root acts on
/// them (`worker`).
pub fn usable_dns_server(a: &std::net::Ipv4Addr) -> bool {
    // `Ipv4Addr::is_shared` is still unstable, hence the octet test.
    let shared = a.octets()[0] == 100 && a.octets()[1] & 0xC0 == 64;
    !(a.is_private() || shared || a.is_loopback() || a.is_link_local() || a.is_multicast() || a.is_unspecified() || a.is_broadcast())
}

/// The reason [`usable_dns_server`] refuses an address, for the messages
/// the three call sites print.
pub const UNUSABLE_DNS_SERVER: &str =
    "is a private, shared (100.64.0.0/10), loopback, link-local, multicast, unspecified or broadcast address, which the exit node cannot reach: while the tunnel was up every lookup on this machine to it would time out";
pub const SLICE: &str = "yutani-eve.slice";
pub const CONF_PATH: &str = "/etc/yutani/tunnel.conf";
pub const STATUS_PATH: &str = "/run/yutani/tunnel.json";
pub const UNIT_NAME: &str = "yutani-tunnel.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/yutani-tunnel.service";
pub const POLKIT_PATH: &str = "/etc/polkit-1/rules.d/50-yutani-tunnel.rules";

/// Write `path` with exactly `mode`, atomically.
///
/// The temporary is created in the target's directory with `mode` from the
/// start — files that hold key material must never be readable by anyone
/// else, not even for the instant between `write` and `chmod`. `open(2)`
/// masks the mode it is given with the umask, so the mode is also set
/// explicitly: a 0644 unit or polkit rule must stay readable by `systemd`
/// and `polkitd` however the caller's umask is set. The `rename` is the last
/// step, so a failure leaves the previous file in place.
fn write_with_mode(path: &str, text: &str, mode: u32) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let target = std::path::Path::new(path);
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => std::path::Path::new("."),
    };
    let name = target.file_name().and_then(|n| n.to_str()).with_context(|| format!("{path} is not a file path"))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));

    let attempt = || -> anyhow::Result<()> {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove stale {}", tmp.display())),
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(text.as_bytes()).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("chmod {mode:o} {}", tmp.display()))?;
        std::fs::rename(&tmp, target).with_context(|| format!("rename into place: {path}"))
    };

    let result = attempt();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Test-only helpers shared by this module's children.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Mutex, MutexGuard};

    // POSIX `mode_t umask(mode_t)`: tests that must prove a mode survives a
    // restrictive umask set it themselves.
    unsafe extern "C" {
        #[link_name = "umask"]
        pub(crate) fn libc_umask(mask: u32) -> u32;
    }

    static MODE_LOCK: Mutex<()> = Mutex::new(());

    /// The umask is process-wide, so tests that change it must not run
    /// concurrently with each other (or with any test that checks a mode).
    pub(crate) fn mode_lock() -> MutexGuard<'static, ()> {
        MODE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    use super::testing::{libc_umask, mode_lock};

    fn mode_of(path: &str) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yutani-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A resolver the exit node cannot reach must never become a
    /// `dns_servers` entry: the mark rule would send every query to it into
    /// the tunnel and the machine's DNS would silently die while connected.
    #[test]
    fn only_addresses_the_exit_node_can_reach_are_usable_dns_servers() {
        // The two neighbours of the shared range (100.64.0.0/10) are public.
        for ok in ["1.1.1.1", "9.9.9.9", "8.8.8.8", "94.140.14.14", "185.228.168.9", "100.63.255.255", "100.128.0.0"] {
            assert!(usable_dns_server(&ok.parse().unwrap()), "{ok} is a public resolver");
        }
        // 100.100.100.100 is Tailscale's MagicDNS resolver — shared address
        // space (RFC 6598), which only exists on this machine's tailnet; 224/4
        // is multicast (mDNS lives at 224.0.0.251).
        for bad in [
            "192.168.1.1", "10.0.0.1", "172.16.0.1", "127.0.0.1", "127.0.0.53", "169.254.1.1", "0.0.0.0", "255.255.255.255",
            "100.64.0.0", "100.100.100.100", "100.127.255.255", "224.0.0.251", "239.255.255.250",
        ] {
            assert!(!usable_dns_server(&bad.parse().unwrap()), "{bad} cannot be reached from the exit node");
        }
        assert!(DEFAULT_DNS_SERVERS.iter().all(usable_dns_server), "the defaults must pass their own check");
    }

    #[test]
    fn modes_are_exact_whatever_the_umask() {
        let dir = temp_dir("mode");
        let unit = dir.join("unit").to_string_lossy().into_owned();
        let key = dir.join("key").to_string_lossy().into_owned();
        // The umask is process-wide: hold the lock so a concurrent test
        // cannot observe (or set) a different one.
        let _lock = mode_lock();
        // SAFETY: umask has no preconditions and cannot fail; restored below.
        let old = unsafe { libc_umask(0o077) };
        let got = std::panic::catch_unwind(|| {
            write_with_mode(&unit, "unit\n", 0o644).unwrap();
            write_with_mode(&key, "key\n", 0o600).unwrap();
            (mode_of(&unit), mode_of(&key))
        });
        // SAFETY: as above.
        unsafe { libc_umask(old) };
        let (unit_mode, key_mode) = got.unwrap();
        assert_eq!(unit_mode, 0o644, "polkitd must be able to read a 0644 file under any umask");
        assert_eq!(key_mode, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_overwrite_replaces_the_content_and_leaves_no_temporary_behind() {
        let dir = temp_dir("atomic");
        let path = dir.join("f").to_string_lossy().into_owned();
        write_with_mode(&path, "old\n", 0o600).unwrap();
        write_with_mode(&path, "new\n", 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(mode_of(&path), 0o644);
        let left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["f".to_string()], "the temporary must be renamed, not left behind");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_keeps_the_target_and_removes_its_temporary() {
        let dir = temp_dir("atomic-fail");
        // A directory can never be replaced by `rename` of a file: the write
        // fails, the target survives, and nothing is left lying around.
        let target = dir.join("d");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("inside"), "keep").unwrap();
        let path = target.to_string_lossy().into_owned();
        assert!(write_with_mode(&path, "new\n", 0o600).is_err());
        assert_eq!(std::fs::read_to_string(target.join("inside")).unwrap(), "keep");
        let left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["d".to_string()], "a failed write must clean up its temporary");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
