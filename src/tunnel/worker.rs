//! `yutani tunnel run`: the root worker behind `yutani-tunnel.service`.
//! Brings the tunnel up, publishes status once a second, tears everything
//! down on SIGTERM/SIGINT (and on any failure while coming up).

use anyhow::{Context as _, anyhow};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::conf::WgConf;
use super::status::{TunnelFile, parse_wg_dump};
use super::{CONF_PATH, IFACE, STATUS_PATH, rules, write_with_mode};

const RUN_DIR: &str = "/run/yutani";
const WG_CONF_TMP: &str = "/run/yutani/wg.conf";

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run one command; `Err` carries the argv and stderr.
fn exec(argv: &[String]) -> anyhow::Result<String> {
    let out = Command::new(&argv[0]).args(&argv[1..]).output().with_context(|| format!("cannot exec {}", argv[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(anyhow!("`{}` failed ({}): {}", argv.join(" "), out.status, String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn exec_stdin(argv: &[String], stdin: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot exec {}", argv[0]))?;
    child.stdin.take().context("stdin")?.write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(anyhow!("`{}` failed ({}): {}", argv.join(" "), out.status, String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Which uid's cgroup to match: the unit runs as root, so the target user is
/// recorded in the conf by `install-root` (`# yutani: uid = 1000`).
fn uid_from_conf(text: &str) -> Option<u32> {
    text.lines().find_map(|l| l.trim().strip_prefix("# yutani: uid =")).and_then(|v| v.trim().parse().ok())
}

/// The value of one `# yutani: <key> = a b c` line, split on whitespace.
fn conf_list<'a>(text: &'a str, key: &str) -> Option<Vec<&'a str>> {
    let prefix = format!("# yutani: {key} =");
    text.lines().find_map(|l| l.trim().strip_prefix(prefix.as_str())).map(|v| v.split_whitespace().collect())
}

/// The resolvers `systemd-resolved` is pointed at for EVE's domains, from
/// `# yutani: dns_servers = 1.1.1.1 9.9.9.9`. Root never reads the user's
/// `config.ron` (spec §9): `install-root` copies the values into the conf
/// it owns. A conf written before this feature has no such line, and a line
/// whose values are all unusable is no better than a missing one — both
/// give the defaults, so an old install keeps working.
fn dns_servers_from_conf(text: &str) -> Vec<std::net::Ipv4Addr> {
    let parsed: Vec<std::net::Ipv4Addr> =
        conf_list(text, "dns_servers").unwrap_or_default().iter().filter_map(|s| s.parse().ok()).collect();
    if parsed.is_empty() { super::DEFAULT_DNS_SERVERS.to_vec() } else { parsed }
}

/// The domains routed to those resolvers, from `# yutani: dns_domains = …`.
/// Re-validated here even though `install-root` validated them: this text
/// is what root acts on, and a plain host name is all `resolvectl domain`
/// may ever be handed.
fn dns_domains_from_conf(text: &str) -> Vec<String> {
    let parsed: Vec<String> = conf_list(text, "dns_domains")
        .unwrap_or_default()
        .iter()
        .filter(|d| crate::model::config::valid_dns_domain(d))
        .map(|d| (*d).to_string())
        .collect();
    if parsed.is_empty() {
        super::DEFAULT_DNS_DOMAINS.iter().map(|d| (*d).to_string()).collect()
    } else {
        parsed
    }
}

/// Everything the worker needs, all of it from the root-owned conf.
struct Loaded {
    conf: WgConf,
    uid: u32,
    dns_servers: Vec<std::net::Ipv4Addr>,
    dns_domains: Vec<String>,
}

fn load(conf_path: &Path) -> anyhow::Result<Loaded> {
    let text = std::fs::read_to_string(conf_path).with_context(|| format!("read {}", conf_path.display()))?;
    let label = conf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let conf = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf_path.display()))?;
    let uid = uid_from_conf(&text).context("conf has no `# yutani: uid = N` line; re-run `yutani tunnel install`")?;
    Ok(Loaded { conf, uid, dns_servers: dns_servers_from_conf(&text), dns_domains: dns_domains_from_conf(&text) })
}

/// Every step is attempted even if an earlier one fails: teardown must leave
/// nothing behind after a partial `up`. Failures are expected (an object that
/// never existed) but not hidden: `exec`'s error carries the argv and the
/// command's stderr, and `info` is inside the unit's default filter.
fn down() {
    for argv in rules::down_commands() {
        if let Err(e) = exec(&argv) {
            tracing::info!("teardown: {e:#}");
        }
    }
    let _ = std::fs::remove_file(STATUS_PATH);
    let _ = std::fs::remove_file(WG_CONF_TMP);
}

/// Runs its action once when it goes out of scope — normal return, `?`, or a
/// panic. `run` holds one from just before `up` until it returns, so the
/// tunnel is torn down on every path out and never twice.
struct Teardown(Option<fn()>);

impl Teardown {
    fn new(action: fn()) -> Self {
        Teardown(Some(action))
    }
}

impl Drop for Teardown {
    fn drop(&mut self) {
        if let Some(action) = self.0.take() {
            action();
        }
    }
}

fn up(conf: &WgConf, uid: u32, dns_servers: &[std::net::Ipv4Addr]) -> anyhow::Result<()> {
    std::fs::create_dir_all(RUN_DIR)?;
    write_with_mode(WG_CONF_TMP, &conf.wg_native(), 0o600)?;
    for argv in rules::up_commands(conf, WG_CONF_TMP) {
        let r = exec(&argv);
        // The conf is only needed by `wg setconf`; drop it as soon as that
        // command has run, failure or not.
        if argv.first().is_some_and(|c| c == "wg") {
            let _ = std::fs::remove_file(WG_CONF_TMP);
        }
        r?;
    }
    let _ = std::fs::remove_file(WG_CONF_TMP);
    exec_stdin(&["nft".to_string(), "-f".to_string(), "-".to_string()], &rules::nft_ruleset(uid, conf.dns, dns_servers))?;
    Ok(())
}

/// Hand `systemd-resolved` the per-link DNS for `yutani0` once the tunnel
/// is up. A failure here (resolved not running, an older `resolvectl`) is
/// *not* a reason to tear the tunnel down: everything else works, and what
/// is lost is only the DNS improvement — EVE's lookups then behave exactly
/// as they did before this feature (the design doc's §2 gap). So: warn,
/// loudly enough to be found in the journal, and carry on.
fn resolved_up(servers: &[std::net::Ipv4Addr], domains: &[String]) {
    for argv in rules::resolved_up_commands(servers, domains) {
        if let Err(e) = exec(&argv) {
            tracing::warn!("resolved: {e:#}; EVE's DNS lookups will not go through the tunnel");
        }
    }
}

/// Re-assert `default dev yutani0 table 51820`. The kernel deletes that
/// route whenever the link goes down (`ip link set yutani0 down`) and never
/// puts it back when the link comes up again; without it marked packets
/// fall through to the LAN route and the kill-switch drops them for the
/// rest of the session. `ip route replace` is idempotent, so the tick can
/// run it unconditionally. While the link really is down the command fails
/// — that is the expected state, not a reason to tear the tunnel down, so
/// it is logged at debug and otherwise ignored.
fn ensure_route() {
    if let Err(e) = exec(&rules::ensure_route_command()) {
        tracing::debug!("route: {e:#}");
    }
}

fn write_status(conf: &WgConf, since: u64) -> anyhow::Result<()> {
    let dump = exec(&["wg".to_string(), "show".to_string(), IFACE.to_string(), "dump".to_string()])?;
    let (endpoint, handshake, rx, tx) = parse_wg_dump(&dump).unwrap_or((conf.endpoint.clone(), 0, 0, 0));
    let file = TunnelFile {
        up: true,
        iface: IFACE.into(),
        address: conf.address.to_string(),
        endpoint,
        latest_handshake_unix: handshake,
        rx_bytes: rx,
        tx_bytes: tx,
        since_unix: since,
    };
    // `write_with_mode` creates the temporary with 0644 from the start and
    // renames it into place, so the file is never briefly unreadable and a
    // failed write leaves the previous status intact.
    write_with_mode(STATUS_PATH, &serde_json::to_string(&file)?, 0o644)
}

pub fn run() -> anyhow::Result<()> {
    let Loaded { conf, uid, dns_servers, dns_domains } = load(Path::new(CONF_PATH))?;
    tracing::info!("tunnel up: {} via {} for uid {uid}", conf.label, conf.endpoint);
    // From here on every exit tears the tunnel down: the guard is the only
    // caller of `down`, so it happens exactly once.
    let _teardown = Teardown::new(down);
    if let Err(e) = up(&conf, uid, &dns_servers) {
        tracing::error!("tunnel start failed: {e:#}; tearing down");
        return Err(e);
    }
    // After `up`: the link must exist before resolved can be told anything
    // about it, and the nft rule that puts these queries into the tunnel is
    // loaded by `up` itself.
    resolved_up(&dns_servers, &dns_domains);
    let since = now_unix();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let result: anyhow::Result<()> = rt.block_on(async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    ensure_route();
                    if let Err(e) = write_status(&conf, since) {
                        tracing::warn!("status: {e:#}");
                    }
                }
                _ = term.recv() => break,
                _ = int.recv() => break,
            }
        }
        Ok(())
    });
    // Drop the guard before logging: teardown runs on drop, so logging first
    // would put "tunnel down" in the journal ahead of the commands that take
    // it down (and ahead of any failure among them).
    drop(_teardown);
    tracing::info!("tunnel down");
    result
}

/// Everything `run` would execute for `conf_path`, secrets redacted.
pub fn dry_run(
    conf_path: &Path,
    uid: u32,
    dns_servers: &[std::net::Ipv4Addr],
    dns_domains: &[String],
) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(conf_path).with_context(|| format!("read {}", conf_path.display()))?;
    let label = conf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let conf = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf_path.display()))?;
    let mut out = format!("# conf ({})\n{}\n# up\n", conf.label, conf.redacted());
    for argv in rules::up_commands(&conf, WG_CONF_TMP) {
        out.push_str(&argv.join(" "));
        out.push('\n');
    }
    out.push_str("nft -f - <<EOF\n");
    out.push_str(&rules::nft_ruleset(uid, conf.dns, dns_servers));
    out.push_str("EOF\n");
    for argv in rules::resolved_up_commands(dns_servers, dns_domains) {
        out.push_str(&argv.join(" "));
        out.push('\n');
    }
    out.push_str("# down\n");
    for argv in rules::down_commands() {
        out.push_str(&argv.join(" "));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_lists_commands_and_ruleset_without_secrets() {
        let dir = std::env::temp_dir().join(format!("yutani-dry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("t.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\n# UK#1\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let out = dry_run(&conf, 1000, &crate::tunnel::DEFAULT_DNS_SERVERS, &[]).unwrap();
        assert!(out.contains("ip link add yutani0 type wireguard"));
        assert!(out.contains("wg setconf yutani0 /run/yutani/wg.conf"));
        assert!(out.contains("table inet yutani {"));
        assert!(out.contains("dnat ip to 10.2.0.1"));
        assert!(out.contains("nft delete table inet yutani"));
        assert!(out.contains("PrivateKey = <redacted>"));
        assert!(!out.contains("U0VDUkVU"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_teardown_guard_runs_exactly_once_on_every_way_out() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        fn bump() {
            RUNS.fetch_add(1, Ordering::SeqCst);
        }
        {
            let _guard = Teardown::new(bump);
        }
        assert_eq!(RUNS.load(Ordering::SeqCst), 1, "dropping the guard tears down once");
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(|| {
            let _guard = Teardown::new(bump);
            panic!("worker blew up");
        });
        std::panic::set_hook(hook);
        assert!(r.is_err());
        assert_eq!(RUNS.load(Ordering::SeqCst), 2, "a panic must still tear down, exactly once");
    }

    #[test]
    fn uid_comes_from_the_stored_conf_comment() {
        assert_eq!(uid_from_conf("# yutani: label = UK#1\n# yutani: uid = 1000\n[Interface]\n"), Some(1000));
        assert_eq!(uid_from_conf("[Interface]\nPrivateKey = k=\n"), None);
    }

    /// The DNS settings travel the same way the uid does: written into the
    /// root-owned conf by `install-root`, never read from the user's
    /// `config.ron` by root (spec §9). A conf written before this feature
    /// has neither line, and must keep working — hence the defaults.
    #[test]
    fn the_dns_settings_come_from_the_stored_conf_comments() {
        let text = "# yutani: label = UK#1\n# yutani: uid = 1000\n\
                    # yutani: dns_servers = 8.8.8.8 8.8.4.4\n\
                    # yutani: dns_domains = example.net example.org\n[Interface]\n";
        assert_eq!(dns_servers_from_conf(text), vec![
            "8.8.8.8".parse::<std::net::Ipv4Addr>().unwrap(),
            "8.8.4.4".parse().unwrap()
        ]);
        assert_eq!(dns_domains_from_conf(text), vec!["example.net", "example.org"]);

        let old = "# yutani: uid = 1000\n[Interface]\n";
        assert_eq!(dns_servers_from_conf(old), crate::tunnel::DEFAULT_DNS_SERVERS.to_vec());
        assert_eq!(dns_domains_from_conf(old), crate::tunnel::DEFAULT_DNS_DOMAINS.to_vec());

        // Unparseable or empty values fall back rather than leaving the
        // worker with an empty list (which would mean "no DNS in the tunnel"
        // while every other part of the setup says there is).
        let junk = "# yutani: dns_servers = nonsense\n# yutani: dns_domains =   \n";
        assert_eq!(dns_servers_from_conf(junk), crate::tunnel::DEFAULT_DNS_SERVERS.to_vec());
        assert_eq!(dns_domains_from_conf(junk), crate::tunnel::DEFAULT_DNS_DOMAINS.to_vec());

        // A domain that is not a plain host name never reaches `resolvectl`.
        assert_eq!(
            dns_domains_from_conf("# yutani: dns_domains = eveonline.com bad~domain\n"),
            vec!["eveonline.com"]
        );
    }

    #[test]
    fn dry_run_lists_the_resolved_commands_and_the_resolver_mark_rule() {
        let dir = std::env::temp_dir().join(format!("yutani-dry-dns-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("t.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let servers = crate::tunnel::DEFAULT_DNS_SERVERS.to_vec();
        let domains: Vec<String> = crate::tunnel::DEFAULT_DNS_DOMAINS.iter().map(|d| (*d).to_string()).collect();
        let out = dry_run(&conf, 1000, &servers, &domains).unwrap();
        assert!(out.contains("ip daddr { 1.1.1.1, 9.9.9.9 } meta l4proto { tcp, udp } th dport 53 meta mark set 0x59"));
        assert!(out.contains("resolvectl dns yutani0 1.1.1.1 9.9.9.9"));
        assert!(out.contains("resolvectl domain yutani0 ~eveonline.com ~ccpgames.com ~evetech.net"));
        assert!(out.contains("resolvectl default-route yutani0 false"));
        assert!(out.contains("resolvectl revert yutani0"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
