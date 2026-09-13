//! Re-read `config.ron` when it changes on disk.

use cosmic::iced::futures::{StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use notify::{RecursiveMode, Watcher};
use std::time::Duration;

use crate::model::config::{Config, config_path};

/// One item per change on disk: `Ok` is the config to apply, `Err` says
/// why the file could not be, and the live config stays as it is.
pub fn subscription() -> Subscription<Result<Config, String>> {
    Subscription::run(run)
}

/// What a re-read of `config.ron` means for the live config. A file that
/// parsed replaces it; no file at all means the defaults are in force; a
/// file that exists but cannot be read or parsed — a typo saved while
/// hand-editing, an editor that truncates before it writes — is `Err`,
/// and must leave the live config alone rather than swap every setting
/// for its default until the next save (`save_config` already refuses to
/// touch such a file; applying it live deserves the same respect).
pub fn reload_outcome(loaded: Result<Option<Config>, String>) -> Result<Config, String> {
    loaded.map(Option::unwrap_or_default)
}

fn run() -> impl iced::futures::Stream<Item = Result<Config, String>> {
    let (tx, rx) = mpsc::channel::<()>(8);
    let path = config_path();
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    // The watcher lives as long as the stream: move it into the stream's state.
    let watcher = {
        let mut tx = tx.clone();
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                // Reading the file (`Config::load` below) generates its own
                // `Access` events on the watched directory; without this
                // filter those events would immediately re-trigger a reload,
                // which reads the file again, forever.
                && !matches!(ev.kind, notify::EventKind::Access(_))
                && ev.paths.iter().any(|p| p.file_name() == path.file_name())
            {
                let _ = tx.try_send(());
            }
        })
        .and_then(|mut w| {
            std::fs::create_dir_all(&dir).ok();
            w.watch(&dir, RecursiveMode::NonRecursive)?;
            Ok(w)
        })
    };
    if let Err(e) = &watcher {
        tracing::warn!("config watcher unavailable: {e}");
    }
    iced::futures::stream::unfold((rx, watcher), |(mut rx, watcher)| async move {
        rx.next().await?;
        // Debounce bursts (editors write several events per save).
        futures_timer::Delay::new(Duration::from_millis(200)).await;
        while rx.try_recv().is_ok() {}
        Some((reload_outcome(Config::try_load()), (rx, watcher)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [I3] A `config.ron` that momentarily fails to parse must not be
    /// applied live as the defaults: that switches mode, visibility, sizes
    /// and `app_ids` under the user, destroys every surface, and can drop
    /// clients matched by a custom app id — twice, once the file is fixed.
    #[test]
    fn a_broken_file_is_an_error_not_the_defaults() {
        let custom = Config { thumb_width: 400, ..Config::default() };
        assert_eq!(reload_outcome(Ok(Some(custom.clone()))), Ok(custom));
        assert_eq!(reload_outcome(Ok(None)), Ok(Config::default()), "no file: defaults are in force");
        assert_eq!(reload_outcome(Err("cannot parse".into())), Err("cannot parse".to_string()));
    }
}
