//! `yutani shortcuts install|uninstall`: our entries in COSMIC's custom
//! keyboard-shortcuts file (spec §7). Only entries whose action spawns
//! `yutani` are ever added, replaced or removed; everything else in the
//! file is preserved as the RON text it was.

use anyhow::Context as _;
use ron::value::RawValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::config::{Modifier, ShortcutsConfig};

/// cosmic-settings-daemon's `Binding`, field-for-field (it deserialises
/// with `deny_unknown_fields`, so nothing may be added here). Upstream
/// (de)serialises `key` through a custom function as a *bare* string
/// (`key: "t"`, never `Some("t")`) while `description` is a plain `Option`
/// (`Some("…")`); `keysym` below mirrors that so we can read cosmic's own
/// files and write what cosmic accepts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Binding {
    pub modifiers: Vec<Modifier>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "keysym")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `key` as cosmic-settings-config's `sym::{serialize, deserialize}` do it:
/// a bare keysym-name string. `None` is skipped on write and never read.
mod keysym {
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(key: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(key.as_deref().unwrap_or("None"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        String::deserialize(d).map(Some)
    }
}

pub fn custom_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom")
}

/// `exe` quoted for `/bin/sh -c` if it needs it.
fn shell_word(exe: &str) -> String {
    if exe.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        exe.to_string()
    } else {
        format!("'{}'", exe.replace('\'', r"'\''"))
    }
}

/// The bindings we want, in a stable order: focus 1–9, next, prev.
pub fn desired(cfg: &ShortcutsConfig, exe: &str) -> Vec<(Binding, String)> {
    let exe = shell_word(exe);
    let bind = |key: &str, description: String| Binding {
        modifiers: cfg.focus_prefix.clone(),
        key: Some(key.to_string()),
        keycode: None,
        description: Some(description),
    };
    let mut out: Vec<(Binding, String)> = (1..=9)
        .map(|n| (bind(&n.to_string(), format!("Yutani: focus client {n}")), format!("{exe} focus {n}")))
        .collect();
    out.push((bind(&cfg.next, "Yutani: next client".into()), format!("{exe} next")));
    out.push((bind(&cfg.prev, "Yutani: previous client".into()), format!("{exe} prev")));
    out
}

/// True for `Spawn("<something>yutani <args>")` where the command's first
/// shell word is `yutani` or a path ending in `/yutani`.
pub fn is_ours(action_ron: &str) -> bool {
    let Ok(action) = RawValue::from_ron(action_ron.trim()) else { return false };
    let Ok(SpawnOnly::Spawn(cmd)) = action.into_rust::<SpawnOnly>() else { return false };
    let first = cmd.trim_start();
    let word = if let Some(rest) = first.strip_prefix('\'') {
        rest.split('\'').next().unwrap_or("")
    } else {
        first.split_whitespace().next().unwrap_or("")
    };
    word == "yutani" || word.ends_with("/yutani")
}

/// Just enough of cosmic's `Action` to recognise `Spawn`.
#[derive(Deserialize)]
enum SpawnOnly {
    Spawn(String),
}

type Entries = BTreeMap<Binding, Box<RawValue>>;

fn parse(existing: &str) -> anyhow::Result<Entries> {
    if existing.trim().is_empty() {
        return Ok(Entries::new());
    }
    let entries: Entries =
        ron::from_str(existing).context("cannot parse the existing custom shortcuts file; not touching it")?;
    // `RawValue` keeps the value's surrounding whitespace from the source
    // text; drop it so re-rendered foreign entries line up with ours.
    entries
        .into_iter()
        .map(|(binding, action)| Ok((binding, RawValue::from_boxed_ron(action.get_ron().trim().into())?)))
        .collect()
}

fn render(entries: &Entries) -> anyhow::Result<String> {
    // One binding per line, exactly as cosmic-settings writes the file:
    // `(modifiers: [Ctrl, Alt], key: "1", description: Some("…")): Spawn("…"),`.
    let config = ron::ser::PrettyConfig::new().depth_limit(1).struct_names(false);
    Ok(ron::ser::to_string_pretty(entries, config)?)
}

/// `existing` with every yutani entry removed and `ours` added.
pub fn merge(existing: &str, ours: &[(Binding, String)]) -> anyhow::Result<String> {
    let mut entries = parse(existing)?;
    entries.retain(|_, action| !is_ours(action.get_ron()));
    for (binding, cmd) in ours {
        let action = ron::to_string(&SpawnOut::Spawn(cmd.clone()))?;
        entries.insert(binding.clone(), RawValue::from_boxed_ron(action.into_boxed_str())?);
    }
    render(&entries)
}

#[derive(Serialize)]
enum SpawnOut {
    Spawn(String),
}

/// `existing` with every yutani entry removed.
pub fn strip(existing: &str) -> anyhow::Result<String> {
    let mut entries = parse(existing)?;
    entries.retain(|_, action| !is_ours(action.get_ron()));
    render(&entries)
}

