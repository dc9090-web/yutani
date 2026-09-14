//! A systemd *user* unit for the daemon, so a crash brings it straight back.
//!
//! The daemon runs on libcosmic, which today has a use-after-free in its
//! surface teardown (see `docs/superpowers/specs/2026-09-11-yutani-design.md`
//! and the crash notes in the project memory); until that is fixed
//! upstream, `Restart=on-failure` turns a crash into a one-second blink. A
//! deliberate `yutani quit` exits 0 and stays down.
//!
//! `yutani service install` writes the unit; the Applications launcher runs
//! `yutani start`, which goes through the unit when it is installed and
//! runs the daemon in-process otherwise.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

pub const UNIT_NAME: &str = "yutani.service";

/// `~/.config/systemd/user/yutani.service`.
pub fn unit_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("systemd").join("user").join(UNIT_NAME)
}

pub fn unit_path() -> PathBuf {
    unit_path_in(&dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")))
}

/// The unit text. `exe` is the absolute daemon path, quoted for
/// `ExecStart` (see `quote_exec`).
pub fn unit_text(exe: &str) -> String {
    let exe = quote_exec(exe);
    format!(
        "[Unit]\n\
         Description=Yutani - EVE Online thumbnails, hotkeys and tunnel\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         # A crash loop (ten failures in a minute) stops rather than spinning.\n\
         StartLimitIntervalSec=60\n\
         StartLimitBurst=10\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe}\n\
         # `yutani quit` exits 0 and stays down; a crash is back in a second.\n\
         Restart=on-failure\n\
         RestartSec=1\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n"
    )
}

/// `path` as one `ExecStart` argument. Unquoted, systemd splits it on
/// whitespace (`~/My Projects/yutani` becomes two words) and reads `%x`
/// as a specifier; inside double quotes `\` and `"` need a backslash,
/// and `%` still needs doubling.
fn quote_exec(path: &str) -> String {
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for c in path.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// The unit the `yutani` package ships (`packaging/yutani.service`,
/// `ExecStart=/usr/bin/yutani`). With it in place `service install` has
/// nothing to write and `yutani start` goes through systemd as it would
/// with the per-user unit.
pub const PACKAGED_UNIT: &str = "/usr/lib/systemd/user/yutani.service";

pub fn installed() -> bool {
    unit_path().is_file() || Path::new(PACKAGED_UNIT).is_file()
}

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .with_context(|| format!("cannot run systemctl --user {}", args.join(" ")))?;
    anyhow::ensure!(status.success(), "systemctl --user {} failed ({status})", args.join(" "));
    Ok(())
}

/// Write the unit for `exe` and reload the user manager. Idempotent. With
/// the packaged unit in place nothing is written: a per-user copy would
/// shadow it with the same text (or, for a source build, a different
/// `ExecStart` than the package's), so the packaged path is returned.
pub fn install(exe: &Path) -> anyhow::Result<PathBuf> {
    if Path::new(PACKAGED_UNIT).is_file() {
        systemctl(&["daemon-reload"])?;
        return Ok(PathBuf::from(PACKAGED_UNIT));
    }
    let path = unit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::model::write_atomic(&path, &unit_text(&exe.to_string_lossy()))?;
    systemctl(&["daemon-reload"])?;
    Ok(path)
}

/// Stop the unit if it runs, remove it, reload. A missing unit is fine.
pub fn uninstall() -> anyhow::Result<bool> {
    let path = unit_path();
    if !path.exists() {
        return Ok(false);
    }
    // Best effort: the unit may never have been started.
    let _ = Command::new("systemctl").args(["--user", "stop", UNIT_NAME]).status();
    std::fs::remove_file(&path)?;
    systemctl(&["daemon-reload"])?;
    Ok(true)
}

/// `yutani start` with the unit installed: hand off to systemd.
pub fn start_unit() -> anyhow::Result<()> {
    systemctl(&["start", UNIT_NAME])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unit the package ships is byte-for-byte what `service install`
    /// would write for `/usr/bin/yutani`, so the two never drift.
    #[test]
    fn the_packaged_unit_is_what_service_install_writes_for_usr_bin() {
        assert_eq!(include_str!("../packaging/yutani.service"), unit_text("/usr/bin/yutani"));
        assert_eq!(PACKAGED_UNIT, "/usr/lib/systemd/user/yutani.service");
    }

    #[test]
    fn the_unit_restarts_on_failure_only_and_lives_under_the_user_config() {
        let text = unit_text("/usr/local/bin/yutani");
        for line in [
            "ExecStart=\"/usr/local/bin/yutani\"",
            "Restart=on-failure",
            "RestartSec=1",
            "StartLimitIntervalSec=60",
            "StartLimitBurst=10",
            "PartOf=graphical-session.target",
            "WantedBy=graphical-session.target",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
        }
        // The start limit is a [Unit] setting; in [Service] systemd ignores it.
        let unit = text.split("[Service]").next().unwrap();
        assert!(unit.contains("StartLimitBurst"));
        assert_eq!(unit_path_in(Path::new("/home/d/.config")), PathBuf::from("/home/d/.config/systemd/user/yutani.service"));
    }

    /// [M4] A checkout under `~/My Projects/` — or any path with a `%`,
    /// `"` or `\` in it — has to survive systemd's ExecStart parsing:
    /// quoted, with the three characters that mean something inside the
    /// quotes (and the `%` specifier) escaped.
    #[test]
    fn the_exec_path_is_quoted_and_escaped_for_systemd() {
        let text = unit_text("/home/d/My Projects/yutani");
        assert!(text.lines().any(|l| l == "ExecStart=\"/home/d/My Projects/yutani\""), "{text}");
        let text = unit_text(r#"/p/a"b\c%d"#);
        assert!(text.lines().any(|l| l == r#"ExecStart="/p/a\"b\\c%%d""#), "{text}");
    }
}
