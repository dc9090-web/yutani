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
/// and the kernel logs nothing). The `killswitch` chain would likewise drop
/// it, since it leaves via the LAN interface and not `yutani0`. Marking the
/// outer packet distinctly lets both chains recognise and exempt it — this
/// is the same reason wg-quick sets a firewall mark on its interfaces.
pub const WG_FWMARK: u32 = 0x5a;
pub const TABLE: u32 = 51820;
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
