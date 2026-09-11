//! StatusNotifierItem tray icon (COSMIC's status-area applet shows it).

use cosmic::iced::futures::StreamExt;
use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::stream::{self, BoxStream};
use cosmic::iced::{self, Subscription};
use ksni::blocking::TrayMethods;

#[derive(Clone, Debug)]
pub enum TrayEvent {
    /// Icon click: flip `hidden`.
    ToggleVisibility,
    /// Menu items: set `hidden` to exactly this (idempotent — clicking
    /// "Show" twice must not hide).
    SetHidden(bool),
    Quit,
}

pub struct YutaniTray {
    tx: mpsc::UnboundedSender<TrayEvent>,
}

impl ksni::Tray for YutaniTray {
    fn id(&self) -> String {
        "yutani".into()
    }
    fn title(&self) -> String {
        "Yutani".into()
    }
    fn icon_name(&self) -> String {
        // Stock icon until packaging installs our own (plan 5).
        "video-display-symbolic".into()
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.unbounded_send(TrayEvent::ToggleVisibility);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        // Two always-present items rather than one label that tracks
        // `hidden`: ksni only re-reads `menu()` on `Handle::update`, and the
        // handle lives inside the subscription's stream, not on `App` — so
        // there is nothing to call `update` from. Each item therefore sets
        // an absolute state rather than toggling, so "Show" while shown is
        // a no-op instead of hiding.
        vec![
            StandardItem {
                label: "Show thumbnails".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.unbounded_send(TrayEvent::SetHidden(false));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide thumbnails".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.unbounded_send(TrayEvent::SetHidden(true));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.unbounded_send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// `Subscription::run_with` (this pinned iced) takes a bare `fn(&D) -> S`, so
/// the builder must be a named fn, not a capturing closure (see
/// `backend::subscription` for the same constraint). The tray has no
/// per-call data, so `Key` just hashes a constant to identify this
/// subscription.
#[derive(Clone)]
struct Key;

impl std::hash::Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "yutani-tray".hash(state);
    }
}

fn run(_key: &Key) -> BoxStream<'static, TrayEvent> {
    let (tx, rx) = mpsc::unbounded::<TrayEvent>();
    let tray = YutaniTray { tx };
    let handle = match tray.spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::warn!("tray unavailable: {e}");
            None
        }
    };
    // The D-Bus service runs on its own thread regardless of the handle
    // (ksni 0.3: `Handle` is a Weak + sender, no Drop). We keep it in the
    // stream state anyway so a future `handle.update(..)` (e.g. dynamic
    // menu labels) has somewhere to live.
    Box::pin(stream::unfold((rx, handle), |(mut rx, handle)| async move {
        let ev = rx.next().await?;
        Some((ev, (rx, handle)))
    }))
}

pub fn subscription() -> Subscription<TrayEvent> {
    iced::Subscription::run_with(Key, run)
}
