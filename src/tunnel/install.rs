//! `yutani tunnel install|uninstall`: one-time privileged setup through
//! `pkexec`, and the root-side `install-root|uninstall-root` it invokes.

use anyhow::{Context as _, anyhow, bail, ensure};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
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

/// The path `exe` will actually run as, once every symlink and `.`/`..`
/// component is resolved. `install_root` must check *and* write this same
/// value: checking the canonicalised path but writing the raw `--exe`
/// string into `ExecStart=` would let a symlink component the user
/// controls pass the trust check while systemd execs whatever the link
/// points to. Errors when `exe` cannot be resolved (missing, dangling
/// symlink, etc.) — the caller treats that the same as a failed trust
/// check.
fn resolved_exe(exe: &str) -> anyhow::Result<PathBuf> {
    Path::new(exe).canonicalize().with_context(|| format!("cannot resolve {exe}"))
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

/// A comma-separated `--dns-servers`/`--dns-domains` value.
fn joined<T: ToString>(items: &[T]) -> String {
    items.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

/// User side: `pkexec <self> tunnel install-root …` (one password prompt).
/// `dns_servers`/`dns_domains` come from the user's validated `Config`; the
/// root side re-validates them and stores them in the conf it owns, because
/// root must never read `config.ron` itself (spec §9).
pub fn install(conf: &Path, dns_servers: &[std::net::Ipv4Addr], dns_domains: &[String]) -> anyhow::Result<()> {
    let conf = conf.canonicalize().with_context(|| format!("{}", conf.display()))?;
    let exe = current_exe()?;
    // Check before the prompt, not after: the root side refuses this anyway,
    // and there is no point spending the user's password on it.
    if let Err(why) = trusted_path(Path::new(&exe)) {
        bail!("{}", untrusted_exe_message(&exe, &conf, &why));
    }
    let (uid, user) = whoami()?;
    let mut cmd = Command::new("pkexec");
    cmd.args([&exe, "tunnel", "install-root", "--conf"])
        .arg(&conf)
        .args(["--uid", &uid.to_string(), "--user", &user, "--exe", &exe]);
    // Omitted rather than passed empty: `--dns-servers ""` is a parse error,
    // and an absent flag is exactly what "use the defaults" means on the
    // root side. `Config::validate` never produces an empty list anyway.
    if !dns_servers.is_empty() {
        cmd.args(["--dns-servers", &joined(dns_servers)]);
    }
    if !dns_domains.is_empty() {
        cmd.args(["--dns-domains", &joined(dns_domains)]);
    }
    let status = cmd.status().context("pkexec")?;
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
    // `dir` does not exist yet, so `trusted_path(dir)` can't be run on it —
    // but creating it under an untrusted parent is exactly as dangerous as
    // using an untrusted existing directory: whoever controls the parent
    // controls what ends up at `dir` too (a pre-placed symlink, a race
    // against the `create_dir_all` below, ...).
    if let Some(parent) = dir.parent() {
        trusted_path(parent)
            .map_err(|why| anyhow!("refusing to create {} under untrusted {}: {why}", dir.display(), parent.display()))?;
    }
    create_dir_with_mode(dir, mode)
}

/// Write `text` at `path` with `mode`, creating (or verifying) its
/// directory. Returns whether anything changed: a file that already holds
/// exactly `text` at exactly `mode` is left alone, so a repeated install of
/// the same conf is a no-op an installer (Ansible's `changed_when`) can
/// tell apart from a real one.
fn write(path: &str, text: &str, mode: u32, dir_mode: u32) -> anyhow::Result<bool> {
    if let Some(dir) = Path::new(path).parent() {
        ensure_dir(dir, dir_mode)?;
    }
    if already_written(path, text, mode) {
        return Ok(false);
    }
    write_with_mode(path, text, mode).map(|()| true)
}

/// Whether `path` already holds exactly `text` at exactly `mode`. Any
/// doubt (unreadable, a directory, a symlink) is "no", and the write goes
/// ahead as it always did.
fn already_written(path: &str, text: &str, mode: u32) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    if !meta.is_file() || meta.permissions().mode() & 0o7777 != mode {
        return false;
    }
    std::fs::read(path).is_ok_and(|bytes| bytes == text.as_bytes())
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

/// The last line of an `install_root` report when every file it would
/// have written already held exactly that (a re-run with the same conf and
/// the same DNS settings). `deploy/ansible` keys its `changed` flag on it.
pub const NOTHING_CHANGED_MARKER: &str = "# nothing changed: the same tunnel was already installed";

/// Marks a failed check in a dry-run report (see `install_root`'s `refuse`
/// closure). Shared with [`report_has_failure`] so the two can never drift.
const CHECK_FAILED_MARKER: &str = "# check FAILED — the real install would stop here:";

/// Whether a dry-run report (from [`install_root`]) recorded a check that
/// would make the real install refuse. Lets a caller that only sees the
/// printed text (e.g. `main`'s `--dry-run` arm) exit non-zero without
/// re-running any of the checks itself.
pub fn report_has_failure(report: &str) -> bool {
    report.contains(CHECK_FAILED_MARKER)
}

/// The DNS settings as the two conf lines the worker parses, with the
/// defaults filled in for an empty list (an `install-root` run by hand, or
/// a `config.ron` whose lists validation emptied). Every domain is checked
/// here, on the root side: these strings arrive from a user-side process
/// and are about to be written into a file only root may write, where a
/// newline would forge a second `# yutani: …` line.
fn dns_conf_lines(servers: &[std::net::Ipv4Addr], domains: &[String]) -> anyhow::Result<String> {
    for d in domains {
        ensure!(
            crate::model::config::valid_dns_domain(d),
            "{d:?} is not a plain DNS host name (letters, digits, `-`, `.`)"
        );
    }
    // Every packet to these addresses on port 53 is marked for the tunnel,
    // whoever sends it, so one the exit node cannot reach (the LAN router,
    // a Pi-hole, loopback) would take the whole machine's DNS down for as
    // long as the tunnel is up. Refused here, root-side, whatever the user
    // side let through.
    for s in servers {
        ensure!(super::usable_dns_server(s), "dns server {s} {}", super::UNUSABLE_DNS_SERVER);
    }
    let servers: Vec<String> = if servers.is_empty() {
        super::DEFAULT_DNS_SERVERS.iter().map(|s| s.to_string()).collect()
    } else {
        servers.iter().map(|s| s.to_string()).collect()
    };
    let domains: Vec<String> = if domains.is_empty() {
        super::DEFAULT_DNS_DOMAINS.iter().map(|d| (*d).to_string()).collect()
    } else {
        domains.to_vec()
    };
    Ok(format!("# yutani: dns_servers = {}\n# yutani: dns_domains = {}\n", servers.join(" "), domains.join(" ")))
}

/// The most `install-root` will read from `--conf`. A WireGuard conf is a
/// few hundred bytes; 64 KiB is a generous ceiling that still keeps a root
/// process from reading something unbounded.
const CONF_MAX_LEN: u64 = 64 * 1024;

/// Whether root may read `--conf` at all, decided on what the path *is*
/// before a byte of it is read: a regular file (not a symlink, which
/// `install` would have resolved; not a FIFO, which would hang the
/// pkexec'd root process forever; not a device — `/dev/zero` is unbounded
/// memory), no larger than [`CONF_MAX_LEN`], and owned by the user it is
/// being installed for, by root, or by `self` — the uid running the check,
/// which is root on the real install and `uid` on a dry run, so that last
/// case changes nothing outside the test-suite, where `uid` is a fixture.
/// Spec §9 calls the conf the one deliberate crossing of the root/user
/// boundary; this is its guard.
fn conf_guard(conf: &Path, uid: u32) -> anyhow::Result<()> {
    let md = std::fs::symlink_metadata(conf).with_context(|| format!("stat {}", conf.display()))?;
    ensure!(!md.file_type().is_symlink(), "{} is a symlink; pass the file it points to", conf.display());
    conf_readable(md.is_file(), md.uid(), md.len(), uid, crate::ipc::uid()).map_err(|why| anyhow!("{}: {why}", conf.display()))
}

/// The verdict behind [`conf_guard`], on the facts alone.
fn conf_readable(regular: bool, owner: u32, len: u64, uid: u32, this: u32) -> Result<(), String> {
    if !regular {
        return Err("not a regular file".to_string());
    }
    if len > CONF_MAX_LEN {
        return Err(format!("{len} bytes is larger than a WireGuard conf can be (at most {} KiB)", CONF_MAX_LEN / 1024));
    }
    if owner != uid && owner != 0 && owner != this {
        return Err(format!("owned by uid {owner}, not by uid {uid} or root"));
    }
    Ok(())
}

/// Root side. With `dry_run`, returns what would be written instead of writing.
pub fn install_root(
    conf: &Path,
    uid: u32,
    username: &str,
    exe: &str,
    dns_servers: &[std::net::Ipv4Addr],
    dns_domains: &[String],
    dry_run: bool,
) -> anyhow::Result<String> {
    ensure!(conf.is_absolute() && Path::new(exe).is_absolute(), "paths must be absolute");
    ensure!(valid_username(username), "{username:?} is not a POSIX portable user name ([a-z_][a-z0-9_-]*$)");
    check_pkexec_uid(std::env::var("PKEXEC_UID").ok().as_deref(), uid)?;
    conf_guard(conf, uid)?;
    let text = std::fs::read_to_string(conf).with_context(|| format!("read {}", conf.display()))?;
    let label = conf.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let parsed = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf.display()))?;
    let dns_lines = dns_conf_lines(dns_servers, dns_domains)?;
    // Root's own lines first, and nothing derived from a user-chosen string
    // among them: the worker takes the first `# yutani: uid`/`dns_*` line it
    // finds, and a file name (which may contain a newline) used to be
    // written ahead of these as a label nobody read.
    let stored = format!("# yutani: uid = {uid}\n{dns_lines}{text}");
    // Resolve once, up front: `unit_text` and the trust check must agree on
    // the same path (see `resolved_exe`). When resolution itself fails, fall
    // back to the raw `--exe` string for display purposes only — the check
    // below still refuses the install (or reports the refusal, in a dry run).
    let resolved = resolved_exe(exe);
    let exe_for_unit = match &resolved {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => exe.to_string(),
    };
    let unit = unit_text(&exe_for_unit);
    let rule = polkit_text(username);
    // Both checks are hard errors for a real install and *reported* by a dry
    // run, which is unprivileged and exists to tell the user what is wrong.
    let mut checks = String::new();
    let refuse = |msg: String, checks: &mut String| -> anyhow::Result<()> {
        if !dry_run {
            bail!("{msg}");
        }
        checks.push_str(&format!("{CHECK_FAILED_MARKER}\n{msg}\n\n"));
        Ok(())
    };
    match &resolved {
        Ok(p) => match trusted_path(p) {
            Ok(()) => checks.push_str(&format!("# {} is root-owned and not writable by others: OK\n\n", p.display())),
            Err(why) => refuse(untrusted_exe_message(&p.to_string_lossy(), conf, why.as_str()), &mut checks)?,
        },
        Err(e) => refuse(format!("cannot resolve {exe}: {e:#}"), &mut checks)?,
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
    // All three are written before the result is looked at: a partial
    // earlier install (say, a conf without its unit) is completed, not
    // skipped.
    let changed = [
        write(CONF_PATH, &stored, 0o600, 0o700)?,
        write(UNIT_PATH, &unit, 0o644, 0o755)?,
        write(POLKIT_PATH, &rule, 0o644, 0o755)?,
    ]
    .contains(&true);
    daemon_reload()?;
    if parsed.dns.is_none() {
        eprintln!("warning: the conf has no DNS entry; EVE's DNS lookups will not go through the tunnel");
    }
    Ok(if changed { report } else { format!("{report}{NOTHING_CHANGED_MARKER}\n") })
}

/// `systemctl daemon-reload`, and whether it worked. systemd keeps a unit
/// it has loaded until told to re-read: a reload that fails after the unit
/// was written leaves the old definition running, and one that fails after
/// it was deleted leaves `systemctl start` working from the cached
/// definition — without the polkit rule, so with a password prompt — while
/// `installed()` says false. So it is an error on both sides.
fn daemon_reload() -> anyhow::Result<()> {
    reload_outcome(Command::new("systemctl").arg("daemon-reload").status())
}

fn reload_outcome(status: std::io::Result<std::process::ExitStatus>) -> anyhow::Result<()> {
    let st = status.context("systemctl daemon-reload")?;
    ensure!(st.success(), "systemctl daemon-reload failed ({st})");
    Ok(())
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
    daemon_reload()?;
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

    fn servers() -> Vec<std::net::Ipv4Addr> {
        crate::tunnel::DEFAULT_DNS_SERVERS.to_vec()
    }

    fn domains() -> Vec<String> {
        crate::tunnel::DEFAULT_DNS_DOMAINS.iter().map(|d| (*d).to_string()).collect()
    }

    /// The user's `config.ron` is never read by root (spec §9), so the DNS
    /// settings make the trip inside the conf root owns — as comment lines
    /// next to `# yutani: uid`, in the shape `worker::dns_*_from_conf`
    /// parses.
    #[test]
    fn the_stored_conf_carries_the_dns_settings_for_the_worker() {
        let dir = std::env::temp_dir().join(format!("yutani-inst-dns-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", &servers(), &domains(), true).unwrap();
        assert!(report.contains("# yutani: dns_servers = 1.1.1.1 9.9.9.9"), "{report}");
        assert!(report.contains("# yutani: dns_domains = eveonline.com ccpgames.com evetech.net"), "{report}");

        // Empty lists (an `install-root` run by hand) get the defaults, not
        // a conf that says "no DNS in the tunnel".
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", &[], &[], true).unwrap();
        assert!(report.contains("# yutani: dns_servers = 1.1.1.1 9.9.9.9"), "{report}");
        assert!(report.contains("# yutani: dns_domains = eveonline.com ccpgames.com evetech.net"), "{report}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// These arrive as `--dns-domains` from a user-side process and end up
    /// in a root-owned file: a value with a newline in it would forge a
    /// second `# yutani: …` line (`# yutani: uid = 0`), so the root side
    /// refuses anything that is not a plain host name instead of trusting
    /// the user side to have validated it.
    #[test]
    fn install_root_refuses_dns_domains_that_could_forge_a_conf_line() {
        let dir = std::env::temp_dir().join(format!("yutani-inst-dnsbad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let forged = vec!["eveonline.com\n# yutani: uid = 0".to_string()];
        let e = install_root(&conf, 1000, "daniel", "/opt/y", &servers(), &forged, true).unwrap_err().to_string();
        assert!(e.contains("host name"), "expected a clear message, got {e}");
        let spaced = vec!["eve online.com".to_string()];
        assert!(install_root(&conf, 1000, "daniel", "/opt/y", &servers(), &spaced, true).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The conf's file name is the user's to choose and a Linux file name
    /// may contain a newline. The label used to be written first, ahead of
    /// the `uid`/`dns_servers` lines, and the worker takes the first match
    /// — so `x\n# yutani: uid = 0\n# yutani: dns_servers = 10.0.0.1.conf`
    /// installed a worker routing for uid 0. Nothing reads the label line
    /// (the worker re-derives it from the `[Peer]` comment), so it is gone:
    /// root's own lines come first, and the first `uid` line is root's.
    #[test]
    fn the_confs_file_name_cannot_forge_the_lines_root_writes() {
        let dir = temp_dir("inst-forge");
        let conf = dir.join("x\n# yutani: uid = 0\n# yutani: dns_servers = 10.0.0.1.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", &servers(), &domains(), true).unwrap();
        assert!(!report.contains("# yutani: label"), "the label line is dead and must not be written: {report}");
        let first = |key: &str| {
            let prefix = format!("# yutani: {key} =");
            report.lines().find_map(|l| l.trim().strip_prefix(prefix.as_str())).map(str::trim).map(str::to_string)
        };
        assert_eq!(first("uid").as_deref(), Some("1000"), "the first uid line must be root's: {report}");
        assert_eq!(first("dns_servers").as_deref(), Some("1.1.1.1 9.9.9.9"), "{report}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A LAN or loopback resolver in `--dns-servers` would have the mark
    /// rule send every query on the machine to it into the tunnel, where
    /// the exit node cannot reach it: machine-wide DNS dies while
    /// connected, with the applet showing a healthy tunnel. The root side
    /// refuses it, naming the address, whatever the user side validated.
    #[test]
    fn install_root_refuses_dns_servers_the_exit_node_cannot_reach() {
        let dir = temp_dir("inst-lan-dns");
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, GOOD_CONF).unwrap();
        for bad in ["192.168.1.1", "10.0.0.1", "127.0.0.53", "169.254.1.1", "0.0.0.0"] {
            let servers = vec![bad.parse().unwrap(), "8.8.8.8".parse().unwrap()];
            let e = install_root(&conf, 1000, "daniel", "/opt/y", &servers, &domains(), true).unwrap_err().to_string();
            assert!(e.contains(bad), "the message must name the address, got {e}");
            assert!(e.contains("exit node"), "got {e}");
        }
        assert!(install_root(&conf, 1000, "daniel", "/opt/y", &["8.8.8.8".parse().unwrap()], &domains(), true).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Root reads whatever `--conf` names. A symlink, a device (`/dev/zero`
    /// would be unbounded memory in a root process; a FIFO would hang it
    /// forever) or an oversized file is refused before the read, by what
    /// the path *is*, never by what it holds. `install` canonicalises the
    /// path and a WireGuard conf is a few hundred bytes, so none of this
    /// touches a real install.
    #[test]
    fn install_root_reads_only_a_small_regular_file_that_is_not_a_symlink() {
        let dir = temp_dir("inst-guard");
        let real = dir.join("EVE.conf");
        std::fs::write(&real, GOOD_CONF).unwrap();
        let link = dir.join("link.conf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let e = install_root(&link, 1000, "daniel", "/opt/y", &servers(), &domains(), true).unwrap_err().to_string();
        assert!(e.contains("symlink"), "got {e}");
        let e = install_root(Path::new("/dev/null"), 1000, "daniel", "/opt/y", &servers(), &domains(), true).unwrap_err().to_string();
        assert!(e.contains("regular file"), "got {e}");
        let big = dir.join("big.conf");
        std::fs::write(&big, format!("{GOOD_CONF}# {}\n", "x".repeat(CONF_MAX_LEN as usize))).unwrap();
        let e = install_root(&big, 1000, "daniel", "/opt/y", &servers(), &domains(), true).unwrap_err().to_string();
        assert!(e.contains("KiB"), "got {e}");
        assert!(install_root(&real, 1000, "daniel", "/opt/y", &servers(), &domains(), true).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The owner rule: the user's own file, or root's; a file some third
    /// user owns is not root's to read on this user's behalf. `this` is
    /// the uid running the check, root or `uid` outside the tests.
    #[test]
    fn the_conf_must_be_a_small_regular_file_of_the_user_or_root() {
        assert!(conf_readable(true, 1000, 300, 1000, 1000).is_ok());
        assert!(conf_readable(true, 0, 300, 1000, 0).is_ok(), "an admin may stage the conf as root");
        assert!(conf_readable(true, 1000, 300, 1000, 0).is_ok(), "the real install: root checking the user's file");
        let e = conf_readable(true, 1001, 300, 1000, 0).unwrap_err();
        assert!(e.contains("uid 1001") && e.contains("uid 1000"), "got {e}");
        assert!(conf_readable(false, 1000, 0, 1000, 0).unwrap_err().contains("regular file"));
        assert!(conf_readable(true, 1000, CONF_MAX_LEN, 1000, 0).is_ok(), "exactly the ceiling is fine");
        assert!(conf_readable(true, 1000, CONF_MAX_LEN + 1, 1000, 0).unwrap_err().contains("KiB"));
    }

    #[test]
    fn install_root_dry_run_writes_nothing_and_hides_the_key() {
        let existed = Path::new(CONF_PATH).exists();
        let dir = std::env::temp_dir().join(format!("yutani-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("EVE-UK-455.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\n# UK#455\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", &servers(), &domains(), true).unwrap();
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
        assert!(install_root(Path::new("rel.conf"), 1000, "d", "/opt/y", &servers(), &domains(), true).unwrap_err().to_string().contains("absolute"));
        let dir = std::env::temp_dir().join(format!("yutani-inst-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("bad.conf");
        std::fs::write(&conf, "[Interface]\nAddress = 10.2.0.2/32\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        assert!(install_root(&conf, 1000, "d", "/opt/y", &servers(), &domains(), true).unwrap_err().to_string().contains("PrivateKey"));
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
        let report = install_root(&conf, 1000, "daniel", "/opt/yutani/yutani", &servers(), &domains(), true).unwrap();
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
        let e = install_root(&conf, 1000, "a\"b", "/opt/y", &servers(), &domains(), true).unwrap_err().to_string();
        assert!(e.contains("user name"), "expected a clear message, got {e}");
        assert!(install_root(&conf, 1000, "daniel_2-x", "/opt/y", &servers(), &domains(), true).is_ok());
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

    /// `systemctl daemon-reload` is what makes systemd forget a deleted
    /// unit (and see a new one). Its failure is an error on both sides, as
    /// it always was for install: swallowed on uninstall, systemd kept the
    /// deleted unit loaded, so `systemctl start` still worked from the
    /// cached definition — without the polkit rule, so with a password
    /// prompt — while `installed()` said false.
    #[test]
    fn a_failed_daemon_reload_is_an_error_not_a_shrug() {
        assert!(reload_outcome(Command::new("true").status()).is_ok());
        let e = reload_outcome(Command::new("false").status()).unwrap_err().to_string();
        assert!(e.contains("daemon-reload"), "got {e}");
        let e = reload_outcome(Command::new("/nonexistent-yutani-systemctl").status()).unwrap_err().to_string();
        assert!(e.contains("daemon-reload"), "got {e}");
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
        let e = install_root(&conf, 1000, "daniel", exe.to_str().unwrap(), &servers(), &domains(), false).unwrap_err().to_string();
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
        let r = install_root(&conf, 1000, "daniel", exe.to_str().unwrap(), &servers(), &domains(), true).unwrap();
        assert!(r.contains("refusing"), "the dry run must report the refusal: {r}");
        assert!(r.contains("sudo install -o root -g root -m 0755"));
        assert!(r.contains("ExecStart="), "the dry run must still print the rest: {r}");
        assert!(r.contains(POLKIT_PATH));
        assert!(!r.contains("U0VDUkVU"), "the key must never be printed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolved_exe_returns_the_canonical_path_for_a_real_file() {
        let dir = temp_dir("resolve-plain");
        let exe = dir.join("yutani");
        std::fs::write(&exe, "#!/bin/true\n").unwrap();
        let resolved = resolved_exe(exe.to_str().unwrap()).unwrap();
        assert_eq!(resolved, exe.canonicalize().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolved_exe_follows_a_symlink_to_its_target() {
        let dir = temp_dir("resolve-symlink");
        let target = dir.join("real-yutani");
        std::fs::write(&target, "#!/bin/true\n").unwrap();
        let link = dir.join("yutani");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let resolved = resolved_exe(link.to_str().unwrap()).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
        assert_ne!(resolved, link, "must resolve past the symlink, not just accept it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolved_exe_errors_on_a_path_that_does_not_exist() {
        let e = resolved_exe("/nonexistent-yutani-path").unwrap_err().to_string();
        assert!(e.contains("cannot resolve"), "got {e}");
    }

    #[test]
    fn install_root_writes_the_resolved_path_not_the_symlink_it_was_given() {
        // The trust check canonicalises `exe`; if `unit_text` were built from
        // the raw `--exe` string instead, a symlink under the user's control
        // could pass the check while systemd execs a different, redirectable
        // target.
        let dir = temp_dir("resolve-unit");
        let conf = dir.join("EVE.conf");
        std::fs::write(&conf, GOOD_CONF).unwrap();
        let target = dir.join("real-yutani");
        std::fs::write(&target, "#!/bin/true\n").unwrap();
        let link = dir.join("yutani-link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let resolved_target = target.canonicalize().unwrap();
        // The temp dir is owned by the test user, so the real install would
        // still refuse this — but the dry run must report the *resolved*
        // path, not the symlink path it was handed.
        let r = install_root(&conf, 1000, "daniel", link.to_str().unwrap(), &servers(), &domains(), true).unwrap();
        assert!(r.contains("refusing"), "a user-owned exe must still be refused: {r}");
        assert!(
            r.contains(&format!("ExecStart={} tunnel run", resolved_target.display())),
            "ExecStart must show the resolved path: {r}"
        );
        assert!(!r.contains(&format!("ExecStart={}", link.display())), "must not write the symlink path into the unit: {r}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn report_has_failure_detects_the_check_failed_marker() {
        assert!(report_has_failure("some text\n# check FAILED — the real install would stop here:\nrefusing ...\n"));
        assert!(!report_has_failure("some text\n# /opt/y is root-owned and not writable by others: OK\n"));
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
    fn ensure_dir_refuses_to_create_a_directory_under_an_untrusted_parent() {
        // `dir` itself does not exist, so only the create path is exercised:
        // a user-owned parent must not be allowed to dictate what a
        // freshly-created directory becomes.
        let parent = temp_dir("ensure-dir-parent");
        let dir = parent.join("etc-yutani-child");
        assert!(!dir.exists());
        let e = ensure_dir(&dir, 0o700).unwrap_err().to_string();
        assert!(e.contains("refusing"), "got {e}");
        assert!(!dir.exists(), "must not create the directory when the parent is untrusted");
        std::fs::remove_dir_all(&parent).unwrap();
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

    #[test]
    fn a_file_already_holding_the_text_at_the_mode_is_not_rewritten() {
        let base = std::env::temp_dir().join(format!("yutani-install-unchanged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("tunnel.conf");
        let path = path.to_str().unwrap();
        // Nothing there yet: a write is due.
        assert!(!already_written(path, "[Interface]\n", 0o600));
        write_with_mode(path, "[Interface]\n", 0o600).unwrap();
        assert!(already_written(path, "[Interface]\n", 0o600));
        // Different text, or the same text at a different mode: due again.
        assert!(!already_written(path, "[Interface]\nDNS = 1.1.1.1\n", 0o600));
        assert!(!already_written(path, "[Interface]\n", 0o644));
        // A symlink to the right content is not the file root wrote.
        let link = base.join("link.conf");
        std::os::unix::fs::symlink(path, &link).unwrap();
        assert!(!already_written(link.to_str().unwrap(), "[Interface]\n", 0o600));
        // The marker only ever ends a report; the dry run never claims it.
        assert!(!NOTHING_CHANGED_MARKER.contains(CHECK_FAILED_MARKER));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
