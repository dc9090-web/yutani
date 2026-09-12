//! `yutani tunnel run`: the root worker behind `yutani-tunnel.service`.
//! Brings the tunnel up, publishes status once a second, tears everything
//! down on SIGTERM/SIGINT (and on any failure while coming up).

use anyhow::{Context as _, anyhow};
use std::os::unix::fs::PermissionsExt as _;
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

fn load(conf_path: &Path) -> anyhow::Result<(WgConf, u32)> {
    let text = std::fs::read_to_string(conf_path).with_context(|| format!("read {}", conf_path.display()))?;
    let label = conf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let conf = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf_path.display()))?;
    let uid = uid_from_conf(&text).context("conf has no `# yutani: uid = N` line; re-run `yutani tunnel install`")?;
    Ok((conf, uid))
}

/// Every step is attempted even if an earlier one fails: teardown must leave
/// nothing behind after a partial `up`.
fn down() {
    for argv in rules::down_commands() {
        if let Err(e) = exec(&argv) {
            tracing::debug!("teardown: {e:#}");
        }
    }
    let _ = std::fs::remove_file(STATUS_PATH);
    let _ = std::fs::remove_file(WG_CONF_TMP);
}

fn up(conf: &WgConf, uid: u32) -> anyhow::Result<()> {
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
    exec_stdin(&["nft".to_string(), "-f".to_string(), "-".to_string()], &rules::nft_ruleset(uid, conf.dns))?;
    Ok(())
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
    let tmp = format!("{STATUS_PATH}.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&file)?)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, STATUS_PATH)?;
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let (conf, uid) = load(Path::new(CONF_PATH))?;
    tracing::info!("tunnel up: {} via {} for uid {uid}", conf.label, conf.endpoint);
    if let Err(e) = up(&conf, uid) {
        tracing::error!("tunnel start failed: {e:#}; tearing down");
        down();
        return Err(e);
    }
    let since = now_unix();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let result: anyhow::Result<()> = rt.block_on(async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick.tick() => {
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
    tracing::info!("tunnel down");
    down();
    result
}

/// Everything `run` would execute for `conf_path`, secrets redacted.
pub fn dry_run(conf_path: &Path, uid: u32) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(conf_path).with_context(|| format!("read {}", conf_path.display()))?;
    let label = conf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let conf = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf_path.display()))?;
    let mut out = format!("# conf ({})\n{}\n# up\n", conf.label, conf.redacted());
    for argv in rules::up_commands(&conf, WG_CONF_TMP) {
        out.push_str(&argv.join(" "));
        out.push('\n');
    }
    out.push_str("nft -f - <<EOF\n");
    out.push_str(&rules::nft_ruleset(uid, conf.dns));
    out.push_str("EOF\n# down\n");
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
        let out = dry_run(&conf, 1000).unwrap();
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
    fn uid_comes_from_the_stored_conf_comment() {
        assert_eq!(uid_from_conf("# yutani: label = UK#1\n# yutani: uid = 1000\n[Interface]\n"), Some(1000));
        assert_eq!(uid_from_conf("[Interface]\nPrivateKey = k=\n"), None);
    }
}
