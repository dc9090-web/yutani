//! `yutani shortcuts install|uninstall`: our entries in COSMIC's custom
//! keyboard-shortcuts file (spec §7). Only entries whose action spawns
//! `yutani` are ever added, replaced or removed; everything else in the
//! file is preserved as the RON text it was.

use anyhow::Context as _;
use ron::value::RawValue;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
        unquote_word(rest)
    } else {
        first.split_whitespace().next().unwrap_or("").to_string()
    };
    word == "yutani" || word.ends_with("/yutani")
}

/// Undo `shell_word`'s single-quoting: collect characters up to the
/// unescaped closing `'`, treating an embedded `'` as a literal character
/// rather than the end of the word.
///
/// `shell_word` encodes an embedded `'` as the 4-byte sequence `'\''`
/// (close-quote, backslash, escaped-quote, reopen-quote). That is what a
/// RON file we wrote ourselves round-trips back to (RON escapes the `\` on
/// write and restores it on read), so the first branch below handles the
/// common case directly. But some RON decoders treat `\'` itself as an
/// escape for a literal `'`, in which case the same source text decodes to
/// a bare run of 3 quote characters instead (the backslash is consumed) —
/// the second branch undoes that: an odd run of more than one quote is a
/// close/escaped-quote/reopen group repeated (one literal `'` per pair),
/// while a lone quote is a genuine closing quote.
fn unquote_word(rest: &str) -> String {
    let chars: Vec<char> = rest.chars().collect();
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' {
            if chars[i..].starts_with(&['\'', '\\', '\'', '\'']) {
                word.push('\'');
                i += 4;
                continue;
            }
            let run = chars[i..].iter().take_while(|&&c| c == '\'').count();
            if run == 1 || run % 2 == 0 {
                break;
            }
            for _ in 0..(run - 1) / 2 {
                word.push('\'');
            }
            i += run;
        } else {
            word.push(chars[i]);
            i += 1;
        }
    }
    word
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

/// cosmic-comp identifies a binding by its (modifiers, key-or-keycode)
/// chord alone — `description` plays no part, and modifier order doesn't
/// matter (`[Alt, Ctrl]` == `[Ctrl, Alt]`).
type Chord = (BTreeSet<Modifier>, Option<String>, Option<u32>);

fn chord_of(binding: &Binding) -> Chord {
    (binding.modifiers.iter().copied().collect(), binding.key.clone(), binding.keycode)
}

fn chord_display((modifiers, key, keycode): &Chord) -> String {
    let mods = modifiers.iter().map(|m| format!("{m:?}")).collect::<Vec<_>>().join("+");
    match (key, keycode) {
        (Some(k), _) => format!("{mods}+{k}"),
        (None, Some(kc)) => format!("{mods}+keycode {kc}"),
        (None, None) => mods,
    }
}

