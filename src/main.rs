mod backend;
mod cli;
mod doctor;
mod eve_process;
mod launch;
mod launch_steam;
mod shortcuts;
mod ui;

// `ipc`, `model` and `status` live in the library (so `yutani-applet` can
// share them). Re-binding them at the binary's crate root keeps every
// `crate::ipc::…` / `crate::model::…` / `crate::status::…` path in `src/ui/`,
// `src/cli.rs` and `src/shortcuts.rs` working unchanged: a private `use` is
// visible to this module and its descendants.
use yutani::{ipc, model, status};

use clap::{Parser, Subcommand};
use std::io::{self, Write as _};
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
    /// Apply a saved layout by name
    Layout {
        /// Name of a layout in ~/.config/yutani/layouts
        name: String,
    },
    /// List the saved layouts, one per line
    Layouts,
    /// Open the settings window, optionally on a page
    Settings {
        /// display | behaviour | layouts | characters | steam
        page: Option<String>,
    },
    /// Ask the running instance to exit
    Quit,
    /// Print the daemon's status (clients, visibility) as JSON
    Status,
    /// Run a command (Steam's %command%) with the game's launch environment
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
    /// Start the daemon: through the systemd user unit when `yutani service install` has been run, else in this process
    Start,
    /// Install or remove a systemd user unit that restarts the daemon after a crash
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Install or remove the COSMIC panel applet (icons, .desktop files, Applications launcher)
    Applet {
        #[command(subcommand)]
        action: AppletAction,
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
enum ServiceAction {
    /// Write ~/.config/systemd/user/yutani.service (Restart=on-failure) and reload (idempotent)
    Install,
    /// Stop and remove the unit
    Uninstall,
}

#[derive(Subcommand)]
enum AppletAction {
    /// Copy the icons and write the applet and launcher .desktop files (idempotent)
    Install,
    /// Remove the icons and both .desktop files
    Uninstall,
}

/// The daemon itself, in this process (the bare `yutani` and `yutani start`
/// without a unit).
fn run_daemon() -> anyhow::Result<ExitCode> {
    if cli::is_running() {
        let _ = writeln!(io::stderr().lock(), "yutani is already running");
        return Ok(ExitCode::from(1));
    }
    let config = model::config::Config::load();
    // The IPC server owns the socket file: it is removed on `quit`, and a
    // stale one (crash/SIGTERM) is replaced at bind. Do not remove it here:
    // if this process lost libcosmic's single-instance race, `ui::run`
    // returns Ok at once and the socket belongs to the winning instance.
    ui::run(config).map(|()| ExitCode::SUCCESS).map_err(anyhow::Error::from)
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
        Some(Command::Layout { name }) => Ok(match ipc::Request::layout(name) {
            Ok(request) => cli::send(&request),
            Err(msg) => {
                eprintln!("yutani: {msg}");
                ExitCode::from(1)
            }
        }),
        Some(Command::Layouts) => Ok(cli::layouts()),
        Some(Command::Settings { page: None }) => Ok(cli::send(&ipc::Request::Settings)),
        Some(Command::Settings { page: Some(page) }) => Ok(cli::send(&ipc::Request::SettingsPage(page))),
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
        Some(Command::Start) => {
            if yutani::service::installed() {
                yutani::service::start_unit().map(|()| ExitCode::SUCCESS)
            } else {
                run_daemon()
            }
        }
        Some(Command::Service { action: ServiceAction::Install }) => {
            std::env::current_exe()
                .and_then(|e| e.canonicalize())
                .map_err(|e| anyhow::anyhow!("cannot resolve the running yutani binary: {e}"))
                .and_then(|exe| yutani::service::install(&exe))
                .map(|path| {
                println!("wrote {}", path.display());
                println!("the Applications launcher (yutani start) now starts it through systemd;");
                println!("a crash restarts it after a second, `yutani quit` stops it.");
                println!("to start it at login: systemctl --user enable {}", yutani::service::UNIT_NAME);
                ExitCode::SUCCESS
            })
        }
        Some(Command::Service { action: ServiceAction::Uninstall }) => yutani::service::uninstall().map(|removed| {
            println!("{}", if removed { "removed the unit" } else { "no unit installed" });
            ExitCode::SUCCESS
        }),
        Some(Command::Applet { action: AppletAction::Install }) => {
            yutani::applet::install::install().map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Applet { action: AppletAction::Uninstall }) => {
            yutani::applet::install::uninstall().map(|()| ExitCode::SUCCESS)
        }
        None => run_daemon(),
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            // Not `eprintln!`: the daemon started by the applet has the
            // applet's pipe as stderr, and once cosmic-panel has restarted
            // the applet nobody reads it — `eprintln!` would panic on the
            // EPIPE (exit 101, message lost either way) instead of exiting 1.
            let _ = writeln!(io::stderr().lock(), "yutani: {err:#}");
            ExitCode::from(1)
        }
    }
}
