//! The daemon's subscription: wake on a directory event (or every 500 ms),
//! read what is new on the blocking pool, emit the events.

use std::path::PathBuf;
use std::time::Duration;

use cosmic::iced::futures::future::{Either, select};
use cosmic::iced::futures::{StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use notify::{RecursiveMode, Watcher};

use super::LogEvent;
use super::tail::Reader;

const FALLBACK_POLL: Duration = Duration::from_millis(500);
/// A frame of combat writes many lines; let the burst land before reading.
const COALESCE: Duration = Duration::from_millis(50);
/// Re-list the directory this often (polls) even without a create event.
const RESCAN_EVERY: u32 = 40;

/// Keyed on `dir`, so a config change to another directory restarts it.
pub fn subscription(dir: PathBuf) -> Subscription<Vec<LogEvent>> {
    Subscription::run_with(dir, |dir| run(dir.clone()))
}

struct State {
    rx: mpsc::Receiver<bool>,
    /// Kept alive for the stream's lifetime; `None` when polling only.
    _watcher: Option<notify::RecommendedWatcher>,
    reader: Reader,
    polls: u32,
    rescan: bool,
}

fn run(dir: PathBuf) -> impl iced::futures::Stream<Item = Vec<LogEvent>> {
    let (tx, rx) = mpsc::channel::<bool>(16);
    let watcher = {
        let mut tx = tx.clone();
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && !matches!(ev.kind, notify::EventKind::Access(_))
            {
                let listing_changed = matches!(ev.kind, notify::EventKind::Create(_) | notify::EventKind::Remove(_));
                let _ = tx.try_send(listing_changed);
            }
        })
        .and_then(|mut w| {
            w.watch(&dir.join("Gamelogs"), RecursiveMode::NonRecursive)?;
            w.watch(&dir.join("Chatlogs"), RecursiveMode::NonRecursive)?;
            Ok(w)
        })
    };
    let watcher = match watcher {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!("EVE log watcher unavailable, polling only: {e}");
            None
        }
    };
    let state = State { rx, _watcher: watcher, reader: Reader::new(dir), polls: 0, rescan: true };
    iced::futures::stream::unfold(Some(state), |state| async move {
        let mut state = state?;
        loop {
            // A wake-up from the watcher, or the fallback tick.
            match select(state.rx.next(), futures_timer::Delay::new(FALLBACK_POLL)).await {
                Either::Left((Some(listing_changed), _)) => {
                    state.rescan |= listing_changed;
                    futures_timer::Delay::new(COALESCE).await;
                    while let Ok(more) = state.rx.try_recv() {
                        state.rescan |= more;
                    }
                }
                Either::Left((None, _)) | Either::Right(_) => {}
            }
            state.polls = state.polls.wrapping_add(1);
            let do_rescan = state.rescan || state.polls % RESCAN_EVERY == 1;
            state.rescan = false;
            let mut reader = state.reader;
            let (events, reader) = match tokio::task::spawn_blocking(move || {
                let mut events = if do_rescan { reader.rescan() } else { Vec::new() };
                events.extend(reader.poll());
                (events, reader)
            })
            .await
            {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!("EVE log read failed; stopping: {e}");
                    return None;
                }
            };
            state.reader = reader;
            if !events.is_empty() {
                return Some((events, Some(state)));
            }
        }
    })
}
