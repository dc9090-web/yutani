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

/// The binary the unit will run, and the one `pkexec` re-executes: the
/// canonical path, so `--dry-run` and the real install agree even when the
/// binary was invoked through a symlink.
pub fn current_exe() -> anyhow::Result<String> {
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
    println!("{}", success_message(&conf));
    Ok(())
}

/// What `install` prints once the root side succeeded. A tunnel that is
/// already up keeps running on the *old* conf until it is restarted (spec
/// §8), so say how.
fn success_message(conf: &Path) -> String {
    format!(
        "Tunnel installed. Next: `yutani tunnel connect`.\n\
If the tunnel is already running, run `yutani tunnel disconnect && yutani tunnel connect` to pick up the new conf.\n\
Delete {} (it holds the private key and is world-readable).",
        conf.display()
    )
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
/// `WgConf::parse` accepts keys case-insensitively, so `privatekey = ...`
/// is a valid conf and must be redacted just like `PrivateKey`.
fn redact_lines(text: &str) -> String {
    text.lines()
        .map(|l| match l.split_once('=') {
            Some((k, _))
                if k.trim().eq_ignore_ascii_case("PrivateKey") || k.trim().eq_ignore_ascii_case("PresharedKey") =>
            {
                format!("{} = <redacted>", k.trim())
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// POSIX portable user names: `[a-z_][a-z0-9_-]*$?`. The name is pasted into
/// a polkit JS string, so anything else (a quote, a space, non-ASCII) is
/// refused rather than escaped.
fn valid_username(name: &str) -> bool {
    let core = name.strip_suffix('$').unwrap_or(name);
    let mut chars = core.chars();
    let Some(first) = chars.next() else { return false };
    (first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// `pkexec` sets `PKEXEC_UID` to the calling user's uid: if it disagrees with
/// `--uid`, the arguments were not the ones the user's `install` generated.
fn check_pkexec_uid(pkexec_uid: Option<&str>, uid: u32) -> anyhow::Result<()> {
    let Some(raw) = pkexec_uid else { return Ok(()) };
    let got: u32 = raw.trim().parse().map_err(|_| anyhow!("PKEXEC_UID is {raw:?}, which is not a uid"))?;
    ensure!(got == uid, "PKEXEC_UID is {got} but --uid is {uid}: refusing to install for another user");
    Ok(())
}

/// Root side. With `dry_run`, returns what would be written instead of writing.
pub fn install_root(conf: &Path, uid: u32, username: &str, exe: &str, dry_run: bool) -> anyhow::Result<String> {
    ensure!(conf.is_absolute() && Path::new(exe).is_absolute(), "paths must be absolute");
    ensure!(valid_username(username), "{username:?} is not a POSIX portable user name ([a-z_][a-z0-9_-]*$)");
    check_pkexec_uid(std::env::var("PKEXEC_UID").ok().as_deref(), uid)?;
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

/// Uninstall proceeds even when nothing is installed, so a stop that had
/// nothing to stop is silent; anything else is the user's business.
fn stop_failure_worth_warning(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    !(s.contains("not loaded") || s.contains("not found"))
}

fn stop_unit() {
    match Command::new("systemctl").args(["stop", UNIT_NAME]).output() {
        Ok(out) if !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stop_failure_worth_warning(&stderr) {
                tracing::warn!("`systemctl stop {UNIT_NAME}` failed ({}): {}", out.status, stderr.trim());
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("cannot run `systemctl stop {UNIT_NAME}`: {e}"),
    }
}

pub fn uninstall_root(dry_run: bool) -> anyhow::Result<String> {
    let report = format!("stop {UNIT_NAME}; remove {CONF_PATH}, {UNIT_PATH}, {POLKIT_PATH}; systemctl daemon-reload\n");
    if dry_run {
        return Ok(report);
    }
    stop_unit();
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
    fn redaction_does_not_depend_on_how_the_conf_spells_the_key() {
        // `WgConf::parse` accepts keys case-insensitively, so the report must
        // redact them the same way or the private key is printed verbatim.
        let dir = std::env::temp_dir().join(format!("yutani-inst-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, "[Interface]\nprivatekey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\n# UK#455\nPublicKey = p=\nPRESHAREDKEY = UFNLU0VDUkVU\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", true).unwrap();
        assert!(!report.contains("U0VDUkVU"), "the private key leaked into the report: {report}");
        assert!(!report.contains("UFNLU0VDUkVU"), "the preshared key leaked into the report");
        assert!(report.contains("privatekey = <redacted>"));
        assert!(report.contains("PRESHAREDKEY = <redacted>"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_root_rejects_user_names_it_cannot_safely_embed() {
        let dir = std::env::temp_dir().join(format!("yutani-inst-user-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let e = install_root(&conf, 1000, "a\"b", "/opt/y", true).unwrap_err().to_string();
        assert!(e.contains("user name"), "expected a clear message, got {e}");
        assert!(install_root(&conf, 1000, "daniel_2-x", "/opt/y", true).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn user_names_follow_the_posix_portable_set() {
        for ok in ["daniel", "_svc", "a-b_c", "eve$", "u0"] {
            assert!(valid_username(ok), "{ok} should be accepted");
        }
        for bad in ["", "Daniel", "0day", "a\"b", "a b", "a;b", "root\\", "hé"] {
            assert!(!valid_username(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn pkexec_uid_must_agree_with_the_requested_uid() {
        assert!(check_pkexec_uid(None, 1000).is_ok(), "not run under pkexec: nothing to check");
        assert!(check_pkexec_uid(Some("1000"), 1000).is_ok());
        let e = check_pkexec_uid(Some("0"), 1000).unwrap_err().to_string();
        assert!(e.contains("PKEXEC_UID"), "got {e}");
        assert!(check_pkexec_uid(Some("nonsense"), 1000).is_err());
    }

    #[test]
    fn a_stop_that_had_nothing_to_stop_is_not_worth_a_warning() {
        assert!(!stop_failure_worth_warning("Failed to stop yutani-tunnel.service: Unit yutani-tunnel.service not loaded."));
        assert!(!stop_failure_worth_warning("Unit yutani-tunnel.service not found."));
        assert!(stop_failure_worth_warning("Failed to stop yutani-tunnel.service: Access denied"));
    }

    #[test]
    fn the_success_message_says_how_to_pick_up_a_changed_conf() {
        let m = success_message(Path::new("/home/d/EVE.conf"));
        assert!(m.contains("yutani tunnel connect"));
        assert!(m.contains("yutani tunnel disconnect && yutani tunnel connect"));
        assert!(m.contains("/home/d/EVE.conf"));
    }

    #[test]
    fn uninstall_root_dry_run_names_every_path_it_would_remove() {
        let r = uninstall_root(true).unwrap();
        assert!(r.contains(UNIT_NAME) && r.contains(CONF_PATH) && r.contains(UNIT_PATH) && r.contains(POLKIT_PATH));
    }
}
