//! Re-read `config.ron` when it changes on disk.

use cosmic::iced::futures::{StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use notify::{RecursiveMode, Watcher};
use std::time::Duration;

use crate::model::config::{Config, config_path};

pub fn subscription() -> Subscription<Config> {
    Subscription::run(run)
}

fn run() -> impl iced::futures::Stream<Item = Config> {
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
        Some((Config::load(), (rx, watcher)))
    })
}
