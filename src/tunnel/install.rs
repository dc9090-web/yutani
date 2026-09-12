//! `yutani tunnel install|uninstall`: one-time privileged setup through
//! `pkexec`, and the root-side `install-root|uninstall-root` it invokes.

use anyhow::{Context as _, anyhow, bail, ensure};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;
use std::process::Command;

use super::conf::WgConf;
use super::{CONF_PATH, POLKIT_PATH, UNIT_NAME, UNIT_PATH, write_with_mode};

fn quote_exec(exe: &str) -> String {
    if exe.chars().any(|c| c.is_whitespace()) { format!("\"{exe}\"") } else { exe.to_string() }
}

/// `Environment=PATH=…` is explicit: systemd's default PATH for system
/// units does not include `/usr/sbin`, where `ip`, `wg`, `nft` and `sysctl`
/// live on some distributions, and the worker execs them by name.
pub fn unit_text(exe: &str) -> String {
    format!(
        "[Unit]\nDescription=Yutani EVE tunnel (WireGuard, per-app routing)\nAfter=network-online.target\nWants=network-online.target\n\n\
[Service]\nType=simple\nExecStart={} tunnel run\nEnvironment=PATH=/usr/sbin:/usr/bin:/sbin:/bin\nRuntimeDirectory=yutani\nRuntimeDirectoryMode=0755\nKillSignal=SIGTERM\nTimeoutStopSec=10\nRestart=no\n\n\
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

/// Invariant behind every check in this file: **nothing the user can write is
/// executed or read by root.** A path satisfies it when it is owned by root
/// and neither group- nor world-writable; the sticky bit (`/tmp`) does not
/// count, since anyone can still create entries there.
pub fn is_trusted(uid: u32, mode: u32) -> bool {
    uid == 0 && mode & 0o022 == 0
}

/// `Ok` when `p` and every one of its ancestors satisfies [`is_trusted`].
/// The path is canonicalised first, so a symlink cannot hide an untrusted
/// component, and a path that does not exist is refused outright.
pub fn trusted_path(p: &Path) -> Result<(), String> {
    let real = p.canonicalize().map_err(|e| format!("{}: {e}", p.display()))?;
    for ancestor in real.ancestors() {
        let md = std::fs::symlink_metadata(ancestor).map_err(|e| format!("{}: {e}", ancestor.display()))?;
        if !is_trusted(md.uid(), md.mode()) {
            return Err(format!(
                "{} is owned by uid {} with mode {:04o}",
                ancestor.display(),
                md.uid(),
                md.mode() & 0o7777
            ));
        }
    }
    Ok(())
}

/// What to tell the user when the binary they ran cannot be the unit's
/// `ExecStart`. Says exactly what to type, because the fix is not obvious.
fn untrusted_exe_message(exe: &str, conf: &Path, why: &str) -> String {
    format!(
        "refusing {exe}: the tunnel unit runs it as root, so it must be root-owned and not writable by others ({why}). Install it first:\n  sudo install -o root -g root -m 0755 {exe} /usr/local/bin/yutani\nthen re-run: /usr/local/bin/yutani tunnel install {}",
        conf.display()
    )
}

/// The name the system has for `uid` (root side; `id -un` avoids linking
/// `getpwuid`). `None` when the uid has no passwd entry.
fn username_for_uid(uid: u32) -> Option<String> {
    let out = Command::new("id").args(["-un", &uid.to_string()]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// The polkit rule is written for `claimed`, but the cgroup rules key on
/// `uid`: if those are different people the tunnel would be controllable by
/// one user and routed for another. `resolved` is what the system says.
fn check_username(uid: u32, claimed: &str, resolved: Option<&str>) -> anyhow::Result<()> {
    match resolved {
        Some(name) if name == claimed => Ok(()),
        Some(name) => bail!("--user is {claimed:?} but uid {uid} is {name:?}: refusing to install for another user"),
        None => bail!("uid {uid} has no user name on this system; refusing to install"),
    }
}

pub fn whoami() -> anyhow::Result<(u32, String)> {
    let uid = crate::ipc::uid();
    let name = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).context("USER not set")?;
    Ok((uid, name))
}

/// User side: `pkexec <self> tunnel install-root …` (one password prompt).
pub fn install(conf: &Path) -> anyhow::Result<()> {
    let conf = conf.canonicalize().with_context(|| format!("{}", conf.display()))?;
    let exe = current_exe()?;
    // Check before the prompt, not after: the root side refuses this anyway,
    // and there is no point spending the user's password on it.
    if let Err(why) = trusted_path(Path::new(&exe)) {
        bail!("{}", untrusted_exe_message(&exe, &conf, &why));
    }
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

/// `create_dir_all` applies the caller's umask, and `pkexec` does not reset
/// it — so the mode is set explicitly afterwards rather than hoped for.
fn create_dir_with_mode(dir: &Path, mode: u32) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {mode:o} {}", dir.display()))
}

/// The directory root will read these files from must itself be untouchable
/// by the user (see [`is_trusted`]): an existing one is verified, a missing
/// one is created with `mode`.
fn ensure_dir(dir: &Path, mode: u32) -> anyhow::Result<()> {
    if dir.exists() {
        trusted_path(dir).map_err(|why| anyhow!("refusing to use {}: {why}", dir.display()))?;
        return Ok(());
    }
    create_dir_with_mode(dir, mode)
}

fn write(path: &str, text: &str, mode: u32, dir_mode: u32) -> anyhow::Result<()> {
    if let Some(dir) = Path::new(path).parent() {
        ensure_dir(dir, dir_mode)?;
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
    // Both checks are hard errors for a real install and *reported* by a dry
    // run, which is unprivileged and exists to tell the user what is wrong.
    let mut checks = String::new();
    let refuse = |msg: String, checks: &mut String| -> anyhow::Result<()> {
        if !dry_run {
            bail!("{msg}");
        }
        checks.push_str(&format!("# check FAILED — the real install would stop here:\n{msg}\n\n"));
        Ok(())
    };
    match trusted_path(Path::new(exe)) {
        Ok(()) => checks.push_str(&format!("# {exe} is root-owned and not writable by others: OK\n\n")),
        Err(why) => refuse(untrusted_exe_message(exe, conf, &why), &mut checks)?,
    }
    match check_username(uid, username, username_for_uid(uid).as_deref()) {
        Ok(()) => checks.push_str(&format!("# uid {uid} is {username:?}: OK\n\n")),
        Err(e) => refuse(format!("{e:#}"), &mut checks)?,
    }
    let report =
        format!("{checks}{CONF_PATH} (0600):\n{}\n{UNIT_PATH}:\n{unit}\n{POLKIT_PATH}:\n{rule}", redact_lines(&stored));
    if dry_run {
        return Ok(report);
    }
    // 0700: only root ever reads the conf, and it holds the private key.
    write(CONF_PATH, &stored, 0o600, 0o700)?;
    write(UNIT_PATH, &unit, 0o644, 0o755)?;
    write(POLKIT_PATH, &rule, 0o644, 0o755)?;
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

    const GOOD_CONF: &str = "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\n# UK#455\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n";

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yutani-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unit_text_runs_the_given_binary_as_the_worker() {
        let u = unit_text("/opt/yutani/yutani");
        assert!(u.contains("[Unit]\nDescription=Yutani EVE tunnel (WireGuard, per-app routing)\n"));
        assert!(u.contains("After=network-online.target\nWants=network-online.target\n"));
        assert!(u.contains("[Service]\nType=simple\nExecStart=/opt/yutani/yutani tunnel run\n"));
        // systemd's default PATH for system units has no /usr/sbin, where
        // `ip`, `wg`, `nft` and `sysctl` live on some distributions.
        assert!(u.contains("Environment=PATH=/usr/sbin:/usr/bin:/sbin:/bin\n"));
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

    #[test]
    fn only_root_owned_paths_that_others_cannot_write_are_trusted() {
        assert!(is_trusted(0, 0o755), "root-owned, nobody else writes: trusted");
        assert!(is_trusted(0, 0o700));
        assert!(!is_trusted(1000, 0o755), "a user-owned path is never trusted");
        assert!(!is_trusted(0, 0o775), "group-writable is not trusted");
        assert!(!is_trusted(0, 0o757), "world-writable is not trusted");
        assert!(!is_trusted(0, 0o1777), "the sticky bit does not make /tmp trusted");
    }

    #[test]
    fn a_path_the_test_user_owns_is_refused() {
        // There is no way to create a root-owned path from the test suite, so
        // this pins the rejection side: anything under a user-owned directory
        // must be refused, naming the offending component.
        let dir = temp_dir("trust");
        let exe = dir.join("yutani");
        std::fs::write(&exe, "#!/bin/true\n").unwrap();
        let why = trusted_path(&exe).unwrap_err();
        assert!(why.contains("yutani") || why.contains(&dir.display().to_string()), "got {why}");
        assert!(trusted_path(&dir).is_err());
        assert!(trusted_path(Path::new("/nonexistent-yutani-path")).is_err(), "a missing path is not trusted");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_refusal_says_exactly_how_to_install_the_binary_as_root() {
        let m = untrusted_exe_message("/home/d/y/target/debug/yutani", Path::new("/home/d/EVE.conf"), "owned by uid 1000");
        assert!(m.contains("refusing /home/d/y/target/debug/yutani"));
        assert!(m.contains("must be root-owned and not writable by others"));
        assert!(m.contains("sudo install -o root -g root -m 0755 /home/d/y/target/debug/yutani /usr/local/bin/yutani"));
        assert!(m.contains("/usr/local/bin/yutani tunnel install /home/d/EVE.conf"));
    }

    #[test]
    fn a_real_install_refuses_an_exe_the_user_can_rewrite() {
        // The unit's ExecStart runs this as root and the polkit rule lets the
        // user start it without a password, so a user-writable binary would be
        // passwordless root.
        let dir = temp_dir("trust-install");
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, GOOD_CONF).unwrap();
        let exe = dir.join("yutani");
        std::fs::write(&exe, "#!/bin/true\n").unwrap();
        let e = install_root(&conf, 1000, "daniel", exe.to_str().unwrap(), false).unwrap_err().to_string();
        assert!(e.contains("refusing"), "got {e}");
        assert!(e.contains("sudo install -o root -g root -m 0755"), "got {e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dry_run_reports_the_trust_check_and_still_prints_the_rest() {
        let dir = temp_dir("trust-dry");
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, GOOD_CONF).unwrap();
        let exe = dir.join("yutani");
        std::fs::write(&exe, "#!/bin/true\n").unwrap();
        let r = install_root(&conf, 1000, "daniel", exe.to_str().unwrap(), true).unwrap();
        assert!(r.contains("refusing"), "the dry run must report the refusal: {r}");
        assert!(r.contains("sudo install -o root -g root -m 0755"));
        assert!(r.contains("ExecStart="), "the dry run must still print the rest: {r}");
        assert!(r.contains(POLKIT_PATH));
        assert!(!r.contains("U0VDUkVU"), "the key must never be printed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_root_side_refuses_a_user_name_that_is_not_the_uids_own() {
        assert!(check_username(1000, "daniel", Some("daniel")).is_ok());
        let e = check_username(1000, "root", Some("daniel")).unwrap_err().to_string();
        assert!(e.contains("uid 1000"), "got {e}");
        assert!(e.contains("daniel") && e.contains("root"), "got {e}");
        let e = check_username(4242, "daniel", None).unwrap_err().to_string();
        assert!(e.contains("4242"), "got {e}");
    }

    #[test]
    fn a_directory_that_is_not_root_owned_is_refused_rather_than_used() {
        let dir = temp_dir("etc-yutani");
        let e = ensure_dir(&dir, 0o700).unwrap_err().to_string();
        assert!(e.contains("refusing"), "got {e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_freshly_created_dir_gets_its_mode_whatever_the_umask() {
        let base = temp_dir("etc-mode");
        // The parent is user-owned, so only the create path can be exercised
        // here: make the new directory a child that does not exist yet and
        // check the mode it is created with, not the trust check.
        let dir = base.join("yutani");
        let _lock = crate::tunnel::testing::mode_lock();
        // SAFETY: umask has no preconditions and cannot fail; restored below.
        let old = unsafe { crate::tunnel::testing::libc_umask(0o077) };
        let created = create_dir_with_mode(&dir, 0o755);
        // SAFETY: as above.
        unsafe { crate::tunnel::testing::libc_umask(old) };
        created.unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o7777, 0o755);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
