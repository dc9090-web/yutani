//! `yutani applet install|uninstall`: the icon-theme copies and the two
//! `.desktop` files — the one cosmic-panel needs to list the applet
//! (spec §3), and the launcher entry that puts "Yutani" in Applications.
//! Everything is under `~/.local/share`, so no privileges are involved.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::assets::ICONS;

/// Must match `Applet::APP_ID` in the applet binary — cosmic-panel keys
/// applets on the desktop-entry id.
pub const DESKTOP_ID: &str = "com.yutani.Applet.desktop";

/// The launcher entry's id. Distinct from [`DESKTOP_ID`]: that one is
/// `NoDisplay=true` and only cosmic-panel reads it, this one is what the
/// Applications list shows.
pub const LAUNCHER_ID: &str = "com.yutani.Yutani.desktop";

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// The user's hicolor icon theme. Entries land in their own subdirectory of
/// it (`crate::assets::ICONS` says which), because symbolic marks and the
/// full-colour launcher icon are filed in different places.
pub fn theme_dir() -> PathBuf {
    data_dir().join("icons").join("hicolor")
}

pub fn applications_dir() -> PathBuf {
    data_dir().join("applications")
}

pub fn desktop_path() -> PathBuf {
    applications_dir().join(DESKTOP_ID)
}

pub fn launcher_path() -> PathBuf {
    applications_dir().join(LAUNCHER_ID)
}

/// Every file one install touches. A struct rather than more and more
/// parameters, so `install_to`/`uninstall_from` stay readable.
pub struct Paths {
    /// The hicolor theme root the `ICONS` go under.
    pub icons: PathBuf,
    /// The applet's `NoDisplay` entry, for cosmic-panel.
    pub applet: PathBuf,
    /// The launcher entry, for the Applications list.
    pub launcher: PathBuf,
}

impl Paths {
    /// The real XDG user paths.
    pub fn user() -> Self {
        Self { icons: theme_dir(), applet: desktop_path(), launcher: launcher_path() }
    }
}

/// The applet binary: the one next to the running `yutani` (cargo target
/// dir, or a prefix bin dir). cosmic-panel runs the `Exec=` line with its
/// own `PATH`, so a bare name would silently produce an applet that never
/// starts; a missing sibling is an error with the install command instead.
pub fn applet_exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot resolve the running yutani binary")?;
    let dir = exe.parent().context("the running yutani binary has no parent directory")?;
    let sibling = dir.join("yutani-applet");
    if sibling.is_file() {
        return Ok(sibling);
    }
    anyhow::bail!(
        "yutani-applet is not installed next to {}\n\
         the panel starts the applet from that directory, so install it there first:\n\
         \x20 sudo install -o root -g root -m 0755 target/release/yutani-applet {}/",
        exe.display(),
        dir.display()
    )
}

/// The daemon binary for the launcher's `Exec=`: the running `yutani`,
/// canonicalised so a relative or symlinked invocation still writes an
/// absolute path that keeps working from any working directory. Falls back
/// to the bare name, letting `PATH` decide, if it cannot be resolved.
pub fn yutani_exe() -> PathBuf {
    std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .unwrap_or_else(|_| PathBuf::from("yutani"))
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

/// The launcher entry: an ordinary visible application that starts the
/// daemon. `Icon=y-color` is the full-colour mark `ICONS` puts in
/// `hicolor/scalable/apps` — the handoff's app icon, and the one thing in
/// the theme the shell will not recolour.
pub fn launcher_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Yutani\n\
         Comment=Live thumbnails and client switching for EVE Online\n\
         Exec={exec}\n\
         Icon=y-color\n\
         Terminal=false\n\
         Categories=Game;Utility;\n\
         Keywords=EVE;Online;thumbnails;\n\
         StartupNotify=false\n"
    )
}

/// Where one `ICONS` entry is written under the theme root.
fn icon_path(theme_dir: &Path, dir: &str, name: &str) -> PathBuf {
    theme_dir.join(dir).join(name)
}

fn write_entry(path: &Path, text: String) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

/// Remove one file, reporting whether it was there. Missing is not an error.
fn remove_entry(path: &Path) -> anyhow::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("remove {}", path.display())),
    }
}

