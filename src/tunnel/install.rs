//! `yutani tunnel install|uninstall`: one-time privileged setup through
//! `pkexec`, and the root-side `install-root|uninstall-root` it invokes.

use anyhow::{Context as _, anyhow, ensure};
use std::path::Path;
use std::process::Command;

use super::conf::WgConf;
use super::{CONF_PATH, POLKIT_PATH, UNIT_NAME, UNIT_PATH, write_with_mode};

fn quote_exec(exe: &str) -> String {
    if exe.chars().any(|c| c.is_whitespace()) { format!("\"{exe}\"") } else { exe.to_string() }
}

pub fn unit_text(exe: &str) -> String {
    format!(
        "[Unit]\nDescription=Yutani EVE tunnel (WireGuard, per-app routing)\nAfter=network-online.target\nWants=network-online.target\n\n\
[Service]\nType=simple\nExecStart={} tunnel run\nRuntimeDirectory=yutani\nRuntimeDirectoryMode=0755\nKillSignal=SIGTERM\nTimeoutStopSec=10\nRestart=no\n\n\
[Install]\nWantedBy=multi-user.target\n",
        quote_exec(exe)
    )
}

pub fn polkit_text(username: &str) -> String {
    format!(
        "// Installed by `yutani tunnel install`: lets {username} start/stop the EVE tunnel unit without a password.\n\
polkit.addRule(function(action, subject) {{\n\
    if (action.id == \"org.freedesktop.systemd1.manage-units\" &&\n\
        action.lookup(\"unit\") == \"{UNIT_NAME}\" &&\n\
        (action.lookup(\"verb\") == \"start\" || action.lookup(\"verb\") == \"stop\" ||\n\
         action.lookup(\"verb\") == \"restart\") &&\n\
        subject.user == \"{username}\") {{\n\
        return polkit.Result.YES;\n\
    }}\n\
}});\n"
    )
}

fn current_exe() -> anyhow::Result<String> {
    Ok(std::env::current_exe()?.canonicalize()?.to_string_lossy().into_owned())
}

fn whoami() -> anyhow::Result<(u32, String)> {
    let uid = crate::ipc::uid();
    let name = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).context("USER not set")?;
    Ok((uid, name))
}

/// User side: `pkexec <self> tunnel install-root …` (one password prompt).
pub fn install(conf: &Path) -> anyhow::Result<()> {
    let conf = conf.canonicalize().with_context(|| format!("{}", conf.display()))?;
    let exe = current_exe()?;
    let (uid, user) = whoami()?;
    let status = Command::new("pkexec")
        .args([&exe, "tunnel", "install-root", "--conf"])
        .arg(&conf)
        .args(["--uid", &uid.to_string(), "--user", &user, "--exe", &exe])
        .status()
        .context("pkexec")?;
    ensure!(status.success(), "install cancelled or failed (pkexec exit {status})");
    println!("Tunnel installed. Next: `yutani tunnel connect`. Delete {} (it holds the private key and is world-readable).", conf.display());
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let exe = current_exe()?;
    let status = Command::new("pkexec").args([&exe, "tunnel", "uninstall-root"]).status().context("pkexec")?;
    ensure!(status.success(), "uninstall cancelled or failed (pkexec exit {status})");
    println!("Tunnel uninstalled.");
    Ok(())
}

fn write(path: &str, text: &str, mode: u32) -> anyhow::Result<()> {
    if let Some(dir) = Path::new(path).parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    write_with_mode(path, text, mode)
}

/// The stored conf as it is safe to print: key material replaced.
fn redact_lines(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.trim_start().starts_with("PrivateKey") || l.trim_start().starts_with("PresharedKey") {
                format!("{} = <redacted>", l.split('=').next().unwrap_or("").trim())
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Root side. With `dry_run`, returns what would be written instead of writing.
pub fn install_root(conf: &Path, uid: u32, username: &str, exe: &str, dry_run: bool) -> anyhow::Result<String> {
    ensure!(conf.is_absolute() && Path::new(exe).is_absolute(), "paths must be absolute");
    let text = std::fs::read_to_string(conf).with_context(|| format!("read {}", conf.display()))?;
    let label = conf.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let parsed = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf.display()))?;
    let stored = format!("# yutani: label = {}\n# yutani: uid = {uid}\n{text}", parsed.label);
    let unit = unit_text(exe);
    let rule = polkit_text(username);
    let report =
        format!("{CONF_PATH} (0600):\n{}\n{UNIT_PATH}:\n{unit}\n{POLKIT_PATH}:\n{rule}", redact_lines(&stored));
    if dry_run {
        return Ok(report);
    }
    write(CONF_PATH, &stored, 0o600)?;
    write(UNIT_PATH, &unit, 0o644)?;
    write(POLKIT_PATH, &rule, 0o644)?;
    let st = Command::new("systemctl").arg("daemon-reload").status().context("systemctl daemon-reload")?;
    ensure!(st.success(), "systemctl daemon-reload failed");
    if parsed.dns.is_none() {
        eprintln!("warning: the conf has no DNS entry; EVE's DNS lookups will not go through the tunnel");
    }
    Ok(report)
}