/// `existing` with every yutani entry removed and `ours` added, except any
/// of `ours` whose chord collides with a surviving foreign entry — writing
/// both would leave two entries in the file keyed on the same chord, and
/// cosmic-comp (and RON map parsing generally) only keeps the last one, so
/// one binding would silently vanish. Colliding entries are skipped
/// instead; the second return value describes each one skipped.
pub fn merge(existing: &str, ours: &[(Binding, String)]) -> anyhow::Result<(String, Vec<String>)> {
    let mut entries = parse(existing)?;
    entries.retain(|_, action| !is_ours(action.get_ron()));

    let foreign_chords: BTreeMap<Chord, String> =
        entries.iter().map(|(binding, action)| (chord_of(binding), action.get_ron().to_string())).collect();

    let mut skipped = Vec::new();
    for (binding, cmd) in ours {
        let chord = chord_of(binding);
        if let Some(foreign_action) = foreign_chords.get(&chord) {
            skipped.push(format!("{}: already bound to {foreign_action}", chord_display(&chord)));
            continue;
        }
        let action = ron::to_string(&SpawnOut::Spawn(cmd.clone()))?;
        entries.insert(binding.clone(), RawValue::from_boxed_ron(action.into_boxed_str())?);
    }
    Ok((render(&entries)?, skipped))
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

/// Write our bindings; returns how many were actually written (fewer than
/// `desired`'s count when some collided with a foreign binding and were
/// skipped — see `merge`).
pub fn install(cfg: &ShortcutsConfig) -> anyhow::Result<usize> {
    let exe = std::env::current_exe().context("current_exe")?;
    let ours = desired(cfg, &exe.to_string_lossy());
    let path = custom_path();
    let (merged, skipped) = merge(&read_existing(&path)?, &ours)?;
    for reason in &skipped {
        eprintln!("warning: skipped {reason}");
    }
    write_in_place(&path, &merged)?;
    Ok(ours.len() - skipped.len())
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
        let (out, skipped) = merge(FOREIGN, &desired(&cfg(), "yutani")).unwrap();
        assert!(skipped.is_empty());
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
        let (out, skipped) = merge(stale, &desired(&cfg(), "yutani")).unwrap();
        assert!(skipped.is_empty());
        assert!(!out.contains("/old/yutani"));
        assert!(out.contains(r#"Spawn("cosmic-term")"#));
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 12);
    }

    #[test]
    fn merge_and_strip_handle_missing_or_empty_file() {
        let (out, skipped) = merge("", &desired(&cfg(), "yutani")).unwrap();
        assert!(skipped.is_empty());
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 11);
        assert_eq!(strip("").unwrap().trim(), "{}");
        let back = strip(&out).unwrap();
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&back).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn strip_keeps_only_foreign_entries() {
        let (mixed, _) = merge(FOREIGN, &desired(&cfg(), "yutani")).unwrap();
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
    fn merge_skips_chords_already_bound_by_foreign_entries() {
        // `next`'s chord (Ctrl+Alt+Right) collides with a foreign entry that
        // lists its modifiers in the opposite order and under a different
        // description — normalisation must still catch it. A keycode-only
        // foreign entry sharing the same modifiers must not collide with
        // anything of ours (we only ever bind by key, never by keycode).
        let foreign = r#"{
    (modifiers: [Alt, Ctrl], key: "Right", description: Some("mine")): Spawn("foo"),
    (modifiers: [Ctrl, Alt], keycode: Some(24)): Spawn("bar"),
}"#;
        let (out, skipped) = merge(foreign, &desired(&cfg(), "yutani")).unwrap();
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("Ctrl+Alt+Right"), "{skipped:?}");
        assert!(out.contains(r#"Spawn("foo")"#));
        assert!(out.contains(r#"Spawn("bar")"#));
        assert!(!out.contains(r#"Spawn("yutani next")"#));
        // 2 foreign entries + 10 of ours (11 desired, minus the 1 skipped).
        let map: std::collections::BTreeMap<Binding, Box<ron::value::RawValue>> = ron::from_str(&out).unwrap();
        assert_eq!(map.len(), 12);
    }

    #[test]
    fn shell_quotes_paths_with_spaces() {
        let d = desired(&cfg(), "/home/me/My Apps/yutani");
        assert_eq!(d[0].1, "'/home/me/My Apps/yutani' focus 1");
        assert!(is_ours(r#"Spawn("'/home/me/My Apps/yutani' focus 1")"#));
    }

    #[test]
    fn is_ours_unquotes_embedded_apostrophes_in_exe_path() {
        // shell_word encodes an embedded `'` as `'\''`; is_ours must undo
        // that instead of stopping at the first `'` it finds.
        let exe = "/home/me/it's/yutani";
        let d = desired(&cfg(), exe);
        assert_eq!(d[9].1, r"'/home/me/it'\''s/yutani' next");
        assert!(is_ours(r#"Spawn("'/home/me/it'\''s/yutani' focus 1")"#));
    }
}
