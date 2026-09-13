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

const WG_CONF_TMP: &str = "/run/yutani/wg.conf";

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The worker's one way of running anything. [`System`] is the real thing;
/// the tests hand `bring_up`/`down`/`serve` a stub that records each argv
/// and answers for it, so the order and the error handling of what root
/// executes are pinned without touching the machine.
trait Exec {
    /// Run `argv` and yield its stdout; `Err` carries the argv and stderr.
    /// With `timeout`, a command still running after that long is killed
    /// and reported as a failure like any other.
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> anyhow::Result<String>;
    /// Run `argv` with `stdin` on its standard input.
    fn run_with_stdin(&self, argv: &[String], stdin: &str) -> anyhow::Result<()>;
}

struct System;

fn failed(argv: &[String], out: &std::process::Output) -> anyhow::Error {
    anyhow!("`{}` failed ({}): {}", argv.join(" "), out.status, String::from_utf8_lossy(&out.stderr).trim())
}

impl Exec for System {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> anyhow::Result<String> {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        let out = match timeout {
            None => cmd.output().with_context(|| format!("cannot exec {}", argv[0]))?,
            Some(t) => crate::proc::output_with_timeout(&mut cmd, t)
                .with_context(|| format!("cannot exec {}", argv[0]))?
                .ok_or_else(|| anyhow!("`{}` outlived {t:?} and was killed", argv.join(" ")))?,
        };
        if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).into_owned()) } else { Err(failed(argv, &out)) }
    }

    fn run_with_stdin(&self, argv: &[String], stdin: &str) -> anyhow::Result<()> {
        use std::io::Write;
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot exec {}", argv[0]))?;
        child.stdin.take().context("stdin")?.write_all(stdin.as_bytes())?;
        let out = child.wait_with_output()?;
        if out.status.success() { Ok(()) } else { Err(failed(argv, &out)) }
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

/// Bounds on each teardown command. `systemctl stop` allows the worker
/// `TimeoutStopSec=10` in all and SIGKILLs it after that, so whatever has
/// not run yet never does — with the nft table still marking and dropping
/// EVE's packets and `/run/yutani` gone, so status says "disconnected".
/// `resolvectl revert` is a D-Bus call whose own reply timeout (25 s) is
/// longer than that whole budget, so a wedged resolved would eat all of it;
/// the netlink commands take milliseconds, and one that does not is stuck.
/// 1 + 5 × 1.5 = 8.5 s worst case, inside the budget with room to spare.
const RESOLVED_TIMEOUT: Duration = Duration::from_secs(1);
const NETLINK_TIMEOUT: Duration = Duration::from_millis(1500);

fn down_timeout(argv: &[String]) -> Duration {
    if argv.first().is_some_and(|c| c == "resolvectl") { RESOLVED_TIMEOUT } else { NETLINK_TIMEOUT }
}

/// Every step is attempted even if an earlier one fails: teardown must leave
/// nothing behind after a partial `up`. Failures are expected (an object that
/// never existed) but not hidden: the error carries the argv and the
/// command's stderr, and `info` is inside the unit's default filter.
fn down(x: &dyn Exec) {
    for argv in rules::down_commands() {
        if let Err(e) = x.run(&argv, Some(down_timeout(&argv))) {
            tracing::info!("teardown: {e:#}");
        }
    }
    let _ = std::fs::remove_file(STATUS_PATH);
    let _ = std::fs::remove_file(WG_CONF_TMP);
}

/// Before `up`: whatever a previous run left behind. A worker that ended
/// without its teardown — SIGKILLed after `TimeoutStopSec`, OOM-killed,
/// crashed — leaves `yutani0`, the two ip rules and the nft table in
/// place, and `ip link add` / `ip rule add` then fail with `File exists`
/// on every start until someone cleans up by hand. Each teardown command
/// fails harmlessly on an object that is not there, so running them first
/// costs a few milliseconds on a clean machine and makes a start
/// independent of how the previous run ended. The failures are the normal
/// case here, hence `debug`; the one thing worth a line is a link that
/// really was left behind.
fn remove_leftovers(x: &dyn Exec) {
    if Path::new(&format!("/sys/class/net/{IFACE}")).exists() {
        tracing::warn!("{IFACE} is still present from a previous run that did not tear down; removing it first");
    }
    for argv in rules::down_commands() {
        if let Err(e) = x.run(&argv, Some(down_timeout(&argv))) {
            tracing::debug!("pre-start cleanup: {e:#}");
        }
    }
    let _ = std::fs::remove_file(STATUS_PATH);
}

/// Runs its action once when it goes out of scope — normal return, `?`, or a
/// panic. `serve` holds one from before anything is created until it
/// returns, so the tunnel is torn down on every path out and never twice.
struct Teardown<'a>(Option<Box<dyn FnOnce() + 'a>>);

