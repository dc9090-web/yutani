//! Thumbnail geometry (snapping) and persisted per-character positions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThumbPos {
    pub output: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub pinned: bool,
}

/// Where a thumbnail whose character name is not known yet is put (spec
/// §4/§9). An empty `output` means "the client's own output, else the
/// primary one".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Anchor {
    pub output: String,
    pub x: i32,
    pub y: i32,
}

impl Default for Anchor {
    fn default() -> Self {
        Self { output: String::new(), x: 40, y: 40 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Keyed by character name.
    pub thumbs: BTreeMap<String, ThumbPos>,
    /// Layout order by character name (spec §9). A character listed here
    /// ranks by its place in the list; anyone else follows by label. This
    /// is what makes a named layout able to fix the **dock** arrangement,
    /// where no positions are stored.
    pub order: Vec<String>,
    /// Where the next unnamed client's thumbnail goes.
    pub new_client_anchor: Anchor,
}

fn round_to(v: i32, grid: i32) -> i32 {
    ((v as f64 / grid as f64).round() as i32) * grid
}

/// Snap a dragged rect's top-left. Edge snapping (flush against, or aligned
/// with, another rect's edges within `edge_threshold`) takes precedence over
/// the grid, per axis. Edge distances are measured from the grid-snapped
/// coordinate (when grid snapping is on), so a thumbnail that lands on a
/// grid line near a neighbour still snaps flush; the effective edge
/// threshold is therefore up to `grid/2 + edge_threshold`.
pub fn snap(rect: Rect, others: &[Rect], grid: Option<i32>, edge_threshold: Option<i32>) -> (i32, i32) {
    let mut x = rect.x;
    let mut y = rect.y;
    if let Some(g) = grid.filter(|g| *g > 0) {
        x = round_to(x, g);
        y = round_to(y, g);
    }
    if let Some(t) = edge_threshold.filter(|t| *t > 0) {
        let mut best_x: Option<(i32, i32)> = None; // (distance, snapped x)
        let mut best_y: Option<(i32, i32)> = None;
        for o in others {
            // candidate x values: flush right-of-o, flush left-of-o, aligned left edges
            for cand in [o.x + o.w, o.x - rect.w, o.x] {
                let d = (x - cand).abs();
                if d <= t && best_x.map_or(true, |(bd, _)| d < bd) {
                    best_x = Some((d, cand));
                }
            }
            for cand in [o.y + o.h, o.y - rect.h, o.y] {
                let d = (y - cand).abs();
                if d <= t && best_y.map_or(true, |(bd, _)| d < bd) {
                    best_y = Some((d, cand));
                }
            }
        }
        if let Some((_, sx)) = best_x {
            x = sx;
        }
        if let Some((_, sy)) = best_y {
            y = sy;
        }
    }
    (x, y)
}

/// Successive unnamed clients stack this far down-right of the anchor.
pub const STACK_STEP: i32 = 24;
/// Upper bound on the recorded `order`, so a long history of characters
/// cannot grow the file without end.
pub const MAX_ORDER: usize = 64;
/// Longest layout name, in characters.
pub const MAX_NAME: usize = 64;
/// Reserved stem: `current.ron` is the auto-saved layout, not a named one.
pub const CURRENT: &str = "current";

/// The anchor a saved layout records: the top-left-most saved thumbnail
/// (smallest y, then x, then connector name), on the output that holds it.
/// With nothing saved, the default (40, 40) on no particular output.
pub fn derive_anchor(thumbs: &BTreeMap<String, ThumbPos>) -> Anchor {
    thumbs
        .values()
        .min_by(|a, b| (a.y, a.x, &a.output).cmp(&(b.y, b.x, &b.output)))
        .map(|t| Anchor { output: t.output.clone(), x: t.x, y: t.y })
        .unwrap_or_default()
}

/// The `order` a save records: the previously recorded order is kept
/// unchanged — a character who is merely logged out keeps their slot —
/// with any live character not already in that recorded order appended
/// after it, in live order. Live characters that *are* already recorded
/// stay at their recorded slot rather than jumping to the front.
/// Deduplicated and capped at `cap`.
pub fn merge_order(live: &[String], previous: &[String], cap: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(live.len() + previous.len());
    for name in previous {
        if !out.iter().any(|n| n == name) {
            out.push(name.clone());
        }
    }
    for name in live {
        if !previous.iter().any(|n| n == name) && !out.iter().any(|n| n == name) {
            out.push(name.clone());
        }
    }
    out.truncate(cap);
    out
}

/// Where an unnamed client's thumbnail goes (spec §4): the anchor, then
/// `step` px down-right per occupied slot. "Occupied" means within half a
/// `step` on **both** axes of a shown thumbnail or of a saved position, so
/// a new client no longer lands on top of a character's saved spot.
///
/// Half a step, not a full one: candidate slots are themselves only `step`
/// apart on the diagonal, so a full-`step` collision radius lets a single
/// occupied point fall within range of two consecutive candidates at once
/// and block both — the caller would then skip past a slot that was
/// actually free. Half a step is the largest radius that cannot straddle
/// two neighbouring slots that way.
pub fn stacked_position(anchor: (i32, i32), taken: &[(i32, i32)], step: i32) -> (i32, i32) {
    let step = step.max(1);
    let radius = (step / 2).max(1);
    (0..)
        .map(|k| (anchor.0 + k * step, anchor.1 + k * step))
        .find(|&(x, y)| !taken.iter().any(|&(tx, ty)| (x - tx).abs() < radius && (y - ty).abs() < radius))
        .expect("the candidate walks away from a finite set of occupied slots")
}

/// Spec §9: a thumbnail whose saved connector is not connected goes to the
/// primary output — the first one we know of — at the same x/y. `None`
/// when no output is connected at all.
pub fn resolve_output<'a>(saved: &str, connected: &'a [String]) -> Option<&'a str> {
    connected
        .iter()
        .find(|o| o.as_str() == saved)
        .or_else(|| connected.first())
        .map(String::as_str)
}

