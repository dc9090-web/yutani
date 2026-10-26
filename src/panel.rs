//! Taking the applet off COSMIC's panels and putting it back.
//!
//! cosmic-panel spawns an applet for as long as its id is in a panel's
//! `plugins_wings` or `plugins_center`, and does not respawn one that exits
//! on its own — but it does spawn it again at the next login. So "Exit
//! Yutani" takes `com.yutani.Applet` out of those lists, after noting where
//! it sat in a stash file; the next daemon start finds the stash and puts
//! it back in the same slot. An applet the user removed in COSMIC Settings
//! leaves no stash, and stays removed.
//!
//! The files are cosmic-config's: one RON value each, under
//! `~/.config/cosmic/com.system76.CosmicPanel.<name>/v1/`. A file that does
//! not parse as the shape expected here is left alone.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The applet's id in the panel lists (`Applet::APP_ID`, the `.desktop`
/// basename).
pub const APPLET_ID: &str = "com.yutani.Applet";

/// Which of a panel's lists the applet sat in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Area {
    /// `plugins_wings`' first list (the left, or top, wing).
    WingStart,
    /// `plugins_wings`' second list.
    WingEnd,
    /// `plugins_center`.
    Center,
}

/// Where the applet was: panel name (`Panel`, `Dock`, …), list, position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub panel: String,
    pub area: Area,
    pub index: usize,
}

type Wings = Option<(Vec<String>, Vec<String>)>;
type Center = Option<Vec<String>>;

/// Remove every `APPLET_ID` from `list`; the first one's index.
fn take(list: &mut Vec<String>) -> Option<usize> {
    let at = list.iter().position(|id| id == APPLET_ID)?;
    list.retain(|id| id != APPLET_ID);
    Some(at)
}

/// Put `APPLET_ID` back at `index` (or the end, if the list is shorter
/// now), unless it is already there. `true` if the list changed.
fn put(list: &mut Vec<String>, index: usize) -> bool {
    if list.iter().any(|id| id == APPLET_ID) {
        return false;
    }
    list.insert(index.min(list.len()), APPLET_ID.to_string());
    true
}

/// The applet's own lists in a `plugins_wings` value, by area.
fn wing(wings: &mut (Vec<String>, Vec<String>), area: Area) -> &mut Vec<String> {
    if area == Area::WingStart { &mut wings.0 } else { &mut wings.1 }
}

/// `~/.config/cosmic`.
fn cosmic_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("cosmic"))
}

/// `~/.local/state/yutani/panel-slots.ron`: where the applet was when
/// Yutani last exited.
pub fn stash_path() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("yutani").join("panel-slots.ron"))
}

/// Each panel's name and its `v1` directory, in name order.
fn panels(cosmic: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(cosmic) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let panel = name.strip_prefix("com.system76.CosmicPanel.")?.to_string();
            Some((panel, e.path().join("v1")))
        })
        .collect();
    found.sort();
    found
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    ron::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn to_ron<T: Serialize>(value: &T) -> anyhow::Result<String> {
    Ok(ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())?)
}

fn write<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    crate::model::write_atomic(path, &to_ron(value)?)?;
    Ok(())
}

fn read_stash(path: &Path) -> Vec<Slot> {
    read(path).unwrap_or_default()
}

/// Take the applet off every panel under `cosmic`, adding where it was to
/// the stash at `stash`. The stash is written *before* any panel file, so
/// an interrupted exit never loses the slot. The number of slots taken.
pub fn take_off_in(cosmic: &Path, stash: &Path) -> anyhow::Result<usize> {
    let mut slots = Vec::new();
    let mut edits: Vec<(PathBuf, String)> = Vec::new();
    for (panel, dir) in panels(cosmic) {
        let wings_path = dir.join("plugins_wings");
        if let Some(Some(mut wings)) = read::<Wings>(&wings_path) {
            let mut changed = false;
            for area in [Area::WingStart, Area::WingEnd] {
                if let Some(index) = take(wing(&mut wings, area)) {
                    slots.push(Slot { panel: panel.clone(), area, index });
                    changed = true;
                }
            }
            if changed {
                edits.push((wings_path, to_ron(&Some(wings))?));
            }
        }
        let center_path = dir.join("plugins_center");
        if let Some(Some(mut center)) = read::<Center>(&center_path)
            && let Some(index) = take(&mut center)
        {
            slots.push(Slot { panel: panel.clone(), area: Area::Center, index });
            edits.push((center_path, to_ron(&Some(center))?));
        }
    }
    if slots.is_empty() {
        return Ok(0);
    }
    let mut stashed = read_stash(stash);
    stashed.retain(|old| !slots.iter().any(|new| new.panel == old.panel));
    stashed.extend(slots.iter().cloned());
    write(stash, &stashed)?;
    for (path, text) in edits {
        crate::model::write_atomic(&path, &text)?;
    }
    Ok(slots.len())
}

/// Put the applet back in every slot the stash at `stash` names, then
/// remove the stash. A panel that is gone, or a file that no longer
/// parses, is skipped. The number of slots restored.
pub fn put_back_in(cosmic: &Path, stash: &Path) -> anyhow::Result<usize> {
    if !stash.exists() {
        return Ok(0);
    }
    let mut restored = 0;
    for slot in read_stash(stash) {
        let dir = cosmic.join(format!("com.system76.CosmicPanel.{}", slot.panel)).join("v1");
        match slot.area {
            Area::WingStart | Area::WingEnd => {
                let path = dir.join("plugins_wings");
                if let Some(Some(mut wings)) = read::<Wings>(&path)
                    && put(wing(&mut wings, slot.area), slot.index)
                {
                    write(&path, &Some(wings))?;
                    restored += 1;
                }
            }
            Area::Center => {
                let path = dir.join("plugins_center");
                if let Some(Some(mut center)) = read::<Center>(&path)
                    && put(&mut center, slot.index)
                {
                    write(&path, &Some(center))?;
                    restored += 1;
                }
            }
        }
    }
    std::fs::remove_file(stash)?;
    Ok(restored)
}

