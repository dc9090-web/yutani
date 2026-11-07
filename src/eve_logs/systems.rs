//! Solar-system security status, as the client shows it.
//!
//! The logs name the system but not its security, so a table of every
//! system's true security ships in the binary (`assets/systems.tsv`,
//! name and three-decimal security, from CCP's Static Data Export via
//! fuzzwork's CSV dump, 2026-11-04 build). The client rounds the true
//! value to one decimal for display, with one exception: anything above
//! zero but below 0.05 shows as 0.1, never 0.0, so no low-sec system
//! can pass for null.

use std::collections::HashMap;
use std::sync::LazyLock;

static TABLE: &str = include_str!("../../assets/systems.tsv");

static SECURITY: LazyLock<HashMap<&'static str, f32>> = LazyLock::new(|| {
    TABLE
        .lines()
        .filter_map(|l| {
            let (name, sec) = l.split_once('\t')?;
            Some((name, sec.trim().parse().ok()?))
        })
        .collect()
});

/// The true (unrounded) security of a named system.
pub fn true_security(name: &str) -> Option<f32> {
    SECURITY.get(name).copied()
}

/// The one-decimal value the client displays for a true security.
pub fn displayed(true_sec: f32) -> f32 {
    if true_sec > 0.0 && true_sec < 0.05 {
        return 0.1;
    }
    let d = (true_sec * 10.0).round() / 10.0;
    if d == 0.0 { 0.0 } else { d } // no "-0.0"
}

/// The displayed security of a named system, if the system is known.
pub fn security(name: &str) -> Option<f32> {
    true_security(name).map(displayed)
}

/// `0.3`, `1.0`, `-0.5`.
pub fn label(displayed: f32) -> String {
    format!("{displayed:.1}")
}

/// The client's colour for a displayed security, as `[r, g, b]` in 0..=1.
pub fn color(displayed: f32) -> [f32; 3] {
    let hex: u32 = match (displayed * 10.0).round() as i32 {
        i32::MIN..=0 => 0xF00000,
        1 => 0xD73000,
        2 => 0xF04800,
        3 => 0xF06000,
        4 => 0xD77700,
        5 => 0xEFEF00,
        6 => 0x8FEF2F,
        7 => 0x00F000,
        8 => 0x00EF47,
        9 => 0x48F0C0,
        _ => 0x2FEFEF,
    };
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_every_system_once() {
        assert_eq!(SECURITY.len(), 8490);
        assert_eq!(TABLE.lines().count(), 8490);
    }

    #[test]
    fn known_systems_display_as_the_client_does() {
        assert_eq!(security("Jita"), Some(0.9));
        assert_eq!(security("Akora"), Some(0.3));
        assert_eq!(security("Messoya"), Some(0.3));
        assert_eq!(security("Korsiki"), Some(0.6));
        assert_eq!(security("Hophib"), Some(0.1), "0.029 true sec is shown as 0.1, not 0.0");
        assert_eq!(security("Polaris"), Some(0.0));
        assert_eq!(security("J100001"), Some(-1.0));
        assert_eq!(security("Nowhere"), None);
    }

    #[test]
    fn rounding_and_labels() {
        assert_eq!(displayed(0.049), 0.1);
        assert_eq!(displayed(0.05), 0.1);
        assert_eq!(displayed(0.449), 0.4);
        assert_eq!(displayed(-0.0006), 0.0);
        assert_eq!(displayed(-0.26), -0.3);
        assert_eq!(label(0.0), "0.0");
        assert_eq!(label(-0.3), "-0.3");
        assert_eq!(label(1.0), "1.0");
    }

    #[test]
    fn colours_follow_the_client_palette() {
        assert_eq!(color(1.0), [0x2f as f32 / 255.0, 0xef as f32 / 255.0, 0xef as f32 / 255.0]);
        assert_eq!(color(0.5), [0xef as f32 / 255.0, 0xef as f32 / 255.0, 0.0]);
        assert_eq!(color(0.3), [0xf0 as f32 / 255.0, 0x60 as f32 / 255.0, 0.0]);
        assert_eq!(color(0.0), [0xf0 as f32 / 255.0, 0.0, 0.0]);
        assert_eq!(color(-1.0), color(0.0));
    }
}
