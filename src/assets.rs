//! The icons, compiled in. `yutani applet install` writes `ICONS` into the
//! user's icon theme so cosmic-panel can show the applet in its list and
//! the launcher has its icon; the applet itself always draws the panel mark
//! from these bytes, so it looks right straight from `cargo run` with
//! nothing installed. Geometry: the 2026-09-14 redesign handoff, "Screen 3".

/// The panel mark, two-piece (the slice through the stem), for 22 px and
/// up. `fill="currentColor"`: the panel tints it.
pub const YUTANI_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/yutani-symbolic.svg");
/// The same mark with the slice omitted — at 16 px it would land on half
/// a pixel.
pub const YUTANI_SYMBOLIC_16: &[u8] = include_bytes!("../assets/icons/yutani-symbolic-16.svg");
/// The launcher icon: plate, sheen, rim, the extruded mark, drop shadow.
pub const YUTANI_APP: &[u8] = include_bytes!("../assets/icons/yutani.svg");
/// The launcher icon below 64 px: no slice, no top-contour stroke.
pub const YUTANI_APP_48: &[u8] = include_bytes!("../assets/icons/yutani-48.svg");
pub const YUTANI_APP_32: &[u8] = include_bytes!("../assets/icons/yutani-32.svg");

/// Popup decorations. Not icon-theme icons: they carry fixed colours from
/// the handoff and are never tinted, so they are not installed.
pub const PIN: &[u8] = include_bytes!("../assets/glyphs/pin.svg");
pub const ARROW_UP: &[u8] = include_bytes!("../assets/glyphs/arrow-up.svg");
pub const ARROW_DOWN: &[u8] = include_bytes!("../assets/glyphs/arrow-down.svg");

/// The symbolic mark at any size: `…/icons/hicolor/symbolic/status`.
pub const SYMBOLIC_DIR: &str = "symbolic/status";
/// The solid 16 px mark: `…/icons/hicolor/16x16/status`.
pub const SYMBOLIC_16_DIR: &str = "16x16/status";
/// Full-colour art the shell must *not* touch: `…/icons/hicolor/scalable/apps`.
pub const SCALABLE_DIR: &str = "scalable/apps";
pub const APPS_48_DIR: &str = "48x48/apps";
pub const APPS_32_DIR: &str = "32x32/apps";

/// The panel icon's theme name (`Icon=` of the applet's desktop entry).
pub const SYMBOLIC_NAME: &str = "yutani-symbolic";
/// The launcher icon's theme name.
pub const APP_NAME: &str = "yutani";

/// What `yutani applet install` copies into `~/.local/share/icons/hicolor`:
/// the directory under that root, the file name, and the bytes. The same
/// theme name lands in several size directories, which is how an icon
/// theme picks the solid mark at 16 px and the simpler plate below 64.
pub const ICONS: [(&str, &str, &[u8]); 5] = [
    (SYMBOLIC_DIR, "yutani-symbolic.svg", YUTANI_SYMBOLIC),
    (SYMBOLIC_16_DIR, "yutani-symbolic.svg", YUTANI_SYMBOLIC_16),
    (SCALABLE_DIR, "yutani.svg", YUTANI_APP),
    (APPS_48_DIR, "yutani.svg", YUTANI_APP_48),
    (APPS_32_DIR, "yutani.svg", YUTANI_APP_32),
];

