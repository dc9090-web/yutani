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

/// The unit text. `exe` is the absolute daemon path.
pub fn unit_text(exe: &str) -> String {
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

pub fn installed() -> bool {
    unit_path().is_file()
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

/// Write the unit for `exe` and reload the user manager. Idempotent.
pub fn install(exe: &Path) -> anyhow::Result<PathBuf> {
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

    #[test]
    fn the_unit_restarts_on_failure_only_and_lives_under_the_user_config() {
        let text = unit_text("/usr/local/bin/yutani");
        for line in [
            "ExecStart=/usr/local/bin/yutani",
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
}