/// Where a live thumbnail goes when a layout is applied, and whether its
/// layer surface has to be destroyed and recreated: a layer surface is
/// bound to one output for life, so a saved position on another output is
/// a recreate, not a `set_margin`.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub pinned: bool,
    pub recreate: bool,
}

pub fn placement(saved: &ThumbPos, connected: &[String], current_output: &str) -> Option<Placement> {
    let output = resolve_output(&saved.output, connected)?;
    Some(Placement {
        recreate: output != current_output,
        output: output.to_string(),
        x: saved.x,
        y: saved.y,
        pinned: saved.pinned,
    })
}

/// Validate a layout name as a file stem; `Ok` is the trimmed name. The
/// error text is what the IPC `layout` request sends back after `err `.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("layout name is empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("layout name is longer than {MAX_NAME} characters"));
    }
    if name.contains(['/', '\\']) {
        return Err("layout name cannot contain / or \\".into());
    }
    if name.chars().any(char::is_control) {
        return Err("layout name cannot contain control characters".into());
    }
    if name == "." || name == ".." {
        return Err("layout name cannot be . or ..".into());
    }
    if name.eq_ignore_ascii_case(CURRENT) {
        return Err(format!("{CURRENT:?} is reserved for the auto-saved layout"));
    }
    Ok(name.to_string())
}

pub fn layouts_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("yutani").join("layouts")
}

pub fn current_path() -> PathBuf {
    layouts_dir().join("current.ron")
}

pub fn named_path_in(dir: &Path, name: &str) -> Result<PathBuf, String> {
    Ok(dir.join(format!("{}.ron", validate_name(name)?)))
}

pub fn named_path(name: &str) -> Result<PathBuf, String> {
    named_path_in(&layouts_dir(), name)
}

/// Saved layout names, sorted case-insensitively. `current` is not one of
/// them, and neither is any file whose stem would fail `validate_name`.
pub fn list_names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| validate_name(s).is_ok_and(|v| v == *s))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn list_names() -> Vec<String> {
    list_names_in(&layouts_dir())
}

