//! `yutani-applet`: the COSMIC panel applet. cosmic-panel spawns one
//! process per panel slot (see `yutani applet install`), so there is no
//! single-instance handling here — that belongs to the daemon.

mod app;
mod console_view;
mod view;
mod widgets;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(
                    "warn,yutani=info,cosmic::theme=off,cosmic::app=error",
                )
            }),
        )
        .with_writer(std::io::stderr)
        .init();
    yutani::applet::fonts::preload();
    if preview() {
        // The popover in an ordinary window, for visual checks without a
        // panel: `YUTANI_APPLET_PREVIEW=1 yutani-applet`. It polls the real
        // daemon like the applet does.
        let settings = cosmic::app::Settings::default()
            .size(cosmic::iced::Size::new(360.0, 780.0))
            .antialiasing(std::env::var_os("YUTANI_APPLET_NO_AA").is_none());
        return cosmic::app::run::<app::Applet>(settings, ());
    }
    cosmic::applet::run::<app::Applet>(())
}

/// `YUTANI_APPLET_PREVIEW` is set: run as a window showing the popover.
pub fn preview() -> bool {
    std::env::var_os("YUTANI_APPLET_PREVIEW").is_some()
}
