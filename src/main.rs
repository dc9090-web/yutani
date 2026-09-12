mod backend;
mod cli;
mod doctor;
mod ipc;
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
    /// Focus the n-th client in layout order (1-based)
    Focus {
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        n: u32,
    },
    /// Focus the next client in layout order
    Next,
    /// Focus the previous client in layout order
    Prev,
    /// Show all thumbnails
    Show,
    /// Hide all thumbnails
    Hide,
    /// Hide the thumbnails if shown, show them if hidden
    Toggle,
    /// Ask the running instance to exit
    Quit,
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
        Some(Command::Focus { n }) => Ok(cli::send(&ipc::Request::Focus(n as usize))),
        Some(Command::Next) => Ok(cli::send(&ipc::Request::Next)),
        Some(Command::Prev) => Ok(cli::send(&ipc::Request::Prev)),
        Some(Command::Show) => Ok(cli::send(&ipc::Request::Show)),
        Some(Command::Hide) => Ok(cli::send(&ipc::Request::Hide)),
        Some(Command::Toggle) => Ok(cli::send(&ipc::Request::Toggle)),
        Some(Command::Quit) => Ok(cli::send(&ipc::Request::Quit)),
        None => {
            if cli::is_running() {
                eprintln!("yutani is already running");
                Ok(ExitCode::from(1))
            } else {
                let config = model::config::Config::load();
                let outcome = ui::run(config).map(|()| ExitCode::SUCCESS).map_err(anyhow::Error::from);
                // Belt and braces: the app removes the socket on `quit`, but a
                // panic or SIGTERM path may not get there.
                let _ = std::fs::remove_file(ipc::socket_path());
                outcome
            }
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
