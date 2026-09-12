//! The handoff's SVGs, compiled in. `yutani applet install` writes `ICONS`
//! into the user's icon theme so cosmic-panel can show the applet in its
//! list; the applet itself always draws from these bytes, so it looks right
//! straight from `cargo run` with nothing installed.

pub const Y_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-symbolic.svg");
pub const Y_SYMBOLIC_DARK: &[u8] = include_bytes!("../assets/icons/y-symbolic-dark.svg");
pub const Y_SYNC_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-sync-symbolic.svg");
pub const Y_ATTENTION_SYMBOLIC: &[u8] = include_bytes!("../assets/icons/y-attention-symbolic.svg");
pub const Y_COLOR: &[u8] = include_bytes!("../assets/icons/y-color.svg");

/// Popup decorations. Not icon-theme icons: they carry fixed colours from
/// the handoff and are never tinted, so they are not installed.
pub const PIN: &[u8] = include_bytes!("../assets/glyphs/pin.svg");
pub const ARROW_UP: &[u8] = include_bytes!("../assets/glyphs/arrow-up.svg");
pub const ARROW_DOWN: &[u8] = include_bytes!("../assets/glyphs/arrow-down.svg");

/// What `yutani applet install` copies to the hicolor symbolic apps dir.
pub const ICONS: [(&str, &[u8]); 5] = [
    ("y-symbolic.svg", Y_SYMBOLIC),
    ("y-symbolic-dark.svg", Y_SYMBOLIC_DARK),
    ("y-sync-symbolic.svg", Y_SYNC_SYMBOLIC),
    ("y-attention-symbolic.svg", Y_ATTENTION_SYMBOLIC),
    ("y-color.svg", Y_COLOR),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_single_colour_svg_named_for_the_icon_theme() {
        assert_eq!(ICONS.len(), 5);
        for (name, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).expect("svg is utf-8");
            assert!(name.ends_with(".svg"), "{name}");
            assert!(text.starts_with("<svg "), "{name} must start with <svg ");
            assert!(text.contains(r#"viewBox="0 0 96 96""#), "{name} keeps the 96×96 box");
            assert!(!text.contains("c2pa"), "{name} must have the metadata blob stripped");
        }
        assert!(ICONS.iter().any(|(n, _)| *n == "y-symbolic.svg"));
        assert!(ICONS.iter().any(|(n, _)| *n == "y-color.svg"));
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
