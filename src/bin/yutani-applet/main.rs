//! `yutani-applet`: the COSMIC panel applet. cosmic-panel spawns one
//! process per panel slot (see `yutani applet install`), so there is no
//! single-instance handling here — that belongs to the daemon.

mod app;
mod view;

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
    cosmic::applet::run::<app::Applet>(())
}