pub fn uninstall_root(dry_run: bool) -> anyhow::Result<String> {
    let report = format!("stop {UNIT_NAME}; remove {CONF_PATH}, {UNIT_PATH}, {POLKIT_PATH}; systemctl daemon-reload\n");
    if dry_run {
        return Ok(report);
    }
    let _ = Command::new("systemctl").args(["stop", UNIT_NAME]).status();
    for p in [CONF_PATH, UNIT_PATH, POLKIT_PATH] {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {p}")),
        }
    }
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_text_runs_the_given_binary_as_the_worker() {
        let u = unit_text("/opt/yutani/yutani");
        assert!(u.contains("[Unit]\nDescription=Yutani EVE tunnel (WireGuard, per-app routing)\n"));
        assert!(u.contains("After=network-online.target\nWants=network-online.target\n"));
        assert!(u.contains("[Service]\nType=simple\nExecStart=/opt/yutani/yutani tunnel run\n"));
        assert!(u.contains("RuntimeDirectory=yutani\nRuntimeDirectoryMode=0755\n"));
        assert!(u.contains("KillSignal=SIGTERM\nTimeoutStopSec=10\nRestart=no\n"));
        assert!(u.contains("[Install]\nWantedBy=multi-user.target\n"));
    }

    #[test]
    fn polkit_text_authorises_one_unit_for_one_user() {
        let p = polkit_text("daniel");
        assert!(p.contains(r#"action.id == "org.freedesktop.systemd1.manage-units""#));
        assert!(p.contains(r#"action.lookup("unit") == "yutani-tunnel.service""#));
        assert!(p.contains(r#"action.lookup("verb") == "start""#));
        assert!(p.contains(r#"action.lookup("verb") == "stop""#));
        assert!(p.contains(r#"action.lookup("verb") == "restart""#));
        assert!(p.contains(r#"subject.user == "daniel""#));
        assert!(p.contains("polkit.Result.YES"));
        assert!(!p.contains("polkit.Result.NO"));
    }

    #[test]
    fn exe_paths_with_spaces_are_quoted_in_the_unit() {
        assert!(unit_text("/home/me/My Apps/yutani").contains("ExecStart=\"/home/me/My Apps/yutani\" tunnel run\n"));
    }

    #[test]
    fn install_root_dry_run_writes_nothing_and_hides_the_key() {
        let existed = Path::new(CONF_PATH).exists();
        let dir = std::env::temp_dir().join(format!("yutani-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("EVE-UK-455.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\n# UK#455\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", true).unwrap();
        assert!(report.contains("# yutani: label = UK#455"));
        assert!(report.contains("# yutani: uid = 1000"));
        assert!(report.contains("PrivateKey = <redacted>"));
        assert!(!report.contains("U0VDUkVU"));
        assert!(report.contains("ExecStart=/opt/yutani/yutani tunnel run"));
        assert!(report.contains(r#"subject.user == "daniel""#));
        assert_eq!(Path::new(CONF_PATH).exists(), existed, "a dry run must not touch {CONF_PATH}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_root_rejects_relative_paths_and_bad_confs() {
        assert!(install_root(Path::new("rel.conf"), 1000, "d", "/opt/y", true).unwrap_err().to_string().contains("absolute"));
        let dir = std::env::temp_dir().join(format!("yutani-inst-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("bad.conf");
        std::fs::write(&conf, "[Interface]\nAddress = 10.2.0.2/32\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        assert!(install_root(&conf, 1000, "d", "/opt/y", true).unwrap_err().to_string().contains("PrivateKey"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uninstall_root_dry_run_names_every_path_it_would_remove() {
        let r = uninstall_root(true).unwrap();
        assert!(r.contains(UNIT_NAME) && r.contains(CONF_PATH) && r.contains(UNIT_PATH) && r.contains(POLKIT_PATH));
    }
}
