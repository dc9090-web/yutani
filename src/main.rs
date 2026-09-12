mod backend;
mod cli;
mod doctor;
mod ipc;
mod model;
mod shortcuts;
mod tunnel;
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
    /// Print the daemon's status (clients, visibility, tunnel) as JSON
    Status,
    /// Install or remove the COSMIC keyboard shortcuts (Ctrl+Alt+1..9, Right, Left by default)
    Shortcuts {
        #[command(subcommand)]
        action: ShortcutsAction,
    },
    /// EVE-only WireGuard tunnel
    Tunnel {
        #[command(subcommand)]
        action: TunnelAction,
    },
}

#[derive(Subcommand)]
enum ShortcutsAction {
    /// Write Yutani's bindings into COSMIC's custom shortcuts (idempotent)
    Install,
    /// Remove Yutani's bindings, leaving everything else untouched
    Uninstall,
}

#[derive(Subcommand)]
enum TunnelAction {
    /// Install the tunnel from a wg-quick .conf (asks for your password once)
    Install {
        conf: std::path::PathBuf,
        /// Print what would be installed instead of installing (no root needed)
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the tunnel unit, conf and polkit rule
    Uninstall,
    /// Start the tunnel (EVE traffic goes via London)
    Connect,
    /// Stop the tunnel (EVE traffic goes direct)
    Disconnect,
    /// Show tunnel state as JSON
    Status,
    /// [root] the worker behind yutani-tunnel.service
    #[command(hide = true)]
    Run,
    /// [root] called by `install` through pkexec
    #[command(hide = true)]
    InstallRoot {
        #[arg(long)]
        conf: std::path::PathBuf,
        #[arg(long)]
        uid: u32,
        #[arg(long)]
        user: String,
        #[arg(long)]
        exe: String,
    },
    /// [root] called by `uninstall` through pkexec
    #[command(hide = true)]
    UninstallRoot,
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
        Some(Command::Status) => Ok(cli::send(&ipc::Request::Status)),
        Some(Command::Shortcuts { action: ShortcutsAction::Install }) => {
            let config = model::config::Config::load();
            shortcuts::install(&config.shortcuts).map(|(installed, wanted)| {
                let path = shortcuts::custom_path();
                // Say "N of M" when some were skipped, so the summary line on
                // its own shows that something was left out.
                let count = if installed == wanted { installed.to_string() } else { format!("{installed} of {wanted}") };
                println!("installed {count} shortcuts into {}", path.display());
                ExitCode::SUCCESS
            })
        }
        Some(Command::Shortcuts { action: ShortcutsAction::Uninstall }) => shortcuts::uninstall().map(|n| {
            println!("removed {n} shortcuts from {}", shortcuts::custom_path().display());
            ExitCode::SUCCESS
        }),
        Some(Command::Tunnel { action }) => match action {
            TunnelAction::Install { conf, dry_run: true } => {
                let (uid, user) = (ipc::uid(), std::env::var("USER").unwrap_or_default());
                // The same canonical path the real install passes to `install-root`.
                let exe = tunnel::install::current_exe().unwrap_or_default();
                tunnel::install::install_root(&conf.canonicalize().unwrap_or(conf.clone()), uid, &user, &exe, true)
                    .and_then(|report| {
                        print!("{report}");
                        tunnel::worker::dry_run(&conf, uid)
                    })
                    .map(|plan| {
                        print!("\n{plan}");
                        ExitCode::SUCCESS
                    })
            }
            TunnelAction::Install { conf, dry_run: false } => tunnel::install::install(&conf).map(|()| ExitCode::SUCCESS),
            TunnelAction::Uninstall => tunnel::install::uninstall().map(|()| ExitCode::SUCCESS),
            TunnelAction::Connect => tunnel::control::connect().map(|()| ExitCode::SUCCESS),
            TunnelAction::Disconnect => tunnel::control::disconnect().map(|()| ExitCode::SUCCESS),
            TunnelAction::Status => {
                let config = model::config::Config::load();
                let st = tunnel::control::current_tunnel_status(&config.tunnel.location);
                println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default());
                Ok(ExitCode::SUCCESS)
            }
            TunnelAction::Run => tunnel::worker::run().map(|()| ExitCode::SUCCESS),
            TunnelAction::InstallRoot { conf, uid, user, exe } => {
                tunnel::install::install_root(&conf, uid, &user, &exe, false).map(|report| {
                    print!("{report}");
                    ExitCode::SUCCESS
                })
            }
            TunnelAction::UninstallRoot => tunnel::install::uninstall_root(false).map(|r| {
                print!("{r}");
                ExitCode::SUCCESS
            }),
        },
        None => {
            if cli::is_running() {
                eprintln!("yutani is already running");
                Ok(ExitCode::from(1))
            } else {
                let config = model::config::Config::load();
                // The IPC server owns the socket file: it is removed on
                // `quit`, and a stale one (crash/SIGTERM) is replaced at bind.
                // Do not remove it here: if this process lost libcosmic's
                // single-instance race, `ui::run` returns Ok at once and the
                // socket belongs to the winning instance.
                ui::run(config).map(|()| ExitCode::SUCCESS).map_err(anyhow::Error::from)
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
