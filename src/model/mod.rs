//! Pure data model: client identity, user config and saved layouts.

pub mod client;
pub mod config;
pub mod date;
pub mod layout;

use std::path::Path;

/// Write `text` to `path` atomically: a sibling temporary file, then a
/// rename over the target. A reader (or the `notify` config watcher) never
/// sees a half-written file, and a failed write leaves the previous
/// contents intact. The temporary deliberately does **not** keep the
/// target's extension, so the config watcher — which keys on the file
/// name — ignores it.
///
/// Not for `~/.config/cosmic/…/custom`: that one must be written in place
/// (see `src/shortcuts.rs`), because cosmic-config's watcher ignores the
/// paired rename events.
///
/// A failed rename takes the temporary with it, so a target that cannot be
/// replaced (a directory in the way, a read-only mount) does not leave a
/// stray `<stem>.tmp` in the config directory for good.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rename that fails leaves the previous contents alone — and must not
    /// leave the temporary behind either, or the next reader of the
    /// directory finds a stray `current.tmp` forever.
    #[test]
    fn write_atomic_leaves_no_temporary_when_the_rename_fails() {
        let dir = std::env::temp_dir().join(format!("yutani-write-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A normal write lands, and leaves only the target behind.
        let path = dir.join("current.ron");
        write_atomic(&path, "(one)").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(one)");
        assert!(!dir.join("current.tmp").exists());

        // The target is a directory: the write succeeds, the rename cannot.
        let blocked = dir.join("blocked.ron");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_atomic(&blocked, "(two)").is_err());
        assert!(!dir.join("blocked.tmp").exists(), "the temporary is left behind");
        assert!(blocked.is_dir());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