impl<'a> Teardown<'a> {
    fn new(action: impl FnOnce() + 'a) -> Self {
        Teardown(Some(Box::new(action)))
    }
}

impl Drop for Teardown<'_> {
    fn drop(&mut self) {
        if let Some(action) = self.0.take() {
            action();
        }
    }
}

/// `wg_conf_tmp` is the transient 0600 file `wg setconf` reads
/// ([`WG_CONF_TMP`] in the real worker; a scratch path in tests).
fn up(x: &dyn Exec, conf: &WgConf, uid: u32, dns_servers: &[std::net::Ipv4Addr], wg_conf_tmp: &Path) -> anyhow::Result<()> {
    if let Some(dir) = wg_conf_tmp.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let wg_conf_tmp = wg_conf_tmp.to_str().context("wg conf path is not UTF-8")?;
    write_with_mode(wg_conf_tmp, &conf.wg_native(), 0o600)?;
    for argv in rules::up_commands(conf, wg_conf_tmp) {
        let r = x.run(&argv, None);
        // The conf is only needed by `wg setconf`; drop it as soon as that
        // command has run, failure or not — and on any earlier failure,
        // so the key never outlives the step that stopped needing it.
        if argv.first().is_some_and(|c| c == "wg") || r.is_err() {
            let _ = std::fs::remove_file(wg_conf_tmp);
        }
        r?;
    }
    let _ = std::fs::remove_file(wg_conf_tmp);
    x.run_with_stdin(&["nft".to_string(), "-f".to_string(), "-".to_string()], &rules::nft_ruleset(uid, conf.dns, dns_servers))?;
    Ok(())
}

/// Hand `systemd-resolved` the per-link DNS for `yutani0` once the tunnel
/// is up. A failure here (resolved not running, an older `resolvectl`) is
/// *not* a reason to tear the tunnel down: everything else works, and what
/// is lost is only the DNS improvement — EVE's lookups then behave exactly
/// as they did before this feature (the design doc's §2 gap). So: warn,
/// loudly enough to be found in the journal, and carry on.
fn resolved_up(x: &dyn Exec, servers: &[std::net::Ipv4Addr], domains: &[String]) {
    for argv in rules::resolved_up_commands(servers, domains) {
        if let Err(e) = x.run(&argv, None) {
            tracing::warn!("resolved: {e:#}; EVE's DNS lookups will not go through the tunnel");
        }
    }
}

/// Everything between "the conf is loaded" and "the tunnel is up", in
/// order: clear leftovers, `up`, then resolved — the link must exist before
/// resolved can be told anything about it, and the nft rule that puts
/// those queries into the tunnel is loaded by `up` itself.
fn bring_up(x: &dyn Exec, loaded: &Loaded, wg_conf_tmp: &Path) -> anyhow::Result<()> {
    remove_leftovers(x);
    up(x, &loaded.conf, loaded.uid, &loaded.dns_servers, wg_conf_tmp)?;
    resolved_up(x, &loaded.dns_servers, &loaded.dns_domains);
    Ok(())
}

/// `IFF_UP` (bit 0) of `/sys/class/net/<iface>/flags`, which is hex.
fn link_flags_say_up(flags: &str) -> bool {
    let hex = flags.trim().trim_start_matches("0x");
    u32::from_str_radix(hex, 16).is_ok_and(|f| f & 1 != 0)
}

fn link_is_up() -> bool {
    std::fs::read_to_string(format!("/sys/class/net/{IFACE}/flags")).is_ok_and(|f| link_flags_say_up(&f))
}