/// Write the five icons and both desktop files; returns how many files were
/// written. Overwrites, so running it twice is a no-op with the same count.
pub fn install_to(paths: &Paths, applet_exec: &str, yutani_exec: &str) -> anyhow::Result<usize> {
    for (dir, name, bytes) in ICONS {
        let path = icon_path(&paths.icons, dir, name);
        let parent = path.parent().expect("an icon path always has its theme subdirectory");
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    }
    write_entry(&paths.applet, desktop_entry(applet_exec))?;
    write_entry(&paths.launcher, launcher_entry(yutani_exec))?;
    Ok(ICONS.len() + 2)
}

/// Remove exactly the files `install_to` wrote; returns how many existed.
pub fn uninstall_from(paths: &Paths) -> anyhow::Result<usize> {
    let mut removed = 0;
    for (dir, name, _) in ICONS {
        removed += usize::from(remove_entry(&icon_path(&paths.icons, dir, name))?);
    }
    removed += usize::from(remove_entry(&paths.applet)?);
    removed += usize::from(remove_entry(&paths.launcher)?);
    Ok(removed)
}

/// Refresh the icon cache if the tool is there; never fatal.
fn update_icon_cache(theme_dir: &Path) {
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg("-q")
        .arg("-t")
        .arg("-f")
        .arg(theme_dir)
        .status();
}

/// Refresh the desktop-entry cache so the launcher shows up without a
/// re-login. Same deal as the icon cache: absent on plenty of systems, and
/// the entry works without it, so a failure is ignored.
fn update_desktop_database(applications: &Path) {
    let _ = std::process::Command::new("update-desktop-database").arg(applications).status();
}

