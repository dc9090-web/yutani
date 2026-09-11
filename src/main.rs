mod backend;
mod doctor;
mod model;
mod ui;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "yutani", version, about = "Live thumbnails for EVE Online on COSMIC")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Check that the compositor supports everything Yutani needs
    Doctor,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,yutani=info,cosmic::theme=off,cosmic::app::cosmic=off,cosmic::app=error")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Doctor) => doctor::run(),
        None => {
            let config = model::config::Config::load();
            ui::run(config)
                .map(|()| ExitCode::SUCCESS)
                .map_err(anyhow::Error::from)
        }
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("yutani: {err:#}");
            ExitCode::from(1)
        }
    }
}