/// Whether this tick has to re-assert the route: the link is up and either
/// nothing has asserted it since the loop began or the link was down on the
/// previous tick. Every other tick leaves the routing table alone — an
/// unconditional `ip route replace` still emits an `RTM_NEWROUTE`
/// notification for an unchanged route, and NetworkManager, networkd,
/// resolved and avahi would each wake up on it once a second for nothing.
fn route_needs_readd(link_was_up: bool, link_is_up: bool) -> bool {
    link_is_up && !link_was_up
}

/// Re-assert `default dev yutani0 table 51820`. The kernel deletes that
/// route whenever the link goes down (`ip link set yutani0 down`) and never
/// puts it back when the link comes up again; without it marked packets
/// fall through to the LAN route and the kill-switch drops them for the
/// rest of the session. `ip route replace` is idempotent, so re-asserting
/// it on the first tick is harmless. A failure (the link vanished between
/// the sysfs read and the command) is the expected state, not a reason to
/// tear the tunnel down, so it is logged at debug and otherwise ignored.
fn ensure_route(x: &dyn Exec) {
    if let Err(e) = x.run(&rules::ensure_route_command(), None) {
        tracing::debug!("route: {e:#}");
    }
}

/// How long the worker keeps a known exit address before looking again.
/// The answer only changes when the peer does, so this is a refresh, not a
/// poll: five minutes costs one `curl` per five minutes of uptime.
pub const EXIT_IP_REFRESH_S: u64 = 300;

/// How long the exit-IP `curl` may take before it is killed. Comfortably
/// above curl's own `-m 6` so the process timeout is only ever the backstop
/// for a curl that ignores it (a stuck DNS resolve, an unkillable TLS
/// handshake), not the normal way out.
const EXIT_IP_TIMEOUT: Duration = Duration::from_secs(8);

