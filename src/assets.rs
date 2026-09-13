//! The handoff's SVGs, compiled in. `yutani applet install` writes `ICONS`
//! into the user's icon theme so cosmic-panel can show the applet in its
//! list; the applet itself always draws from these bytes, so it looks right
//! straight from `cargo run` with nothing installed.

pub const Y_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-symbolic.svg");
/// The handoff's `#2c2c2c` variant "for light panels". Shipped for
/// completeness only: a symbolic icon is recoloured by the shell from
/// `y-symbolic`, so nothing here — panel button or popup — ever asks for
/// this file. It exists so the installed icon theme matches the bundle.
pub const Y_SYMBOLIC_DARK: &[u8] = include_bytes!("../assets/icons/y-symbolic-dark.svg");
pub const Y_SYNC_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-sync-symbolic.svg");
pub const Y_ATTENTION_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-attention-symbolic.svg");
pub const Y_COLOR: &[u8] = include_bytes!("../assets/icons/y-color.svg");

/// Popup decorations. Not icon-theme icons: they carry fixed colours from
/// the handoff and are never tinted, so they are not installed.
pub const PIN: &[u8] = include_bytes!("../assets/glyphs/pin.svg");
pub const ARROW_UP: &[u8] = include_bytes!("../assets/glyphs/arrow-up.svg");
pub const ARROW_DOWN: &[u8] = include_bytes!("../assets/glyphs/arrow-down.svg");

/// Single-colour marks the shell recolours: `…/icons/hicolor/symbolic/apps`.
pub const SYMBOLIC_DIR: &str = "symbolic/apps";
/// Full-colour art the shell must *not* touch: `…/icons/hicolor/scalable/apps`.
pub const SCALABLE_DIR: &str = "scalable/apps";

/// What `yutani applet install` copies into `~/.local/share/icons/hicolor`:
/// the directory under that root, the file name, and the bytes.
///
/// The blue `y-color.svg` is deliberately *not* among the symbolic ones:
/// anything under `symbolic/` is fair game for the shell to recolour, which
/// would throw its blue away. It is the launcher icon — and, since the
/// `IconState::Active` state (spec §3), the panel mark for a tunnel that is
/// up with EVE behind it, which the applet draws from these bytes with
/// `symbolic(false)` rather than by icon-theme name.
pub const ICONS: [(&str, &str, &[u8]); 5] = [
    (SYMBOLIC_DIR, "y-symbolic.svg", Y_SYMBOLIC),
    (SYMBOLIC_DIR, "y-symbolic-dark.svg", Y_SYMBOLIC_DARK),
    (SYMBOLIC_DIR, "y-sync-symbolic.svg", Y_SYNC_SYMBOLIC),
    (SYMBOLIC_DIR, "y-attention-symbolic.svg", Y_ATTENTION_SYMBOLIC),
    (SCALABLE_DIR, "y-color.svg", Y_COLOR),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_single_colour_svg_named_for_the_icon_theme() {
        assert_eq!(ICONS.len(), 5);
        for (_dir, name, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).expect("svg is utf-8");
            assert!(name.ends_with(".svg"), "{name}");
            assert!(text.starts_with("<svg "), "{name} must start with <svg ");
            assert!(text.contains(r#"viewBox="0 0 96 96""#), "{name} keeps the 96×96 box");
            assert!(!text.contains("c2pa"), "{name} must have the metadata blob stripped");
        }
        assert!(ICONS.iter().any(|(_, n, _)| *n == "y-symbolic.svg"));
        assert!(ICONS.iter().any(|(_, n, _)| *n == "y-color.svg"));
    }

    /// M2: symbolic icons are recoloured by the shell, so the blue launcher
    /// mark must not be filed among them — it goes to `scalable/apps`, the
    /// full-colour app icon the handoff calls it.
    #[test]
    fn the_blue_mark_is_a_launcher_icon_and_never_a_symbolic_one() {
        let dir = |name: &str| ICONS.iter().find(|(_, n, _)| *n == name).expect(name).0;
        assert_eq!(dir("y-color.svg"), SCALABLE_DIR);
        for name in
            ["y-symbolic.svg", "y-symbolic-dark.svg", "y-sync-symbolic.svg", "y-attention-symbolic.svg"]
        {
            assert_eq!(dir(name), SYMBOLIC_DIR, "{name}");
        }
        assert_eq!((SYMBOLIC_DIR, SCALABLE_DIR), ("symbolic/apps", "scalable/apps"));
    }

    /// M3: a badge drawn past the viewBox is silently cropped by the
    /// renderer — at panel sizes that is a flat-bottomed dot. Every `rect`
    /// has to close inside the 96-unit box.
    #[test]
    fn no_icon_draws_outside_its_viewbox() {
        let attr = |tag: &str, name: &str| -> f32 {
            let rest = tag.split_once(&format!("{name}=\"")).unwrap_or_else(|| panic!("{name} in {tag}")).1;
            rest.split_once('"').unwrap().0.parse().unwrap()
        };
        for (_dir, file, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).unwrap();
            for tag in text.split("<rect").skip(1) {
                let tag = tag.split_once('>').unwrap().0;
                let right = attr(tag, "x") + attr(tag, "width");
                let bottom = attr(tag, "y") + attr(tag, "height");
                assert!(right <= 96.0, "{file}: a rect reaches x={right}, past the 96-unit box");
                assert!(bottom <= 96.0, "{file}: a rect reaches y={bottom}, past the 96-unit box");
            }
        }
    }

    #[test]
    fn the_tinted_panel_icons_are_white_and_the_launcher_icon_is_blue() {
        for bytes in [Y_SYMBOLIC, Y_SYNC_SYMBOLIC, Y_ATTENTION_SYMBOLIC] {
            assert!(std::str::from_utf8(bytes).unwrap().contains(r##"fill="#ffffff""##));
        }
        assert!(std::str::from_utf8(Y_SYMBOLIC_DARK).unwrap().contains(r##"fill="#2c2c2c""##));
        assert!(std::str::from_utf8(Y_COLOR).unwrap().contains(r##"fill="#0A5CFF""##));
        // The sync badge must be one even-odd path, or a symbolic tint fills its hole.
        let sync = std::str::from_utf8(Y_SYNC_SYMBOLIC).unwrap();
        assert!(!sync.contains("<circle"), "sync badge must be an even-odd ring, not two circles");
        assert_eq!(sync.matches("fill-rule=\"evenodd\"").count(), 2);
    }

    #[test]
    fn the_popup_glyphs_carry_the_handoff_stroke_colours() {
        assert!(std::str::from_utf8(PIN).unwrap().contains(r##"stroke="#9096A0""##));
        assert!(std::str::from_utf8(ARROW_UP).unwrap().contains(r##"stroke="#2FD6B0""##));
        assert!(std::str::from_utf8(ARROW_DOWN).unwrap().contains(r##"stroke="#5B9BFF""##));
    }
}
