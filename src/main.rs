mod adopt;
mod backend;
mod cli;
mod doctor;
mod launch;
mod proc;
mod shortcuts;
mod ui;

// `ipc`, `model` and `tunnel` now live in the library (so `yutani-applet`
// can share them). Re-binding them at the binary's crate root keeps every
// `crate::ipc::…` / `crate::model::…` / `crate::tunnel::…` path in
// `src/ui/`, `src/cli.rs`, `src/shortcuts.rs` and `src/adopt.rs` working
// unchanged: a private `use` is visible to this module and its descendants.
use yutani::{ipc, model, tunnel};

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
    /// Run a command (Steam's %command%) inside the tunnel cgroup
    Launch {
        /// The game command, e.g. Steam's %command%
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    /// Install or remove the COSMIC keyboard shortcuts (Ctrl+Alt+1..9, Right, Left by default)
    Shortcuts {
        #[command(subcommand)]
        action: ShortcutsAction,
    },
    /// Install or remove the COSMIC panel applet (icons and .desktop file)
    Applet {
        #[command(subcommand)]
        action: AppletAction,
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
enum AppletAction {
    /// Copy the icons and write the applet's .desktop file (idempotent)
    Install,
    /// Remove the icons and the .desktop file
    Uninstall,
}

#[derive(Subcommand)]
enum TunnelAction {
    /// Install the tunnel from a wg-quick .conf (asks for your password once)
    Install {
        conf: std::path::PathBuf,
        /// Print what would be installed instead of installing (no root
        /// needed); exits 1 if the real install would be refused
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the tunnel unit, conf and polkit rule
    Uninstall,
    /// Start the tunnel (EVE traffic goes via the configured exit)
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
        Some(Command::Launch { command }) => Ok(launch::run(command)),
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
        Some(Command::Applet { action: AppletAction::Install }) => {
            yutani::applet::install::install().map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Applet { action: AppletAction::Uninstall }) => {
            yutani::applet::install::uninstall().map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Tunnel { action }) => match action {
            TunnelAction::Install { conf, dry_run: true } => {
                // The same uid/name pair and canonical exe path the real
                // install passes to `install-root`.
                tunnel::install::whoami().and_then(|(uid, user)| {
                    let exe = tunnel::install::current_exe().unwrap_or_default();
                    let report =
                        tunnel::install::install_root(&conf.canonicalize().unwrap_or(conf.clone()), uid, &user, &exe, true)?;
                    print!("{report}");
                    let plan = tunnel::worker::dry_run(&conf, uid)?;
                    print!("\n{plan}");
                    // The whole report is printed either way; the exit
                    // status is what lets a script tell "would install
                    // clean" from "would be refused" without parsing it.
                    Ok(if tunnel::install::report_has_failure(&report) { ExitCode::from(1) } else { ExitCode::SUCCESS })
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