pub fn delete_named_in(dir: &Path, name: &str) -> Result<(), String> {
    let path = named_path_in(dir, name)?;
    std::fs::remove_file(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("no such layout {name}"),
        _ => format!("cannot delete {}: {e}", path.display()),
    })
}

pub fn delete_named(name: &str) -> Result<(), String> {
    delete_named_in(&layouts_dir(), name)
}

/// Rename a saved layout, refusing to overwrite an existing one.
pub fn rename_named_in(dir: &Path, from: &str, to: &str) -> Result<(), String> {
    let from_path = named_path_in(dir, from)?;
    let to_path = named_path_in(dir, to)?;
    if from_path == to_path {
        return Ok(());
    }
    if !from_path.exists() {
        return Err(format!("no such layout {from}"));
    }
    if to_path.exists() {
        return Err(format!("layout {} already exists", validate_name(to)?));
    }
    std::fs::rename(&from_path, &to_path).map_err(|e| format!("cannot rename {}: {e}", from_path.display()))
}

pub fn rename_named(from: &str, to: &str) -> Result<(), String> {
    rename_named_in(&layouts_dir(), from, to)
}

/// See [`Layout::save_gate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveGate {
    /// Not poisoned: write normally.
    Proceed,
    /// Poisoned and already warned about it: refuse without logging again.
    RefuseSilently,
    /// Poisoned and not yet warned: refuse, and this is the call that logs
    /// the one-time warning.
    RefuseAndWarn,
}

impl Layout {
    pub fn load() -> Self {
        Self::load_from(&current_path())
    }

    /// Lenient load: missing or unreadable/unparseable file all become
    /// `Layout::default()`, with a warning for the latter two. The file
    /// itself is never touched. Callers that need to tell "unparseable"
    /// apart from "missing" — so they can refuse to overwrite a poisoned
    /// file — use [`Layout::try_load_from`] instead.
    pub fn load_from(path: &Path) -> Self {
        match Self::try_load_from(path) {
            Ok(Some(layout)) => layout,
            Ok(None) => Self::default(),
            Err(msg) => {
                tracing::warn!("{msg}; starting with an empty layout");
                Self::default()
            }
        }
    }