pub fn install() -> anyhow::Result<()> {
    let paths = Paths::user();
    let applet = applet_exe()?;
    let yutani = yutani_exe();
    let count = install_to(&paths, &applet.to_string_lossy(), &yutani.to_string_lossy())?;
    update_icon_cache(&paths.icons);
    update_desktop_database(&applications_dir());
    println!(
        "installed {count} files ({}, {} and {})",
        paths.icons.display(),
        paths.applet.display(),
        paths.launcher.display()
    );
    println!("\"Yutani\" is now in Applications");
    println!("one manual step is left:");
    println!("  Settings → Desktop → Panel → Applets → add \"Yutani\"");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let paths = Paths::user();
    let count = uninstall_from(&paths)?;
    update_icon_cache(&paths.icons);
    update_desktop_database(&applications_dir());
    println!("removed {count} files");
    println!("remove the applet from the panel in Settings → Desktop → Panel → Applets");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway `~/.local/share` for one test, removed when the test's
    /// binding is dropped however the test ends.
    struct Sandbox {
        root: PathBuf,
        paths: Paths,
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn dirs(tag: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("yutani-applet-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths {
            icons: root.join("icons").join("hicolor"),
            applet: root.join("applications").join(DESKTOP_ID),
            launcher: root.join("applications").join(LAUNCHER_ID),
        };
        Sandbox { root, paths }
    }

    fn install(s: &Sandbox) -> usize {
        install_to(&s.paths, "/opt/yutani-applet", "/opt/yutani").unwrap()
    }

    fn symbolic(sandbox: &Sandbox) -> PathBuf {
        sandbox.paths.icons.join(crate::assets::SYMBOLIC_DIR)
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

    /// The launcher is the opposite of the applet entry: visible, in the
    /// game/utility categories, and carrying the full-colour icon.
    #[test]
    fn the_launcher_entry_puts_yutani_in_the_applications_list() {
        let text = launcher_entry("/usr/local/bin/yutani");
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.ends_with('\n'));
        for line in [
            "Type=Application",
            "Name=Yutani",
            "Comment=Live thumbnails and client switching for EVE Online",
            "Exec=/usr/local/bin/yutani",
            "Icon=y-color",
            "Terminal=false",
            "Categories=Game;Utility;",
            "Keywords=EVE;Online;thumbnails;",
            "StartupNotify=false",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
        }
        // NoDisplay would hide it — that is the applet entry's job, not this one.
        assert!(!text.contains("NoDisplay"), "the launcher must be visible");
        assert!(!text.contains("X-CosmicApplet"), "the launcher is not an applet");
    }

    /// `Icon=y-color` is a theme lookup, so it only resolves because
    /// `ICONS` installs `y-color.svg` — under `scalable/apps`, where its
    /// blue survives.
    #[test]
    fn the_launcher_icon_name_is_one_the_install_actually_writes() {
        assert!(launcher_entry("/opt/yutani").lines().any(|l| l == "Icon=y-color"));
        assert!(
            ICONS
                .iter()
                .any(|(dir, name, _)| *name == "y-color.svg" && *dir == crate::assets::SCALABLE_DIR)
        );
    }

    #[test]
    fn install_writes_every_icon_and_both_desktop_files_and_is_idempotent() {
        let s = dirs("install");
        assert_eq!(install(&s), 7);
        for (dir, name, bytes) in crate::assets::ICONS {
            assert_eq!(std::fs::read(s.paths.icons.join(dir).join(name)).unwrap(), bytes);
        }
        let applet = std::fs::read_to_string(&s.paths.applet).unwrap();
        assert!(applet.contains("Exec=/opt/yutani-applet"));
        let launcher = std::fs::read_to_string(&s.paths.launcher).unwrap();
        assert!(launcher.contains("Exec=/opt/yutani\n"));
        assert!(launcher.contains("Icon=y-color"));
        // Running it again rewrites the same seven files, not more.
        assert_eq!(install(&s), 7);
        assert_eq!(std::fs::read_dir(symbolic(&s)).unwrap().count(), 4);
        assert_eq!(
            std::fs::read_dir(s.paths.icons.join(crate::assets::SCALABLE_DIR)).unwrap().count(),
            1
        );
        // Both entries, and nothing else, in `applications`.
        assert_eq!(std::fs::read_dir(s.paths.applet.parent().unwrap()).unwrap().count(), 2);
    }

    /// The blue mark must not land among the symbolic icons, where the shell
    /// would happily recolour it into the panel's foreground.
    #[test]
    fn the_launcher_icon_is_installed_outside_the_symbolic_directory() {
        let s = dirs("launcher");
        install(&s);
        assert!(s.paths.icons.join("scalable/apps/y-color.svg").is_file());
        assert!(!symbolic(&s).join("y-color.svg").exists());
    }

    #[test]
    fn uninstall_removes_exactly_our_files() {
        let s = dirs("uninstall");
        install(&s);
        std::fs::write(symbolic(&s).join("someone-elses-symbolic.svg"), b"<svg/>").unwrap();
        let foreign = s.paths.applet.parent().unwrap().join("someone-elses.desktop");
        std::fs::write(&foreign, b"[Desktop Entry]\n").unwrap();
        assert_eq!(uninstall_from(&s.paths).unwrap(), 7);
        assert!(!s.paths.applet.exists());
        assert!(!s.paths.launcher.exists());
        assert!(symbolic(&s).join("someone-elses-symbolic.svg").exists(), "foreign icons stay");
        assert!(foreign.exists(), "foreign desktop entries stay");
        assert_eq!(std::fs::read_dir(symbolic(&s)).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_dir(s.paths.icons.join(crate::assets::SCALABLE_DIR)).unwrap().count(),
            0
        );
    }

    /// A half-installed tree (say, an older version that never wrote the
    /// launcher) must still uninstall cleanly, counting only what was there.
    #[test]
    fn uninstall_counts_only_the_files_that_existed() {
        let s = dirs("partial");
        install(&s);
        std::fs::remove_file(&s.paths.launcher).unwrap();
        assert_eq!(uninstall_from(&s.paths).unwrap(), 6);
    }

    #[test]
    fn uninstall_on_a_clean_system_removes_nothing_and_does_not_fail() {
        let s = dirs("clean");
        assert_eq!(uninstall_from(&s.paths).unwrap(), 0);
    }

    #[test]
    fn the_real_paths_are_the_xdg_user_ones() {
        let paths = Paths::user();
        assert!(paths.icons.ends_with("icons/hicolor"));
        assert!(paths.applet.ends_with("applications/com.yutani.Applet.desktop"));
        assert!(paths.launcher.ends_with("applications/com.yutani.Yutani.desktop"));
        // The test binary has no `yutani-applet` sibling in its deps dir, so
        // the error names the fix rather than falling back to a bare name.
        match applet_exe() {
            Ok(p) => assert_eq!(p.file_name().unwrap(), "yutani-applet"),
            Err(e) => assert!(e.to_string().contains("sudo install"), "{e}"),
        }
        // Canonicalised, so absolute — or the bare name when that failed.
        let yutani = yutani_exe();
        assert!(yutani.is_absolute() || yutani == Path::new("yutani"), "{}", yutani.display());
    }
}
