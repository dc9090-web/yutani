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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Keyed by character name.
    pub thumbs: BTreeMap<String, ThumbPos>,
}

fn round_to(v: i32, grid: i32) -> i32 {
    ((v as f64 / grid as f64).round() as i32) * grid
}

/// Snap a dragged rect's top-left. Edge snapping (flush against, or aligned
/// with, another rect's edges within `edge_threshold`) takes precedence over
/// the grid, per axis.
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

pub fn current_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yutani")
        .join("layouts")
        .join("current.ron")
}

impl Layout {
    pub fn load() -> Self {
        Self::load_from(&current_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("cannot read {}: {e}; starting with an empty layout", path.display());
                return Self::default();
            }
        };
        ron::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("cannot parse {}: {e}; starting with an empty layout", path.display());
            Self::default()
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&current_path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?)?;
        Ok(())
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
}