/// [`take_off_in`] on the real directories.
pub fn take_off() -> anyhow::Result<usize> {
    let (Some(cosmic), Some(stash)) = (cosmic_dir(), stash_path()) else {
        anyhow::bail!("no config or state directory");
    };
    take_off_in(&cosmic, &stash)
}

/// [`put_back_in`] on the real directories.
pub fn put_back() -> anyhow::Result<usize> {
    let (Some(cosmic), Some(stash)) = (cosmic_dir(), stash_path()) else {
        return Ok(0);
    };
    put_back_in(&cosmic, &stash)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sandbox(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("yutani-panel-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox(dir)
    }

    fn panel_file(root: &Path, panel: &str, file: &str, text: &str) -> PathBuf {
        let path = root.join("cosmic").join(format!("com.system76.CosmicPanel.{panel}")).join("v1").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// The file cosmic-panel writes, as it is on the dev machine.
    const WINGS: &str = r#"Some(([
    "com.system76.CosmicAppletWorkspaces",
], [
    "io.github.dc.CosmicAppletVpn",
    "com.yutani.Applet",
    "com.system76.CosmicAppletStatusArea",
]))"#;

    #[test]
    fn exit_takes_the_applet_off_and_the_next_start_puts_it_back_in_its_slot() {
        let s = sandbox("roundtrip");
        let wings = panel_file(&s.0, "Panel", "plugins_wings", WINGS);
        let center = panel_file(&s.0, "Panel", "plugins_center", "Some([\n    \"com.system76.CosmicAppletTime\",\n])");
        let stash = s.0.join("state").join("panel-slots.ron");
        let cosmic = s.0.join("cosmic");

        assert_eq!(take_off_in(&cosmic, &stash).unwrap(), 1);
        let after: Wings = read(&wings).unwrap();
        assert_eq!(after.as_ref().unwrap().1, ["io.github.dc.CosmicAppletVpn", "com.system76.CosmicAppletStatusArea"]);
        assert_eq!(read_stash(&stash), [Slot { panel: "Panel".into(), area: Area::WingEnd, index: 1 }]);
        let center_before = std::fs::read_to_string(&center).unwrap();

        assert_eq!(put_back_in(&cosmic, &stash).unwrap(), 1);
        let back: Wings = read(&wings).unwrap();
        assert_eq!(back, read::<Wings>(&panel_file(&s.0, "Check", "plugins_wings", WINGS)).unwrap());
        assert!(!stash.exists(), "the stash is spent");
        assert_eq!(std::fs::read_to_string(&center).unwrap(), center_before, "an untouched list is not rewritten");
    }

    /// A second exit with nothing on the panel keeps the first stash, and a
    /// start with no stash changes nothing.
    #[test]
    fn nothing_on_the_panel_is_nothing_to_do() {
        let s = sandbox("idle");
        let cosmic = s.0.join("cosmic");
        let stash = s.0.join("panel-slots.ron");
        panel_file(&s.0, "Panel", "plugins_wings", WINGS);
        take_off_in(&cosmic, &stash).unwrap();
        assert_eq!(take_off_in(&cosmic, &stash).unwrap(), 0);
        assert_eq!(read_stash(&stash).len(), 1);
        put_back_in(&cosmic, &stash).unwrap();
        assert_eq!(put_back_in(&cosmic, &stash).unwrap(), 0);
    }

    /// The dock, the centre list, and a list that shrank in between.
    #[test]
    fn every_panel_and_area_is_covered_and_a_shorter_list_gets_it_at_the_end() {
        let s = sandbox("areas");
        let cosmic = s.0.join("cosmic");
        let stash = s.0.join("panel-slots.ron");
        let center = panel_file(&s.0, "Dock", "plugins_center", "Some([\"a\", \"b\", \"com.yutani.Applet\"])");
        panel_file(&s.0, "Panel", "plugins_wings", WINGS);
        assert_eq!(take_off_in(&cosmic, &stash).unwrap(), 2);
        std::fs::write(&center, "Some([\"a\"])").unwrap();
        assert_eq!(put_back_in(&cosmic, &stash).unwrap(), 2);
        assert_eq!(read::<Center>(&center).unwrap().unwrap(), ["a", APPLET_ID]);
    }

    /// The user re-added it by hand before the next start: no duplicate.
    #[test]
    fn an_applet_already_back_is_not_added_twice() {
        let s = sandbox("dup");
        let cosmic = s.0.join("cosmic");
        let stash = s.0.join("panel-slots.ron");
        let wings = panel_file(&s.0, "Panel", "plugins_wings", WINGS);
        take_off_in(&cosmic, &stash).unwrap();
        std::fs::write(&wings, WINGS).unwrap();
        assert_eq!(put_back_in(&cosmic, &stash).unwrap(), 0);
        let ids = read::<Wings>(&wings).unwrap().unwrap().1;
        assert_eq!(ids.iter().filter(|id| *id == APPLET_ID).count(), 1);
    }

    /// A file in some other shape is not ours to rewrite.
    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        let s = sandbox("garbage");
        let cosmic = s.0.join("cosmic");
        let stash = s.0.join("panel-slots.ron");
        let text = "Some(( \"com.yutani.Applet\" ";
        let wings = panel_file(&s.0, "Panel", "plugins_wings", text);
        assert_eq!(take_off_in(&cosmic, &stash).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&wings).unwrap(), text);
        assert!(!stash.exists());
    }
}
