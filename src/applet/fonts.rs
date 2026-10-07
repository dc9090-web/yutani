//! The popover's bundled faces (Nostromo handoff "Typography"): B612 Mono
//! for everything, Michroma for the wordmark and the section indices, and a
//! five-glyph Noto Sans JP for the one Japanese subline. Loaded once at
//! applet start; should a load fail, every face falls back to COSMIC's
//! monospace (`set_missing`).
//!
//! iced has no letter-spacing, so the view tracks text itself, one glyph per
//! fixed-width cell. `advance_em` is the glyph's own advance for that.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use cosmic::iced::Font;
use cosmic::iced::font::{Family, Weight};

pub const B612_MONO: &[u8] = include_bytes!("../../assets/fonts/B612Mono-Regular.ttf");
pub const B612_MONO_BOLD: &[u8] = include_bytes!("../../assets/fonts/B612Mono-Bold.ttf");
pub const MICHROMA: &[u8] = include_bytes!("../../assets/fonts/Michroma-Regular.ttf");
pub const NOTO_JP: &[u8] = include_bytes!("../../assets/fonts/NotoSansJP-Yutani.ttf");
pub const ALL: [&[u8]; 4] = [B612_MONO, B612_MONO_BOLD, MICHROMA, NOTO_JP];

pub const FAMILY_MONO: &str = "B612 Mono";
pub const FAMILY_DISPLAY: &str = "Michroma";
pub const FAMILY_JP: &str = "Noto Sans JP";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Mono,
    MonoBold,
    Display,
    Jp,
}

static MISSING: AtomicBool = AtomicBool::new(false);

/// A font failed to load: from now on every face is COSMIC's monospace.
pub fn set_missing() {
    MISSING.store(true, Ordering::Relaxed);
}

pub fn font(face: Face) -> Font {
    if MISSING.load(Ordering::Relaxed) {
        let mono = cosmic::font::mono();
        return if face == Face::MonoBold { Font { weight: Weight::Bold, ..mono } } else { mono };
    }
    match face {
        Face::Mono => Font { family: Family::Name(FAMILY_MONO), ..Font::DEFAULT },
        Face::MonoBold => Font { family: Family::Name(FAMILY_MONO), weight: Weight::Bold, ..Font::DEFAULT },
        Face::Display => Font { family: Family::Name(FAMILY_DISPLAY), ..Font::DEFAULT },
        Face::Jp => Font { family: Family::Name(FAMILY_JP), weight: Weight::Medium, ..Font::DEFAULT },
    }
}

fn bytes(face: Face) -> &'static [u8] {
    match face {
        Face::Mono => B612_MONO,
        Face::MonoBold => B612_MONO_BOLD,
        Face::Display => MICHROMA,
        Face::Jp => NOTO_JP,
    }
}

fn parsed(face: Face) -> Option<&'static ttf_parser::Face<'static>> {
    static FACES: [OnceLock<Option<ttf_parser::Face<'static>>>; 4] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
    FACES[face as usize].get_or_init(|| ttf_parser::Face::parse(bytes(face), 0).ok()).as_ref()
}

fn raw_advance(face: Face, ch: char) -> Option<f32> {
    let f = parsed(face)?;
    let gid = f.glyph_index(ch)?;
    Some(f32::from(f.glyph_hor_advance(gid)?) / f32::from(f.units_per_em()))
}

/// `ch`'s advance in em. A glyph the face lacks (it will render in a
/// fallback font) takes the wider of this face's and B612 Mono's `0`, so a
/// tracked cell is never narrower than what lands in it.
pub fn advance_em(face: Face, ch: char) -> f32 {
    raw_advance(face, ch).unwrap_or_else(|| {
        raw_advance(face, '0').unwrap_or(0.0).max(raw_advance(Face::Mono, '0').unwrap_or(0.6))
    })
}

/// Load every bundled face; `true` when all of them loaded.
pub fn load_all() -> cosmic::iced::Task<bool> {
    let tasks = ALL.map(|b| cosmic::iced::font::load(b).map(|r| r.is_ok()));
    cosmic::iced::Task::batch(tasks).collect().map(|oks: Vec<bool>| oks.into_iter().all(|ok| ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_parses_and_has_the_glyphs_it_is_used_for() {
        // B612 Mono has no ▲ or ◄ (it does have ▼ ▶); those would fall back.
        for face in [Face::Mono, Face::MonoBold] {
            for ch in "YUTANI 0123456789·—▼▶…%°/+:.".chars() {
                assert!(raw_advance(face, ch).is_some(), "{face:?} {ch:?}");
            }
        }
        for ch in "YUTANI01234".chars() {
            assert!(raw_advance(Face::Display, ch).is_some(), "Display {ch:?}");
        }
        for ch in "ユタニ重工".chars() {
            assert!(raw_advance(Face::Jp, ch).is_some(), "Jp {ch:?}");
        }
    }

    /// Each embedded font's family (typographic ID 16 if present, else ID 1)
    /// is the name `font()` asks for.
    #[test]
    fn embedded_family_names_match_the_constants() {
        fn family(bytes: &[u8]) -> String {
            let face = ttf_parser::Face::parse(bytes, 0).unwrap();
            let find = |id| {
                face.names().into_iter().find(|n| n.name_id == id && n.is_unicode()).and_then(|n| n.to_string())
            };
            find(16).or_else(|| find(1)).expect("family name")
        }
        assert_eq!(family(B612_MONO), FAMILY_MONO);
        assert_eq!(family(B612_MONO_BOLD), FAMILY_MONO);
        assert_eq!(family(MICHROMA), FAMILY_DISPLAY);
        assert_eq!(family(NOTO_JP), FAMILY_JP);
    }

    /// B612 Mono is monospaced: every digit has the same advance, so the
    /// readouts never jitter.
    #[test]
    fn the_mono_face_is_monospaced() {
        let w = advance_em(Face::Mono, '0');
        for ch in "123456789ABCXYZ".chars() {
            assert!((advance_em(Face::Mono, ch) - w).abs() < 1e-6, "{ch:?}");
        }
    }

    /// A glyph a face lacks falls back to the advance of `0` rather than
    /// collapsing to zero width.
    #[test]
    fn a_missing_glyph_falls_back_to_a_digit_width() {
        let mono0 = raw_advance(Face::Mono, '0').unwrap();
        let jp0 = raw_advance(Face::Jp, '0').unwrap_or(0.0);
        assert!(raw_advance(Face::Jp, 'Q').is_none());
        assert_eq!(advance_em(Face::Jp, 'Q'), jp0.max(mono0));
        let display0 = raw_advance(Face::Display, '0').unwrap();
        assert!(raw_advance(Face::Display, 'ユ').is_none());
        assert_eq!(advance_em(Face::Display, 'ユ'), display0.max(mono0));
    }

    #[test]
    fn handles_name_the_bundled_families() {
        assert_eq!(font(Face::MonoBold).weight, cosmic::iced::font::Weight::Bold);
        assert_eq!(font(Face::Mono).family, cosmic::iced::font::Family::Name(FAMILY_MONO));
        assert_eq!(ALL.len(), 4);
    }
}