/// The files the pre-redesign install wrote (2026-09-12 handoff). Both
/// `install` and `uninstall` remove them, so an upgraded machine is left
/// with no stale marks in its theme.
pub const LEGACY_ICONS: [(&str, &str); 5] = [
    ("symbolic/apps", "y-symbolic.svg"),
    ("symbolic/apps", "y-symbolic-dark.svg"),
    ("symbolic/apps", "y-sync-symbolic.svg"),
    ("symbolic/apps", "y-attention-symbolic.svg"),
    ("scalable/apps", "y-color.svg"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).expect("svg is utf-8")
    }

    /// The handoff's rule for the tray: a `currentColor` symbolic SVG on
    /// the 32-unit grid, no gradients, no fixed colours.
    #[test]
    fn the_panel_marks_are_current_colour_on_the_32_unit_grid() {
        for bytes in [YUTANI_SYMBOLIC, YUTANI_SYMBOLIC_16] {
            let t = text(bytes);
            assert!(t.starts_with("<svg "));
            assert!(t.contains(r#"viewBox="0 0 32 32""#));
            assert!(t.contains(r#"fill="currentColor""#));
            assert!(!t.contains(r##"fill="#"##), "no fixed colour");
            assert!(!t.contains("Gradient"), "no gradients");
        }
        // Two pieces from 22 px up (the slice is the mark's signature); one
        // solid piece at 16, where the slice would land on half a pixel.
        assert_eq!(text(YUTANI_SYMBOLIC).matches("<path").count(), 2);
        assert_eq!(text(YUTANI_SYMBOLIC_16).matches("<path").count(), 1);
    }

    /// The launcher icon is the one place with depth: a 116 plate in a 128
    /// box, radius 26, with the face gradient and a dark side wall. Below
    /// 64 px the slice and the top-contour stroke are dropped.
    #[test]
    fn the_launcher_icons_are_the_plate_with_the_extruded_mark() {
        for bytes in [YUTANI_APP, YUTANI_APP_48, YUTANI_APP_32] {
            let t = text(bytes);
            assert!(t.contains(r#"viewBox="0 0 128 128""#));
            assert!(t.contains(r#"<rect x="6" y="6" width="116" height="116" rx="26" fill="url(#plate)""#));
            assert!(t.contains(r#"fill="url(#face)""#));
            assert!(t.contains(r##"fill="#0a0d14" fill-opacity="0.55""##), "the side wall");
            assert!(t.contains("translate(24 24) scale(2.5)"));
        }
        assert!(text(YUTANI_APP).contains(r#"stroke-opacity="0.85" stroke-width="0.35""#));
        assert_eq!(text(YUTANI_APP).matches("<path").count(), 5, "two pieces × wall/face + contour");
        for bytes in [YUTANI_APP_48, YUTANI_APP_32] {
            assert!(!text(bytes).contains(r#"stroke-width="0.35""#), "no contour stroke below 64");
            assert_eq!(text(bytes).matches("<path").count(), 2, "one solid piece × wall/face");
        }
    }

    /// One theme name per role, each filed where the icon theme looks for
    /// that size; nothing fixed-colour under `symbolic/`.
    #[test]
    fn every_icon_is_filed_under_its_theme_name_and_size() {
        let names: Vec<&str> = ICONS.iter().map(|(_, n, _)| *n).collect();
        assert_eq!(names, ["yutani-symbolic.svg", "yutani-symbolic.svg", "yutani.svg", "yutani.svg", "yutani.svg"]);
        let dirs: Vec<&str> = ICONS.iter().map(|(d, _, _)| *d).collect();
        assert_eq!(dirs, [SYMBOLIC_DIR, SYMBOLIC_16_DIR, SCALABLE_DIR, APPS_48_DIR, APPS_32_DIR]);
        assert_eq!((SYMBOLIC_NAME, APP_NAME), ("yutani-symbolic", "yutani"));
        for (dir, name, bytes) in ICONS {
            let symbolic = dir.starts_with("symbolic") || dir.ends_with("status");
            assert_eq!(symbolic, name.ends_with("-symbolic.svg"), "{dir}/{name}");
            assert_eq!(symbolic, text(bytes).contains("currentColor"), "{dir}/{name}");
        }
        // Every old file is named for removal, and none of them is written.
        assert_eq!(LEGACY_ICONS.len(), 5);
        for (dir, name) in LEGACY_ICONS {
            assert!(name.starts_with("y-"), "{name}");
            assert!(!ICONS.iter().any(|(d, n, _)| *d == dir && *n == name));
        }
    }

    #[test]
    fn the_popup_glyphs_carry_the_handoff_stroke_colours() {
        assert!(text(PIN).contains(r##"stroke="#9096A0""##));
        assert!(text(ARROW_UP).contains(r##"stroke="#2FD6B0""##));
        assert!(text(ARROW_DOWN).contains(r##"stroke="#5B9BFF""##));
    }
}