    /// Strict load, so a caller can tell a hand-edited `current.ron` with a
    /// typo apart from one that simply doesn't exist yet: `Ok(None)` = no
    /// file, `Ok(Some(layout))` = it parsed, `Err` = it exists but cannot be
    /// read or parsed — in which case nothing may overwrite it (spec §10:
    /// "never overwrite the offending file").
    pub fn try_load_from(path: &Path) -> Result<Option<Layout>, String> {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
            Ok(text) => ron::from_str::<Layout>(&text)
                .map(Some)
                .map_err(|e| format!("cannot parse {}: {e}", path.display())),
        }
    }

    pub fn try_load() -> Result<Option<Layout>, String> {
        Self::try_load_from(&current_path())
    }

    /// What `save_current_layout` does about a save attempt, given whether
    /// `current.ron` is known to be poisoned (unparseable on disk) and
    /// whether the one-time warning about that has already been logged.
    /// Never overwrites a poisoned file (spec §10); warns about it exactly
    /// once, not on every add/remove event that would otherwise trigger a
    /// save.
    pub fn save_gate(poisoned: bool, already_warned: bool) -> SaveGate {
        match (poisoned, already_warned) {
            (false, _) => SaveGate::Proceed,
            (true, true) => SaveGate::RefuseSilently,
            (true, false) => SaveGate::RefuseAndWarn,
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&current_path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        super::write_atomic(path, &text)?;
        Ok(())
    }

    /// Read a named layout. `Err` is the text the IPC `layout` request
    /// sends back after `err `.
    pub fn load_named_in(dir: &Path, name: &str) -> Result<Layout, String> {
        let path = named_path_in(dir, name)?;
        let text = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("no such layout {name}"),
            _ => format!("cannot read {}: {e}", path.display()),
        })?;
        ron::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))
    }

    pub fn load_named(name: &str) -> Result<Layout, String> {
        Self::load_named_in(&layouts_dir(), name)
    }

    pub fn save_named_in(&self, dir: &Path, name: &str) -> Result<(), String> {
        let path = named_path_in(dir, name)?;
        self.save_to(&path).map_err(|e| format!("cannot save {}: {e:#}", path.display()))
    }

    pub fn save_named(&self, name: &str) -> Result<(), String> {
        self.save_named_in(&layouts_dir(), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32) -> Rect {
        Rect { x, y, w: 100, h: 60 }
    }

    #[test]
    fn no_snapping_returns_input() {
        assert_eq!(snap(r(37, 51), &[], None, None), (37, 51));
    }

    #[test]
    fn grid_snaps_to_nearest_multiple() {
        assert_eq!(snap(r(37, 51), &[], Some(32), None), (32, 64));
        assert_eq!(snap(r(47, 15), &[], Some(32), None), (32, 0));
        assert_eq!(snap(r(49, 17), &[], Some(32), None), (64, 32));
    }

    #[test]
    fn edge_snaps_flush_against_neighbour_within_threshold() {
        let other = r(200, 40); // occupies x 200..300, y 40..100
        // our right edge (x+100) near other's left edge (200)
        assert_eq!(snap(r(93, 40), &[other], None, Some(12)), (100, 40));
        // our left edge near other's right edge (300)
        assert_eq!(snap(r(308, 40), &[other], None, Some(12)), (300, 40));
        // our top near other's bottom (100)
        assert_eq!(snap(r(200, 109), &[other], None, Some(12)), (200, 100));
        // our bottom (y+60) near other's top (40)
        assert_eq!(snap(r(200, -25), &[other], None, Some(12)), (200, -20));
        // beyond threshold: untouched
        assert_eq!(snap(r(80, 40), &[other], None, Some(12)), (80, 40));
    }

    #[test]
    fn edge_snapping_also_aligns_same_side_edges() {
        let other = r(200, 40);
        // our left edge near other's left edge
        assert_eq!(snap(r(205, 300), &[other], None, Some(12)), (200, 300));
        // our top near other's top
        assert_eq!(snap(r(500, 45), &[other], None, Some(12)), (500, 40));
    }

    #[test]
    fn edge_snap_wins_over_grid_when_both_enabled() {
        let other = r(200, 40);
        assert_eq!(snap(r(93, 51), &[other], Some(32), Some(12)), (100, 64));
    }

    #[test]
    fn edge_snap_picks_the_nearest_of_several_neighbours() {
        let a = r(200, 40);  // right edge 300
        let b = r(400, 40);  // left edge 400
        // our left edge (305) is 5 from a's right, our right edge (405) is 5 from b's left → tie → first wins (a)
        assert_eq!(snap(r(305, 40), &[a, b], None, Some(12)), (300, 40));
        // clearly nearer to b
        assert_eq!(snap(r(296, 300), &[a, b], None, Some(12)), (300, 300));
        assert_eq!(snap(r(309, 300), &[b, a], None, Some(12)), (300, 300));
    }

    #[test]
    fn layout_round_trips_and_missing_is_empty() {
        let dir = std::env::temp_dir().join(format!("yutani-layout-{}", std::process::id()));
        let path = dir.join("current.ron");
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), ThumbPos { output: "DP-1".into(), x: 40, y: 40, pinned: true });
        l.save_to(&path).unwrap();
        assert_eq!(Layout::load_from(&path), l);
        assert_eq!(Layout::load_from(&dir.join("nope.ron")), Layout::default());
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(Layout::load_from(&path), Layout::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn try_load_from_distinguishes_missing_from_broken() {
        let dir = tmpdir("layout-try-load");
        let path = dir.join("current.ron");

        // Missing: Ok(None), and the file is not created by the check.
        assert_eq!(Layout::try_load_from(&path), Ok(None));
        assert!(!path.exists());

        // Unparseable: Err, and the file is left untouched.
        std::fs::write(&path, "garbage").unwrap();
        assert!(Layout::try_load_from(&path).unwrap_err().contains("cannot parse"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage");

        // Valid: Ok(Some(..)).
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), pos("DP-1", 40, 40));
        l.save_to(&path).unwrap();
        assert_eq!(Layout::try_load_from(&path), Ok(Some(l)));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_gate_writes_normally_unless_poisoned_and_warns_at_most_once() {
        assert_eq!(Layout::save_gate(false, false), SaveGate::Proceed);
        assert_eq!(Layout::save_gate(false, true), SaveGate::Proceed);
        assert_eq!(Layout::save_gate(true, false), SaveGate::RefuseAndWarn);
        assert_eq!(Layout::save_gate(true, true), SaveGate::RefuseSilently);
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pos(output: &str, x: i32, y: i32) -> ThumbPos {
        ThumbPos { output: output.into(), x, y, pinned: false }
    }

    #[test]
    fn layout_round_trips_with_order_and_anchor() {
        let dir = tmpdir("layout-rt");
        let path = dir.join("current.ron");
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), ThumbPos { output: "DP-1".into(), x: 40, y: 40, pinned: true });
        l.thumbs.insert("Kel Draven".into(), pos("DP-1", 380, 40));
        l.order = vec!["Aria Vex".into(), "Kel Draven".into()];
        l.new_client_anchor = Anchor { output: "DP-1".into(), x: 40, y: 40 };
        l.save_to(&path).unwrap();
        assert_eq!(Layout::load_from(&path), l);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_layout_file_without_the_new_fields_still_loads() {
        // Every `current.ron` written before this plan has only `thumbs`.
        let l: Layout = ron::from_str(r#"(thumbs: {"Aria Vex": (output: "DP-1", x: 40, y: 40)})"#).unwrap();
        assert_eq!(l.thumbs.len(), 1);
        assert!(l.order.is_empty());
        assert_eq!(l.new_client_anchor, Anchor::default());
        assert_eq!(Anchor::default(), Anchor { output: String::new(), x: 40, y: 40 });
    }

    #[test]
    fn anchor_is_the_top_left_most_saved_thumbnail() {
        let mut thumbs = BTreeMap::new();
        assert_eq!(derive_anchor(&thumbs), Anchor::default());
        thumbs.insert("Kel".to_string(), pos("DP-1", 380, 40));
        thumbs.insert("Aria".to_string(), pos("DP-1", 40, 40));
        thumbs.insert("Zoe".to_string(), pos("DP-2", 10, 900));
        assert_eq!(derive_anchor(&thumbs), Anchor { output: "DP-1".into(), x: 40, y: 40 });
    }

    #[test]
    fn merge_order_keeps_recorded_slots_and_appends_new_live_names() {
        let live = ["Kel".to_string(), "Aria".to_string()];
        let previous = ["Aria".to_string(), "Zoe".to_string()];
        // Aria keeps her recorded slot (does not jump to the front just
        // because she's live); Zoe (logged out) keeps hers too; Kel (live,
        // never recorded) is appended after.
        assert_eq!(merge_order(&live, &previous, MAX_ORDER), vec!["Aria", "Zoe", "Kel"]);
        // The cap bounds the file even after years of characters: it drops
        // from the newly-appended end, not out of the recorded order.
        assert_eq!(merge_order(&live, &previous, 2), vec!["Aria", "Zoe"]);
        assert!(merge_order(&[], &[], MAX_ORDER).is_empty());
    }

    #[test]
    fn stacked_position_steps_down_right_past_occupied_slots() {
        assert_eq!(stacked_position((40, 40), &[], 24), (40, 40));
        assert_eq!(stacked_position((40, 40), &[(40, 40)], 24), (64, 64));
        assert_eq!(stacked_position((40, 40), &[(40, 40), (64, 64)], 24), (88, 88));
        // A saved position a few pixels away still counts as occupied, so a
        // new client no longer lands on top of a character's saved spot.
        assert_eq!(stacked_position((40, 40), &[(50, 45)], 24), (64, 64));
        // Far away on one axis only: not a collision.
        assert_eq!(stacked_position((40, 40), &[(400, 45)], 24), (40, 40));
    }

    #[test]
    fn resolve_output_falls_back_to_the_primary_when_the_connector_is_gone() {
        let connected = ["DP-1".to_string(), "HDMI-A-1".to_string()];
        assert_eq!(resolve_output("HDMI-A-1", &connected), Some("HDMI-A-1"));
        assert_eq!(resolve_output("DP-9", &connected), Some("DP-1"));
        assert_eq!(resolve_output("", &connected), Some("DP-1"));
        assert_eq!(resolve_output("DP-1", &[]), None);
    }

    #[test]
    fn placement_recreates_only_when_the_output_changes() {
        let connected = ["DP-1".to_string(), "HDMI-A-1".to_string()];
        let saved = ThumbPos { output: "HDMI-A-1".into(), x: 12, y: 34, pinned: true };
        assert_eq!(
            placement(&saved, &connected, "HDMI-A-1"),
            Some(Placement { output: "HDMI-A-1".into(), x: 12, y: 34, pinned: true, recreate: false })
        );
        assert_eq!(
            placement(&saved, &connected, "DP-1"),
            Some(Placement { output: "HDMI-A-1".into(), x: 12, y: 34, pinned: true, recreate: true })
        );
        // Disconnected connector: the primary output at the same x/y (spec §9).
        let saved = ThumbPos { output: "DP-9".into(), x: 12, y: 34, pinned: false };
        assert_eq!(
            placement(&saved, &connected, "DP-1"),
            Some(Placement { output: "DP-1".into(), x: 12, y: 34, pinned: false, recreate: false })
        );
        assert_eq!(placement(&saved, &[], "DP-1"), None);
    }

    #[test]
    fn layout_names_must_be_plain_file_stems() {
        assert_eq!(validate_name("  pvp fleet  ").unwrap(), "pvp fleet");
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("a\nb").is_err());
        assert!(validate_name("current").is_err());
        assert!(validate_name("CURRENT").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME)).is_ok());
        assert!(validate_name(&"x".repeat(MAX_NAME + 1)).is_err());
    }

    #[test]
    fn named_layouts_save_list_load_rename_and_delete() {
        let dir = tmpdir("layout-named");
        let mut l = Layout::default();
        l.thumbs.insert("Aria Vex".into(), pos("DP-1", 40, 40));
        l.order = vec!["Aria Vex".into()];

        assert_eq!(list_names_in(&dir), Vec::<String>::new());
        assert_eq!(Layout::load_named_in(&dir, "pvp").unwrap_err(), "no such layout pvp");

        l.save_named_in(&dir, "pvp").unwrap();
        l.save_named_in(&dir, "Mining").unwrap();
        // `current.ron` is not a named layout.
        Layout::default().save_to(&dir.join("current.ron")).unwrap();
        assert_eq!(list_names_in(&dir), vec!["Mining".to_string(), "pvp".to_string()]);
        assert_eq!(Layout::load_named_in(&dir, "pvp").unwrap(), l);

        rename_named_in(&dir, "pvp", "pvp fleet").unwrap();
        assert_eq!(list_names_in(&dir), vec!["Mining".to_string(), "pvp fleet".to_string()]);
        assert!(rename_named_in(&dir, "pvp", "x").unwrap_err().contains("no such layout"));
        assert!(rename_named_in(&dir, "pvp fleet", "Mining").unwrap_err().contains("already exists"));
        assert!(rename_named_in(&dir, "pvp fleet", "a/b").is_err());

        delete_named_in(&dir, "Mining").unwrap();
        assert_eq!(list_names_in(&dir), vec!["pvp fleet".to_string()]);
        assert_eq!(delete_named_in(&dir, "Mining").unwrap_err(), "no such layout Mining");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_is_atomic_and_leaves_no_temporary_behind() {
        let dir = tmpdir("layout-atomic");
        let path = dir.join("current.ron");
        Layout::default().save_to(&path).unwrap();
        let left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["current.ron".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