/// Ask what the public internet sees this machine as, *through the tunnel*.
///
/// `--interface <tunnel address>` is the whole trick: it binds the request
/// to 10.2.0.2, which is exactly what the policy rule `from 10.2.0.2`
/// matches, so the query is routed down `yutani0` and answered by the exit
/// node rather than by the LAN's own uplink. `-4` because the tunnel is
/// v4-only and a v6 answer would be some other path entirely.
pub fn exit_ip_command(addr: std::net::Ipv4Addr) -> Vec<String> {
    ["curl", "-4", "-sS", "-m", "6", "--interface", &addr.to_string(), "https://api.ipify.org"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// The body of that request, if it is an IPv4 address and nothing else.
///
/// Anything else — an error page, a captive portal's redirect, a v6
/// address, two addresses — is not an answer, and the applet must never be
/// handed a string that only *looks* like an address.
pub fn parse_exit_ip(body: &str) -> Option<std::net::Ipv4Addr> {
    body.trim().parse().ok()
}

/// When the exit address is worth asking for again: the first look after
/// the link came up (`last_refresh` 0), the moment the peer first
/// handshakes (the route is only then carrying traffic, so the answer can
/// differ from the one before it), and every [`EXIT_IP_REFRESH_S`]
/// thereafter. `saturating_sub` keeps a backwards clock step from becoming
/// a refresh on every tick.
pub fn should_refresh_exit_ip(last_refresh: u64, now: u64, handshake_changed: bool) -> bool {
    last_refresh == 0 || handshake_changed || now.saturating_sub(last_refresh) >= EXIT_IP_REFRESH_S
}

/// What the worker remembers between ticks about the exit address.
struct ExitIp {
    /// The last address that parsed. Kept across failures: a tunnel that is
    /// still up has not changed its exit node just because one `curl` could
    /// not reach api.ipify.org, and blanking the field would flap the
    /// applet's band between an address and the internal one.
    address: Option<String>,
    /// Unix seconds of the last *attempt* (not the last success): a failing
    /// query must back off exactly as a succeeding one does, or a broken
    /// network would mean one `curl` per second.
    last_refresh: u64,
    /// The handshake timestamp seen on the previous tick, to spot the 0 →
    /// non-zero transition.
    last_handshake: u64,
}

impl ExitIp {
    fn new() -> Self {
        ExitIp { address: None, last_refresh: 0, last_handshake: 0 }
    }

    /// Refresh `address` if this tick is one of the moments
    /// [`should_refresh_exit_ip`] names. Every failure keeps the previous
    /// value and is logged at debug: the exit address is a nicety on the
    /// applet's band, never a reason to disturb a working tunnel.
    fn refresh(&mut self, x: &dyn Exec, addr: std::net::Ipv4Addr, handshake: u64, now: u64) {
        let handshake_changed = self.last_handshake == 0 && handshake > 0;
        self.last_handshake = handshake;
        if !should_refresh_exit_ip(self.last_refresh, now, handshake_changed) {
            return;
        }
        self.last_refresh = now;
        match x.run(&exit_ip_command(addr), Some(EXIT_IP_TIMEOUT)) {
            Ok(body) => match parse_exit_ip(&body) {
                Some(ip) => self.address = Some(ip.to_string()),
                None => tracing::debug!("exit ip: answer is not an IPv4 address"),
            },
            Err(e) => tracing::debug!("exit ip: {e:#}"),
        }
    }
}

fn write_status(x: &dyn Exec, conf: &WgConf, since: u64, exit: &mut ExitIp) -> anyhow::Result<()> {
    let dump = x.run(&["wg".to_string(), "show".to_string(), IFACE.to_string(), "dump".to_string()], None)?;
    let (endpoint, handshake, rx, tx) = parse_wg_dump(&dump).unwrap_or((conf.endpoint.clone(), 0, 0, 0));
    exit.refresh(x, conf.address, handshake, now_unix());
    let file = TunnelFile {
        up: true,
        iface: IFACE.into(),
        address: conf.address.to_string(),
        endpoint,
        latest_handshake_unix: handshake,
        rx_bytes: rx,
        tx_bytes: tx,
        since_unix: since,
        exit_address: exit.address.clone(),
    };
    // `write_with_mode` creates the temporary with 0644 from the start and
    // renames it into place, so the file is never briefly unreadable and a
    // failed write leaves the previous status intact.
    write_with_mode(STATUS_PATH, &serde_json::to_string(&file)?, 0o644)
}

/// The worker's life once the signal handlers are in place: bring the
/// tunnel up, publish status once a second, return on SIGTERM/SIGINT or
/// on a failure to come up. The teardown guard is taken here, before
/// anything is created, so every way out — a signal, `?`, a panic — tears
/// the tunnel down exactly once; it is the only caller of `down`.
async fn serve(
    x: &dyn Exec,
    loaded: &Loaded,
    wg_conf_tmp: &Path,
    term: &mut tokio::signal::unix::Signal,
    int: &mut tokio::signal::unix::Signal,
) -> anyhow::Result<()> {
    let _teardown = Teardown::new(|| down(x));
    if let Err(e) = bring_up(x, loaded, wg_conf_tmp) {
        tracing::error!("tunnel start failed: {e:#}; tearing down");
        return Err(e);
    }
    let since = now_unix();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    // The exit-IP refresh can block a tick for as long as
    // `EXIT_IP_TIMEOUT`; without this an interval that fell behind
    // would then fire every missed tick back to back (tokio's default
    // `Burst`) instead of simply carrying on once a second.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut exit = ExitIp::new();
    let mut link_was_up = false;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let up_now = link_is_up();
                if route_needs_readd(link_was_up, up_now) {
                    ensure_route(x);
                }
                link_was_up = up_now;
                if let Err(e) = write_status(x, &loaded.conf, since, &mut exit) {
                    tracing::warn!("status: {e:#}");
                }
            }
            _ = term.recv() => break,
            _ = int.recv() => break,
        }
    }
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let loaded = load(Path::new(CONF_PATH))?;
    tracing::info!("tunnel up: {} via {} for uid {}", loaded.conf.label, loaded.conf.endpoint, loaded.uid);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let result: anyhow::Result<()> = rt.block_on(async {
        // The handlers go in before anything is created. Until `signal()`
        // has run, SIGTERM and SIGINT keep their default disposition and
        // kill the process on the spot — no unwinding, so no `Teardown` —
        // and a `systemctl stop` during `up` (a Disconnect right after a
        // Connect, a `restart`, `quit` just after a connect) would leave the
        // link, the rules and the nft table behind while systemd records a
        // clean stop. That window is the whole of `up`: seconds, when
        // `wg setconf` is resolving a hostname `Endpoint`. tokio keeps a
        // signal that lands between registration and the first `recv`, so
        // one that arrives while `up` is running ends the loop as soon as
        // it is entered — after `up`, and followed by the teardown.
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        serve(&System, &loaded, Path::new(WG_CONF_TMP), &mut term, &mut int).await
    });
    // The guard dropped inside `serve`, so this line lands in the journal
    // after the commands that took the tunnel down (and any failure among
    // them), never ahead of them.
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

    /// The exit IP is fetched *through* the tunnel: the policy rule
    /// `from 10.2.0.2` is what routes it into `yutani0`, and `--interface`
    /// is what gives curl that source address.
    #[test]
    fn the_exit_ip_is_asked_for_through_the_tunnel_address() {
        let addr: std::net::Ipv4Addr = "10.2.0.2".parse().unwrap();
        assert_eq!(exit_ip_command(addr), vec![
            "curl",
            "-4",
            "-sS",
            "-m",
            "6",
            "--interface",
            "10.2.0.2",
            "https://api.ipify.org"
        ]);
    }

    /// api.ipify.org answers with a bare address and no newline, but a
    /// proxy, a captive portal or an error page can answer with anything —
    /// and anything that is not an IPv4 address must never reach the applet.
    #[test]
    fn only_a_bare_ipv4_address_is_accepted_as_the_exit_ip() {
        assert_eq!(parse_exit_ip("198.51.100.10"), Some("198.51.100.10".parse().unwrap()));
        assert_eq!(parse_exit_ip("  198.51.100.10\n"), Some("198.51.100.10".parse().unwrap()));
        assert_eq!(parse_exit_ip(""), None);
        assert_eq!(parse_exit_ip("   "), None);
        assert_eq!(parse_exit_ip("<html>error</html>"), None);
        assert_eq!(parse_exit_ip("2a00:1450::1"), None);
        assert_eq!(parse_exit_ip("198.51.100.10 198.51.100.11"), None);
    }

    /// Rarely, and on the two events that can change the answer: the first
    /// look after the link came up (`last_refresh` 0) and the first
    /// handshake after none. Otherwise every five minutes — a `curl` in the
    /// one-second status loop is not something to run on every tick.
    #[test]
    fn the_exit_ip_is_refreshed_on_the_events_that_can_change_it_and_rarely_otherwise() {
        // Never looked yet.
        assert!(should_refresh_exit_ip(0, 1_789_180_000, false));
        // Just looked.
        assert!(!should_refresh_exit_ip(1_789_180_000, 1_789_180_001, false));
        // The peer handshaked for the first time: the route is only now
        // carrying traffic, so the answer may differ from the one before it.
        assert!(should_refresh_exit_ip(1_789_180_000, 1_789_180_001, true));
        // The five-minute floor.
        assert!(!should_refresh_exit_ip(1_789_180_000, 1_789_180_000 + EXIT_IP_REFRESH_S - 1, false));
        assert!(should_refresh_exit_ip(1_789_180_000, 1_789_180_000 + EXIT_IP_REFRESH_S, false));
        // A clock that stepped backwards must not turn into a refresh storm.
        assert!(!should_refresh_exit_ip(1_789_180_000, 1_000, false));
    }

    use std::cell::RefCell;

    /// Records every argv (and the bound it was given) instead of running
    /// it. `fail` says which commands answer with an error, the way the
    /// real ones do on an object that is not there.
    struct Recorder {
        calls: RefCell<Vec<(Vec<String>, Option<Duration>)>>,
        stdin: RefCell<Vec<String>>,
        fail: fn(&[String]) -> bool,
    }

    impl Recorder {
        fn new(fail: fn(&[String]) -> bool) -> Self {
            Recorder { calls: RefCell::new(Vec::new()), stdin: RefCell::new(Vec::new()), fail }
        }

        fn joined(&self) -> Vec<String> {
            self.calls.borrow().iter().map(|(argv, _)| argv.join(" ")).collect()
        }
    }

    impl Exec for Recorder {
        fn run(&self, argv: &[String], timeout: Option<Duration>) -> anyhow::Result<String> {
            self.calls.borrow_mut().push((argv.to_vec(), timeout));
            if (self.fail)(argv) { Err(anyhow!("`{}` failed (stub)", argv.join(" "))) } else { Ok(String::new()) }
        }

        fn run_with_stdin(&self, argv: &[String], stdin: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push((argv.to_vec(), None));
            self.stdin.borrow_mut().push(stdin.to_string());
            if (self.fail)(argv) { Err(anyhow!("`{}` failed (stub)", argv.join(" "))) } else { Ok(()) }
        }
    }

    /// What a clean machine answers: every teardown command fails because
    /// there is nothing to tear down.
    fn nothing_to_tear_down(argv: &[String]) -> bool {
        rules::down_commands().iter().any(|d| d == argv)
    }

    fn loaded() -> Loaded {
        let conf = WgConf::parse(
            "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n",
            "t",
        )
        .unwrap();
        let dns_servers = crate::tunnel::DEFAULT_DNS_SERVERS.to_vec();
        let dns_domains = crate::tunnel::DEFAULT_DNS_DOMAINS.iter().map(|d| (*d).to_string()).collect();
        Loaded { conf, uid: 1000, dns_servers, dns_domains }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-worker-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn joined(cmds: &[Vec<String>]) -> Vec<String> {
        cmds.iter().map(|c| c.join(" ")).collect()
    }

    /// A worker that ended without its teardown (SIGKILL after
    /// `TimeoutStopSec`, an OOM kill, a crash) leaves `yutani0`, the ip
    /// rules and the nft table behind, and `ip link add` / `ip rule add`
    /// then fail with `File exists` on every start. So a start begins by
    /// running the teardown, whose failures on a clean machine are
    /// expected and ignored — and only then brings the link up and tells
    /// resolved about it.
    #[test]
    fn a_start_clears_leftovers_then_brings_the_link_up_then_tells_resolved() {
        let dir = scratch("start");
        let wg = dir.join("wg.conf");
        let rec = Recorder::new(nothing_to_tear_down);
        let l = loaded();
        bring_up(&rec, &l, &wg).unwrap();
        let mut expected = joined(&rules::down_commands());
        expected.extend(joined(&rules::up_commands(&l.conf, wg.to_str().unwrap())));
        expected.push("nft -f -".to_string());
        expected.extend(joined(&rules::resolved_up_commands(&l.dns_servers, &l.dns_domains)));
        assert_eq!(rec.joined(), expected);
        assert_eq!(rec.stdin.borrow().as_slice(), [rules::nft_ruleset(1000, l.conf.dns, &l.dns_servers)]);
        assert!(!wg.exists(), "the wg conf holds the private key and must not outlive `wg setconf`");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failure while coming up stops right there: nothing after the
    /// failed command runs (resolved is never told about a link that does
    /// not exist), and the error names the command.
    #[test]
    fn a_failure_while_coming_up_stops_there_and_names_the_command() {
        let dir = scratch("start-fail");
        let wg = dir.join("wg.conf");
        fn link_add_fails(argv: &[String]) -> bool {
            nothing_to_tear_down(argv) || argv.join(" ") == "ip link add yutani0 type wireguard"
        }
        let rec = Recorder::new(link_add_fails);
        let e = bring_up(&rec, &loaded(), &wg).unwrap_err().to_string();
        assert!(e.contains("ip link add yutani0"), "got {e}");
        let calls = rec.joined();
        assert_eq!(calls.last().map(String::as_str), Some("ip link add yutani0 type wireguard"));
        assert!(!calls.iter().any(|c| c.starts_with("resolvectl dns")));
        assert!(!wg.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `systemctl stop` gives the worker `TimeoutStopSec=10` in all and
    /// SIGKILLs it after that, so whatever has not run yet never does —
    /// with the nft table still marking and dropping EVE's packets.
    /// `resolvectl revert` is a D-Bus call whose own reply timeout (25 s)
    /// alone exceeds the budget, so every teardown command is bounded and
    /// the bounds add up to less than the budget.
    #[test]
    fn every_teardown_command_is_bounded_inside_the_stop_budget() {
        for pass in [down as fn(&dyn Exec), remove_leftovers] {
            let rec = Recorder::new(nothing_to_tear_down);
            pass(&rec);
            let calls = rec.calls.borrow();
            assert_eq!(calls.len(), rules::down_commands().len());
            let mut total = Duration::ZERO;
            for (argv, timeout) in calls.iter() {
                let timeout = timeout.expect("every teardown command must carry a bound");
                let want = if argv[0] == "resolvectl" { RESOLVED_TIMEOUT } else { NETLINK_TIMEOUT };
                assert_eq!(timeout, want, "{}", argv.join(" "));
                total += timeout;
            }
            assert!(total < Duration::from_secs(10), "the bounds add up to {total:?}, more than TimeoutStopSec");
        }
    }

    /// The real runner honours that bound: a command that hangs is killed
    /// and reported as a failure, not waited for.
    #[test]
    fn the_system_runner_kills_a_command_that_outlives_its_bound() {
        let sleep = ["sleep".to_string(), "10".to_string()];
        let started = std::time::Instant::now();
        let e = System.run(&sleep, Some(Duration::from_millis(100))).unwrap_err().to_string();
        assert!(started.elapsed() < Duration::from_secs(2), "must not wait for the child's own 10 s");
        assert!(e.contains("outlived"), "got {e}");
        assert_eq!(System.run(&["echo".to_string(), "hi".to_string()], Some(Duration::from_secs(5))).unwrap().trim(), "hi");
        assert!(System.run(&["false".to_string()], None).is_err());
        assert!(System.run(&["true".to_string()], None).is_ok());
    }

    /// The stop that used to lose the tunnel: SIGTERM while `up` is still
    /// running. The handlers are registered *before* anything is created,
    /// and tokio keeps a signal that lands between registration and the
    /// first `recv`, so a stop that arrives during `up` is honoured as soon
    /// as the loop is entered — after `up` has finished, and followed by a
    /// full teardown.
    #[test]
    fn a_stop_that_arrives_while_the_tunnel_is_coming_up_still_tears_it_down() {
        use tokio::signal::unix::{SignalKind, signal};
        let dir = scratch("signal");
        let wg = dir.join("wg.conf");
        let rec = Recorder::new(nothing_to_tear_down);
        let l = loaded();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let started = std::time::Instant::now();
        let result: anyhow::Result<()> = rt.block_on(async {
            let mut term = signal(SignalKind::terminate())?;
            let mut int = signal(SignalKind::interrupt())?;
            // Delivered before `serve` has run a single command — the
            // handler is in place, so it is queued rather than fatal.
            let st = Command::new("kill").args(["-TERM", &std::process::id().to_string()]).status()?;
            assert!(st.success());
            serve(&rec, &l, &wg, &mut term, &mut int).await
        });
        result.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "the queued signal must end the loop at once");
        let calls = rec.joined();
        let up = joined(&rules::up_commands(&l.conf, wg.to_str().unwrap()));
        let down = joined(&rules::down_commands());
        // Leftover cleanup, the whole of `up`, resolved — nothing skipped.
        assert_eq!(&calls[..down.len()], &down[..]);
        assert_eq!(&calls[down.len()..down.len() + up.len()], &up[..]);
        assert!(calls.iter().any(|c| c.starts_with("resolvectl dns yutani0")));
        // And the teardown ran at the end, exactly once.
        assert_eq!(&calls[calls.len() - down.len()..], &down[..]);
        assert_eq!(calls.iter().filter(|c| *c == "ip link del yutani0").count(), 2, "one leftover pass, one teardown");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `ip link set yutani0 down` makes the kernel drop the table-51820
    /// route and bringing the link back up does not restore it. Only that
    /// transition needs the route re-added: doing it every second would be
    /// an `RTM_NEWROUTE` notification per second for NetworkManager,
    /// resolved and friends to wake up on.
    #[test]
    fn the_route_is_re_added_on_the_first_tick_and_when_the_link_comes_back_up() {
        assert!(route_needs_readd(false, true), "first tick: the link is up and nothing has asserted the route yet");
        assert!(!route_needs_readd(true, true), "steady state: nothing to do");
        assert!(!route_needs_readd(true, false), "the link went down: the route is gone and cannot be added yet");
        assert!(!route_needs_readd(false, false));
        assert!(route_needs_readd(false, true), "the link came back: this is the case the tick exists for");
        // `/sys/class/net/<iface>/flags` is hex with IFF_UP as bit 0.
        assert!(link_flags_say_up("0x1003\n"));
        assert!(link_flags_say_up("0x1"));
        assert!(!link_flags_say_up("0x1002\n"));
        assert!(!link_flags_say_up(""));
        assert!(!link_flags_say_up("junk"));
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