fn read_existing(path: &std::path::Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Write the file in place. Deliberately *not* tmp+rename: cosmic-config's
/// watcher (which cosmic-comp uses to reload shortcuts) ignores paired
/// rename events and keys changes by the touched file's name, so a rename
/// from `custom.tmp` never triggers a reload of `custom`. A direct write
/// raises Create/Modify events on `custom` itself.
fn write_in_place(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    let dir = path.parent().context("shortcuts path has no parent")?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Write our bindings; returns how many were written.
pub fn install(cfg: &ShortcutsConfig) -> anyhow::Result<usize> {
    let exe = std::env::current_exe().context("current_exe")?;
    let ours = desired(cfg, &exe.to_string_lossy());
    let path = custom_path();
    let merged = merge(&read_existing(&path)?, &ours)?;
    write_in_place(&path, &merged)?;
    Ok(ours.len())
}

/// Remove our bindings; returns how many were removed.
pub fn uninstall() -> anyhow::Result<usize> {
    let path = custom_path();
    let existing = read_existing(&path)?;
    let before = parse(&existing)?.len();
    let stripped = strip(&existing)?;
    let after = parse(&stripped)?.len();
    write_in_place(&path, &stripped)?;
    Ok(before - after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::Modifier;

    fn cfg() -> ShortcutsConfig {
        ShortcutsConfig::default()
    }

    #[test]
    fn desired_has_eleven_entries_with_absolute_commands() {
        let d = desired(&cfg(), "/opt/yutani/bin/yutani");
        assert_eq!(d.len(), 11);
        assert_eq!(d[0].0, Binding { modifiers: vec![Modifier::Ctrl, Modifier::Alt], key: Some("1".into()), keycode: None, description: Some("Yutani: focus client 1".into()) });
        assert_eq!(d[0].1, "/opt/yutani/bin/yutani focus 1");
        assert_eq!(d[8].1, "/opt/yutani/bin/yutani focus 9");
        assert_eq!(d[9].0.key.as_deref(), Some("Right"));
        assert_eq!(d[9].1, "/opt/yutani/bin/yutani next");
        assert_eq!(d[10].1, "/opt/yutani/bin/yutani prev");
    }

    #[test]
    fn is_ours_matches_spawn_of_yutani_only() {
        assert!(is_ours(r#"Spawn("yutani focus 1")"#));
        assert!(is_ours(r#"Spawn("/home/d/Yutani/target/debug/yutani next")"#));
        assert!(is_ours(r#" Spawn( "/usr/bin/yutani prev" ) "#));
        assert!(!is_ours(r#"Spawn("cosmic-term")"#));
        assert!(!is_ours(r#"Spawn("notyutani focus 1")"#));
        assert!(!is_ours(r#"Spawn("/usr/bin/yutani-helper x")"#));
        assert!(!is_ours("Close"));
    }

    const FOREIGN: &str = r#"{
    (modifiers: [Super], key: "t", description: Some("Terminal")): Spawn("cosmic-term"),
    (modifiers: [Super, Shift], key: "q"): Close,
}"#;

    #[test]
    fn merge_keeps_foreign_entries_and_adds_ours() {
        let out = merge(FOREIGN, &desired(&cfg(), "yutani")).unwrap();
        assert!(out.contains(r#"Spawn("cosmic-term")"#));
        assert!(out.contains("Close"));
        assert!(out.contains(r#"(modifiers: [Ctrl, Alt], key: "1", description: Some("Yutani: focus client 1")): Spawn("yutani focus 1")"#));
        assert!(out.contains(r#"key: "Right""#));
        // Parses back as a RON map with 13 entries.
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 13);
    }

    #[test]
    fn merge_replaces_stale_yutani_entries_even_on_other_keys() {
        let stale = r#"{ (modifiers: [Super], key: "F1"): Spawn("/old/yutani focus 1"), (modifiers: [Super], key: "t"): Spawn("cosmic-term") }"#;
        let out = merge(stale, &desired(&cfg(), "yutani")).unwrap();
        assert!(!out.contains("/old/yutani"));
        assert!(out.contains(r#"Spawn("cosmic-term")"#));
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 12);
    }

    #[test]
    fn merge_and_strip_handle_missing_or_empty_file() {
        let out = merge("", &desired(&cfg(), "yutani")).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 11);
        assert_eq!(strip("").unwrap().trim(), "{}");
        let back = strip(&out).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&back).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn strip_keeps_only_foreign_entries() {
        let mixed = merge(FOREIGN, &desired(&cfg(), "yutani")).unwrap();
        let back = strip(&mixed).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&back).unwrap();
        assert_eq!(map.len(), 2);
        assert!(back.contains("cosmic-term"));
    }

    #[test]
    fn merge_rejects_unparseable_input_instead_of_clobbering() {
        assert!(merge("{ this is not ron", &desired(&cfg(), "yutani")).is_err());
    }

    #[test]
    fn shell_quotes_paths_with_spaces() {
        let d = desired(&cfg(), "/home/me/My Apps/yutani");
        assert_eq!(d[0].1, "'/home/me/My Apps/yutani' focus 1");
        assert!(is_ours(r#"Spawn("'/home/me/My Apps/yutani' focus 1")"#));
    }
}
