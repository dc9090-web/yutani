//! User configuration: `~/.config/yutani/config.ron`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Always,
    EveFocusedOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Floating,
    Dock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Modifier names exactly as cosmic-settings writes them in the shortcuts file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Modifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}

/// Keys for `yutani shortcuts install` (spec §7). `next`/`prev` are xkb
/// keysym names as COSMIC writes them ("Right", "Left", "Tab", "a").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    pub focus_prefix: Vec<Modifier>,
    pub next: String,
    pub prev: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self { focus_prefix: vec![Modifier::Ctrl, Modifier::Alt], next: "Right".into(), prev: "Left".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Wayland app_ids that are EVE clients. Defaults to the app_id observed
    /// for the EVE client under Steam + GE-Proton with
    /// `PROTON_ENABLE_WAYLAND=1` (`exefile.exe`) plus the Steam launcher's
    /// app_id (`steam_app_8500`), seen only transiently before the client
    /// takes over.
    pub app_ids: Vec<String>,
    /// Thumbnail width in logical pixels; height follows the window's aspect.
    pub thumb_width: u32,
    /// Max capture rate: 10, 15, 30 or 60.
    pub fps: u32,
    /// "#rrggbb" or "#rrggbbaa"; `None` follows the COSMIC theme's accent
    /// colour (the same colour the compositor outlines the focused window with).
    pub active_border: Option<String>,
    pub inactive_border: String,
    /// Border width in logical px; 0 (default) draws no border at all —
    /// corners are rounded in the frame itself.
    pub border_px: u32,
    pub show_names: bool,
    /// Hover zoom multiplier, 1.0–4.0 (1.0 = no hover zoom).
    pub zoom_factor: f32,
    pub visibility: Visibility,
    /// Hide the thumbnail of the client that currently has focus.
    pub hide_active: bool,
    /// Snap to a 32 px grid while dragging.
    pub snap_grid: bool,
    /// Snap flush against other thumbnails while dragging.
    pub snap_edges: bool,
    pub mode: Mode,
    /// Corner radius of each thumbnail in logical px (COSMIC window radius is 8).
    pub corner_radius: u32,
    pub dock_edge: Edge,
    /// Keyboard shortcuts written by `yutani shortcuts install`.
    pub shortcuts: ShortcutsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_ids: vec!["exefile.exe".to_string(), "steam_app_8500".to_string()],
            thumb_width: 480,
            fps: 30,
            active_border: None,
            inactive_border: "#404040".to_string(),
            border_px: 0,
            show_names: true,
            zoom_factor: 1.0,
            visibility: Visibility::EveFocusedOnly,
            hide_active: false,
            snap_grid: true,
            snap_edges: true,
            mode: Mode::Dock,
            corner_radius: 8,
            dock_edge: Edge::Top,
            shortcuts: ShortcutsConfig::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yutani")
        .join("config.ron")
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    /// Missing file → defaults. Unreadable/unparseable file → warn, defaults,
    /// and the file is never touched.
    pub fn load_from(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!("cannot read {}: {err}; using defaults", path.display());
                return Self::default();
            }
        };
        match ron::from_str(&text) {
            Ok(config) => Config::validate(config),
            Err(err) => {
                tracing::warn!("cannot parse {}: {err}; using defaults", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Replace out-of-range values with defaults, warning about each.
    pub fn validate(mut self) -> Self {
        let d = Config::default();
        macro_rules! check {
            ($field:ident, $ok:expr, $why:literal) => {
                if !$ok(&self.$field) {
                    tracing::warn!(concat!("config: ", stringify!($field), " {:?} is invalid (", $why, "); using default"), self.$field);
                    self.$field = d.$field.clone();
                }
            };
        }
        check!(thumb_width, |v: &u32| (80..=1600).contains(v), "80..=1600");
        check!(fps, |v: &u32| [10, 15, 30, 60].contains(v), "10|15|30|60");
        check!(zoom_factor, |v: &f32| (1.0..=4.0).contains(v), "1.0..=4.0");
        check!(border_px, |v: &u32| *v <= 16, "0..=16");
        check!(corner_radius, |v: &u32| *v <= 64, "0..=64");
        check!(active_border, |v: &Option<String>| v.as_deref().is_none_or(|h| parse_color(h).is_some()), "#rrggbb[aa] or absent");
        check!(inactive_border, |v: &String| parse_color(v).is_some(), "#rrggbb[aa]");
        if self.app_ids.is_empty() {
            tracing::warn!("config: app_ids is empty; using default");
            self.app_ids = d.app_ids.clone();
        }
        {
            // Trimmed, because what is left here is what gets written to the
            // shortcuts file verbatim, and " Right " is no keysym name.
            let (next, prev) = (self.shortcuts.next.trim().to_string(), self.shortcuts.prev.trim().to_string());
            let (next, prev) = (next.as_str(), prev.as_str());
            // Two bindings on one key would silently overwrite each other in
            // the shortcuts file; the digits are taken by `focus 1..9`.
            let is_digit = |k: &str| k.len() == 1 && k.as_bytes()[0].is_ascii_digit() && k != "0";
            if next.is_empty() || prev.is_empty() || next.eq_ignore_ascii_case(prev) || is_digit(next) || is_digit(prev) {
                tracing::warn!("config: shortcuts.next/prev must be distinct, non-empty and not 1-9; using defaults");
                self.shortcuts = ShortcutsConfig::default();
            } else if resolve_keysym(next).is_none() || resolve_keysym(prev).is_none() {
                // cosmic-settings-config reads `key` through
                // `xkb::keysym_from_name`; a name that is not a keysym fails
                // the whole `custom` map in cosmic-comp, which would silently
                // disable *every* custom shortcut the user has, not just ours.
                tracing::warn!(
                    "config: shortcuts.next {next:?} / prev {prev:?} must be xkb keysym names (\"Right\", \"Tab\", \"a\", \"F12\"); using defaults"
                );
                self.shortcuts = ShortcutsConfig::default();
            } else {
                (self.shortcuts.next, self.shortcuts.prev) = (next.to_string(), prev.to_string());
            }
        }
        {
            let prefix = &self.shortcuts.focus_prefix;
            let mut seen = std::collections::HashSet::new();
            let has_dup = !prefix.iter().all(|m| seen.insert(*m));
            if prefix.is_empty() || has_dup {
                tracing::warn!("config: shortcuts.focus_prefix {prefix:?} must be non-empty with no duplicate modifiers; using defaults");
                self.shortcuts = ShortcutsConfig::default();
            }
        }
        self
    }
}

/// Resolve an xkb keysym name the way cosmic-comp does: the exact name
/// first, then a case-insensitive retry (so "right" finds `Right`). `None`
/// for a name that is no keysym at all.
pub fn resolve_keysym(name: &str) -> Option<xkbcommon::xkb::Keysym> {
    use xkbcommon::xkb;
    let exact = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    if exact != xkb::Keysym::NoSymbol {
        return Some(exact);
    }
    let lax = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
    (lax != xkb::Keysym::NoSymbol).then_some(lax)
}

/// "#rrggbb" or "#rrggbbaa" → [r, g, b, a] in 0.0–1.0.
pub fn parse_color(hex: &str) -> Option<[f32; 4]> {
    let digits = hex.strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    if digits.len() != 6 && digits.len() != 8 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let r = byte(0)?;
    let g = byte(2)?;
    let b = byte(4)?;
    let a = if digits.len() == 8 { byte(6)? } else { 255 };
    Some([r, g, b, a].map(|v| v as f32 / 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.app_ids, vec!["exefile.exe".to_string(), "steam_app_8500".to_string()]);
        assert_eq!(c.thumb_width, 480);
        assert_eq!(c.fps, 30);
        assert_eq!(c.active_border, None);
        assert_eq!(c.inactive_border, "#404040");
        assert_eq!(c.border_px, 0);
        assert!(c.show_names);
    }

    #[test]
    fn round_trips_through_ron() {
        let dir = std::env::temp_dir().join(format!("yutani-test-{}", std::process::id()));
        let path = dir.join("config.ron");
        let mut c = Config::default();
        c.thumb_width = 200;
        c.app_ids.push("firefox".into());
        c.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(Config::load_from(Path::new("/nonexistent/yutani.ron")), Config::default());
    }

    #[test]
    fn bad_file_gives_defaults_and_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("yutani-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        std::fs::write(&path, "(this is not ron").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(this is not ron");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let dir = std::env::temp_dir().join(format!("yutani-partial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        std::fs::write(&path, "(fps: 60)").unwrap();
        let c = Config::load_from(&path);
        assert_eq!(c.fps, 60);
        assert_eq!(c.thumb_width, 480);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#ff8800"), Some([1.0, 136.0 / 255.0, 0.0, 1.0]));
        assert_eq!(parse_color("#00000080"), Some([0.0, 0.0, 0.0, 128.0 / 255.0]));
        assert_eq!(parse_color("ff8800"), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#gg0000"), None);
    }

    #[test]
    fn non_ascii_color_is_rejected_not_a_panic() {
        assert_eq!(parse_color("#a±bcd"), None);
        assert_eq!(parse_color("#ab±cdef"), None);
    }

    #[test]
    fn plan2_defaults() {
        let c = Config::default();
        assert_eq!(c.zoom_factor, 1.0);
        assert_eq!(c.visibility, Visibility::EveFocusedOnly);
        assert!(!c.hide_active);
        assert!(c.snap_grid);
        assert!(c.snap_edges);
    }

    #[test]
    fn validate_replaces_bad_values_with_defaults() {
        let c = Config {
            thumb_width: 10,
            fps: 17,
            zoom_factor: 0.2,
            active_border: Some("nope".into()),
            border_px: 99,
            ..Config::default()
        }
        .validate();
        let d = Config::default();
        assert_eq!(c.thumb_width, d.thumb_width);
        assert_eq!(c.fps, d.fps);
        assert_eq!(c.zoom_factor, d.zoom_factor);
        assert_eq!(c.active_border, d.active_border);
        assert_eq!(c.border_px, d.border_px);
    }

    #[test]
    fn validate_keeps_good_values() {
        let c = Config { thumb_width: 480, fps: 60, zoom_factor: 2.0, border_px: 0, ..Config::default() };
        assert_eq!(c.clone().validate(), c);
    }

    #[test]
    fn plan3_defaults_and_validation() {
        let c = Config::default();
        assert_eq!(c.mode, Mode::Dock);
        assert_eq!(c.dock_edge, Edge::Top);
        assert_eq!(c.corner_radius, 8);
        let text = "(mode: Floating, dock_edge: Left)";
        let c: Config = ron::from_str(text).unwrap();
        assert_eq!(c.mode, Mode::Floating);
        assert_eq!(c.dock_edge, Edge::Left);
    }

    #[test]
    fn stale_opacity_field_is_ignored() {
        // Removed in the GPU-thumbnails spec; old config files still parse.
        let c: Config = ron::from_str("(opacity: 0.5, thumb_width: 300)").unwrap();
        assert_eq!(c.thumb_width, 300);
    }

    #[test]
    fn load_from_validates() {
        let dir = std::env::temp_dir().join(format!("yutani-val-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.ron");
        std::fs::write(&path, "(fps: 17, thumb_width: 400)").unwrap();
        let c = Config::load_from(&path);
        assert_eq!(c.fps, 30);
        assert_eq!(c.thumb_width, 400);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shortcuts_default_and_parse() {
        let c = Config::default();
        assert_eq!(c.shortcuts.focus_prefix, vec![Modifier::Ctrl, Modifier::Alt]);
        assert_eq!(c.shortcuts.next, "Right");
        assert_eq!(c.shortcuts.prev, "Left");
        let c: Config = ron::from_str("(shortcuts: (focus_prefix: [Super], next: \"n\", prev: \"p\"))").unwrap();
        assert_eq!(c.shortcuts.focus_prefix, vec![Modifier::Super]);
        assert_eq!(c.shortcuts.next, "n");
        // Partial override keeps the other defaults.
        let c: Config = ron::from_str("(shortcuts: (next: \"Tab\"))").unwrap();
        assert_eq!(c.shortcuts.prev, "Left");
    }

    #[test]
    fn validate_rejects_empty_equal_or_digit_shortcut_keys() {
        let mut c = Config::default();
        c.shortcuts.next = String::new();
        assert_eq!(c.validate().shortcuts.next, "Right");
        let mut c = Config::default();
        c.shortcuts.next = "Tab".into();
        c.shortcuts.prev = "tab".into();
        assert_eq!(c.validate().shortcuts.prev, "Left");
        let mut c = Config::default();
        c.shortcuts.prev = "3".into();
        assert_eq!(c.validate().shortcuts.prev, "Left");
        let mut c = Config::default();
        c.shortcuts.next = "Tab".into();
        c.shortcuts.prev = "grave".into();
        assert_eq!(c.clone().validate(), c);
    }

    #[test]
    fn validate_rejects_keys_that_are_not_keysym_names() {
        // cosmic-settings-config parses `key` with xkb::keysym_from_name; an
        // unknown name fails the whole custom map and would silently disable
        // every custom shortcut the user has.
        let mut c = Config::default();
        c.shortcuts.next = "Rihgt".into();
        assert_eq!(c.validate().shortcuts.next, "Right");
        let mut c = Config::default();
        c.shortcuts.prev = "Ctrl+Left".into();
        assert_eq!(c.validate().shortcuts.prev, "Left");
        // Real keysym names survive, including ones that only differ in case.
        let c = Config { shortcuts: ShortcutsConfig { next: "bracketright".into(), prev: "F12".into(), ..ShortcutsConfig::default() }, ..Config::default() };
        assert_eq!(c.clone().validate(), c);
        // Surrounding whitespace is what gets written to the shortcuts file,
        // and " Right " is no keysym; keep the trimmed name, not the default.
        let mut c = Config::default();
        c.shortcuts.next = " Tab ".into();
        assert_eq!(c.validate().shortcuts.next, "Tab");
    }

    #[test]
    fn validate_rejects_empty_or_duplicate_focus_prefix() {
        let mut c = Config::default();
        c.shortcuts.focus_prefix = vec![];
        assert_eq!(c.validate().shortcuts.focus_prefix, vec![Modifier::Ctrl, Modifier::Alt]);
        let mut c = Config::default();
        c.shortcuts.focus_prefix = vec![Modifier::Super, Modifier::Super];
        assert_eq!(c.validate().shortcuts.focus_prefix, vec![Modifier::Ctrl, Modifier::Alt]);
        let mut c = Config::default();
        c.shortcuts.focus_prefix = vec![Modifier::Super];
        assert_eq!(c.clone().validate(), c);
    }
}
