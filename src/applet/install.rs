//! `yutani applet install|uninstall`: the icon-theme copies and the
//! `.desktop` file cosmic-panel needs to list the applet (spec §3).
//! Everything is under `~/.local/share`, so no privileges are involved.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::assets::ICONS;

/// Must match `Applet::APP_ID` in the applet binary — cosmic-panel keys
/// applets on the desktop-entry id.
pub const DESKTOP_ID: &str = "com.yutani.Applet.desktop";

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Where symbolic app icons go for the current user.
pub fn icon_dir() -> PathBuf {
    data_dir().join("icons").join("hicolor").join("symbolic").join("apps")
}

pub fn desktop_path() -> PathBuf {
    data_dir().join("applications").join(DESKTOP_ID)
}

/// The applet binary: the one next to the running `yutani` when there is
/// one (cargo target dir, or a prefix bin dir), else the bare name so a
/// `PATH` lookup decides.
pub fn applet_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("yutani-applet")))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| PathBuf::from("yutani-applet"))
}

/// The desktop entry, shaped like COSMIC's own applets
/// (`/usr/share/applications/com.system76.CosmicApplet*.desktop`).
pub fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Name=Yutani\n\
         Comment=EVE Online clients and the WireGuard tunnel\n\
         Type=Application\n\
         Exec={exec}\n\
         Terminal=false\n\
         Categories=COSMIC;\n\
         Keywords=COSMIC;Applet;EVE;WireGuard;VPN;Yutani;\n\
         Icon=y-symbolic\n\
         StartupNotify=true\n\
         NoDisplay=true\n\
         X-CosmicApplet=true\n\
         X-OverflowPriority=50\n"
    )
}

/// Write the five icons and the desktop file; returns how many files were
/// written. Overwrites, so running it twice is a no-op with the same count.
pub fn install_to(icon_dir: &Path, desktop: &Path, exec: &str) -> anyhow::Result<usize> {
    std::fs::create_dir_all(icon_dir).with_context(|| format!("create {}", icon_dir.display()))?;
    for (name, bytes) in ICONS {
        let path = icon_dir.join(name);
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    }
    if let Some(parent) = desktop.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(desktop, desktop_entry(exec))
        .with_context(|| format!("write {}", desktop.display()))?;
    Ok(ICONS.len() + 1)
}

/// Remove exactly the files `install_to` wrote; returns how many existed.
pub fn uninstall_from(icon_dir: &Path, desktop: &Path) -> anyhow::Result<usize> {
    let mut removed = 0;
    for (name, _) in ICONS {
        let path = icon_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }
    match std::fs::remove_file(desktop) {
        Ok(()) => removed += 1,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("remove {}", desktop.display())),
    }
    Ok(removed)
}

/// Refresh the icon cache if the tool is there; never fatal.
fn update_icon_cache(icon_dir: &Path) {
    let theme_root = icon_dir.ancestors().nth(2); // …/icons/hicolor
    if let Some(root) = theme_root {
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .arg("-q")
            .arg("-t")
            .arg("-f")
            .arg(root)
            .status();
    }
}

pub fn install() -> anyhow::Result<()> {
    let icons = icon_dir();
    let desktop = desktop_path();
    let exe = applet_exe();
    let count = install_to(&icons, &desktop, &exe.to_string_lossy())?;
    update_icon_cache(&icons);
    println!("installed {count} files ({} and {})", icons.display(), desktop.display());
    println!("one manual step is left:");
    println!("  Settings → Desktop → Panel → Applets → add \"Yutani\"");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let icons = icon_dir();
    let desktop = desktop_path();
    let count = uninstall_from(&icons, &desktop)?;
    update_icon_cache(&icons);
    println!("removed {count} files");
    println!("remove the applet from the panel in Settings → Desktop → Panel → Applets");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("yutani-applet-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        (root.join("icons"), root.join("applications").join(DESKTOP_ID))
    }

    #[test]
    fn the_desktop_entry_is_what_cosmic_panel_looks_for() {
        let text = desktop_entry("/usr/local/bin/yutani-applet");
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.ends_with('\n'));
        for line in [
            "Name=Yutani",
            "Type=Application",
            "Exec=/usr/local/bin/yutani-applet",
            "Icon=y-symbolic",
            "Terminal=false",
            "Categories=COSMIC;",
            "NoDisplay=true",
            "X-CosmicApplet=true",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
        }
    }

    #[test]
    fn install_writes_every_icon_and_the_desktop_file_and_is_idempotent() {
        let (icons, desktop) = dirs("install");
        assert_eq!(install_to(&icons, &desktop, "/opt/yutani-applet").unwrap(), 6);
        for (name, bytes) in crate::assets::ICONS {
            assert_eq!(std::fs::read(icons.join(name)).unwrap(), bytes);
        }
        assert!(std::fs::read_to_string(&desktop).unwrap().contains("Exec=/opt/yutani-applet"));
        // Running it again rewrites the same six files, not more.
        assert_eq!(install_to(&icons, &desktop, "/opt/yutani-applet").unwrap(), 6);
        assert_eq!(std::fs::read_dir(&icons).unwrap().count(), 5);
    }

    #[test]
    fn uninstall_removes_exactly_our_files() {
        let (icons, desktop) = dirs("uninstall");
        install_to(&icons, &desktop, "/opt/yutani-applet").unwrap();
        std::fs::write(icons.join("someone-elses-symbolic.svg"), b"<svg/>").unwrap();
        assert_eq!(uninstall_from(&icons, &desktop).unwrap(), 6);
        assert!(!desktop.exists());
        assert!(icons.join("someone-elses-symbolic.svg").exists(), "foreign icons stay");
        assert_eq!(std::fs::read_dir(&icons).unwrap().count(), 1);
    }

    #[test]
    fn uninstall_on_a_clean_system_removes_nothing_and_does_not_fail() {
        let (icons, desktop) = dirs("clean");
        assert_eq!(uninstall_from(&icons, &desktop).unwrap(), 0);
    }

    #[test]
    fn the_real_paths_are_the_xdg_user_ones() {
        assert!(icon_dir().ends_with("icons/hicolor/symbolic/apps"));
        assert!(desktop_path().ends_with("applications/com.yutani.Applet.desktop"));
        assert_eq!(applet_exe().file_name().unwrap(), "yutani-applet");
    }
}
