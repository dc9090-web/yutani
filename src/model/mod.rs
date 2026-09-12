//! Pure data model: client identity, user config and saved layouts.

pub mod client;
pub mod config;
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
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
