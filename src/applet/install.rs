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

/// Every file one install touches. A struct rather than more and more
/// parameters, so `install_to`/`uninstall_from` stay readable.
pub struct Paths {
    /// The hicolor theme root the `ICONS` go under. Entries land in their
    /// own subdirectory of it (`crate::assets::ICONS` says which), because
    /// symbolic marks and the full-colour launcher icon are filed in
    /// different places.
    pub icons: PathBuf,
    /// The applet's `NoDisplay` entry, for cosmic-panel.
    pub applet: PathBuf,
    /// The launcher entry, for the Applications list.
    pub launcher: PathBuf,
}

impl Paths {
    /// The real XDG user paths under `~/.local/share` (or `$XDG_DATA_HOME`),
    /// or the reason there are none. No fallback: with no resolvable home
    /// (`env -i`, a service user without a passwd entry) an install would
    /// otherwise land in the current directory, report success, and never
    /// be found by the panel — a refusal that says why is the useful outcome.
    pub fn user() -> anyhow::Result<Self> {
        Self::in_data_dir(dirs::data_dir())
    }

    /// `user()` for a given (or missing) data dir, so the refusal is testable.
    pub fn in_data_dir(data_dir: Option<PathBuf>) -> anyhow::Result<Self> {
        let data = data_dir.context("cannot resolve the XDG data dir (is HOME unset?)")?;
        let applications = data.join("applications");
        Ok(Self {
            icons: data.join("icons").join("hicolor"),
            applet: applications.join(DESKTOP_ID),
            launcher: applications.join(LAUNCHER_ID),
        })
    }

    /// The directory both desktop entries live in.
    fn applications(&self) -> &Path {
        self.applet.parent().expect("a desktop entry path always has its applications directory")
    }
}

/// `path` as a Desktop Entry `Exec=` argument. The spec splits `Exec` on
/// whitespace and reads `%x` as a field code, `\`, `"`, `` ` `` and `$` as
/// quoting syntax, so a path containing any of its reserved characters is
/// wrapped in double quotes with those four backslash-escaped, and every
/// `%` is doubled (field codes are expanded after unquoting). A plain path
/// is returned unchanged, so the common entry stays readable.
pub fn exec_quote(path: &str) -> String {
    const RESERVED: &[char] =
        &[' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`'];
    let percent_escaped = path.replace('%', "%%");
    if !path.contains(RESERVED) {
        return percent_escaped;
    }
    let mut quoted = String::with_capacity(percent_escaped.len() + 2);
    quoted.push('"');
    for c in percent_escaped.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
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
    let exec = exec_quote(exec);
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
/// daemon — through `yutani start`, so a `yutani service install` (crash
/// auto-restart) is honoured without rewriting this file. `Icon=y-color` is the full-colour mark `ICONS` puts in
/// `hicolor/scalable/apps` — the handoff's app icon, and the one thing in
/// the theme the shell will not recolour.
pub fn launcher_entry(exec: &str) -> String {
    let exec = exec_quote(exec);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Yutani\n\
         Comment=Live thumbnails and client switching for EVE Online\n\
         Exec={exec} start\n\
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
    let paths = Paths::user()?;
    let applet = applet_exe()?;
    let yutani = yutani_exe();
    let count = install_to(&paths, &applet.to_string_lossy(), &yutani.to_string_lossy())?;
    update_icon_cache(&paths.icons);
    update_desktop_database(paths.applications());
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
    let paths = Paths::user()?;
    let count = uninstall_from(&paths)?;
    update_icon_cache(&paths.icons);
    update_desktop_database(paths.applications());
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
            "Exec=/usr/local/bin/yutani start",
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
        assert!(launcher.contains("Exec=/opt/yutani start\n"));
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

    /// A plain absolute path is written as it is: the common case must
    /// stay byte-for-byte what the tests above expect.
    #[test]
    fn exec_quote_leaves_a_plain_path_alone() {
        assert_eq!(exec_quote("/usr/local/bin/yutani-applet"), "/usr/local/bin/yutani-applet");
    }

    /// Desktop Entry `Exec=` splits on whitespace and reads `%x` as a field
    /// code, so a binary under `~/Projects/EVE Tools` or a `%` in the path
    /// must be quoted and escaped, or cosmic-panel execs nothing at all.
    #[test]
    fn exec_quote_quotes_a_space_and_doubles_a_percent() {
        assert_eq!(
            exec_quote("/home/d/Projects/EVE Tools/target/release/yutani-applet"),
            "\"/home/d/Projects/EVE Tools/target/release/yutani-applet\""
        );
        assert_eq!(exec_quote("/opt/100%/yutani"), "/opt/100%%/yutani");
        assert_eq!(exec_quote("/opt/50% off/yutani"), "\"/opt/50%% off/yutani\"");
    }

    /// Inside the quotes the spec's four characters are backslash-escaped.
    #[test]
    fn exec_quote_escapes_the_reserved_characters_inside_the_quotes() {
        assert_eq!(exec_quote(r#"/a "b"/c\d/$e`f"#), r#""/a \"b\"/c\\d/\$e\`f""#);
    }

    /// The entries carry the quoted form, and a quoted path still ends up
    /// followed by the launcher's ` start`.
    #[test]
    fn the_entries_quote_the_exec_path() {
        let applet = desktop_entry("/home/d/EVE Tools/yutani-applet");
        assert!(applet.lines().any(|l| l == "Exec=\"/home/d/EVE Tools/yutani-applet\""), "{applet}");
        let launcher = launcher_entry("/home/d/EVE Tools/yutani");
        assert!(launcher.lines().any(|l| l == "Exec=\"/home/d/EVE Tools/yutani\" start"), "{launcher}");
    }

    /// No resolvable XDG data dir (`env -i`, a user with no passwd entry) is
    /// a refusal that names the cause — not an install into the current
    /// directory that the panel will never find.
    #[test]
    fn no_data_dir_is_an_error_not_the_current_directory() {
        let err = Paths::in_data_dir(None).map(|_| ()).expect_err("must refuse");
        assert!(err.to_string().contains("HOME"), "{err}");
        let paths = Paths::in_data_dir(Some(PathBuf::from("/x/share"))).unwrap();
        assert_eq!(paths.icons, PathBuf::from("/x/share/icons/hicolor"));
        assert_eq!(paths.applet, PathBuf::from("/x/share/applications").join(DESKTOP_ID));
        assert_eq!(paths.launcher, PathBuf::from("/x/share/applications").join(LAUNCHER_ID));
    }

    #[test]
    fn the_real_paths_are_the_xdg_user_ones() {
        let paths = Paths::user().unwrap();
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
