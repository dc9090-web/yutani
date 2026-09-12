//! The applet's `cosmic::Application`: poll `status`, keep the last reply,
//! render it, send actions back. No domain state of its own.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cosmic::app::{Core, Task};
use cosmic::iced::window::Id;
use cosmic::iced::{Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};

use yutani::applet::client::{self, IpcError};
use yutani::applet::display::{Display, degrade, display};
use yutani::applet::rate::{Rates, Sampler};
use yutani::applet::{
    Action, PENDING_S, Poll, daemon_exe, note_visible, pending_done, poll_interval, still_pending,
};
use yutani::tunnel::status::Status;

use crate::view;

pub struct Applet {
    pub core: Core,
    /// The popup's surface id while it is open.
    pub popup: Option<Id>,
    /// The daemon's last `status` reply; `None` is the offline state.
    pub status: Option<Status>,
    pub rates: Rates,
    pub sampler: Sampler,
    /// Monotonic base for the rate sampler and the note timer.
    pub started: Instant,
    /// A `tunnel connect|disconnect` is in flight (icon shows sync):
    /// `(the state it asked for, the deadline it gives up at)`.
    pub pending: Option<(bool, Instant)>,
    /// At most one `status` request outstanding, with at most one deferred
    /// behind it. See [`Poll`].
    pub poll: Poll,
    pub accounts_open: bool,
    /// The last `err …` reply, shown for 3 s.
    pub note: Option<Note>,
}

/// A one-line error note (spec §7): what went wrong, when it was said, and
/// which menu row it belongs under. `action` is `None` for a failed poll,
/// which belongs to no row and sits at the foot of the menu instead.
pub struct Note {
    pub text: String,
    pub at_ms: u64,
    pub action: Option<Action>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// The poll timer fired.
    Tick,
    /// A `status` request came back.
    Status(Result<Status, IpcError>),
    /// A menu row was pressed.
    Press(Action),
    /// A pressed action finished.
    Done(Action, Result<(), String>),
    /// The Accounts… header was pressed: expand or collapse its list.
    ToggleAccounts,
    /// Popup create/destroy, handled by libcosmic.
    Surface(cosmic::surface::Action<Msg>),
    PopupClosed(Id),
}

/// Spawn `cmd` fully detached from the applet: no inherited stdio (so a
/// noisy child cannot write to whatever the applet's own stdio happens to
/// be), its own process group (so a signal aimed at the applet's process
/// group — the panel's, at logout — does not also reach it), and reaped on
/// a dedicated thread so a finished child never sits as a zombie under the
/// applet's pid for as long as the applet keeps running. Used for both
/// fire-and-forget spawns: `xdg-open` and `Start Yutani`.
fn spawn_detached(cmd: &mut Command) -> io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl Applet {
    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// True while a connect/disconnect is still settling (spec §3): until
    /// the daemon reports the state that was asked for, or the deadline
    /// runs out, whichever comes first.
    pub fn pending(&self) -> bool {
        let observed = self.status.as_ref().is_some_and(|s| s.tunnel.connected);
        self.pending
            .is_some_and(|(want, until)| still_pending(until, Instant::now(), pending_done(want, observed)))
    }

    pub fn display(&self) -> Display {
        display(self.status.as_ref(), self.rates)
    }

    /// Ask for a `status` now, or — if one is already outstanding — leave
    /// it to [`Applet::replied`] to issue when that one comes back. This is
    /// the *action* path (press, or an action's reply), which may not drop
    /// its poll: nothing else will show the result before the next tick.
    fn poll(&mut self) -> Task<Msg> {
        if self.poll.request() { Self::status_task() } else { Task::none() }
    }

    fn status_task() -> Task<Msg> {
        cosmic::task::future(async { Msg::Status(client::status().await) })
    }

    /// A `status` reply landed: release the guard, and issue whatever was
    /// deferred behind it.
    fn replied(&mut self) -> Task<Msg> {
        if self.poll.replied() { Self::status_task() } else { Task::none() }
    }

    fn note(&mut self, text: String, action: Option<Action>) {
        let at_ms = self.now_ms();
        self.note = Some(Note { text, at_ms, action });
    }

    /// Open `~/.config/yutani/config.ron`, creating it with defaults first
    /// (spec §4.4). Until plan 5's settings window exists this is the
    /// Preferences… item.
    fn open_preferences() -> Result<(), String> {
        let path = yutani::model::config::config_path();
        if !path.exists()
            && let Err(err) = yutani::model::config::Config::default().save_to(&path)
        {
            return Err(format!("{err:#}"));
        }
        let mut cmd = Command::new("xdg-open");
        cmd.arg(&path);
        spawn_detached(&mut cmd).map_err(|err| {
            // The note never spells the expanded path (that leaks the
            // user's home directory into a UI string); it always says
            // exactly what a person would type.
            let reason = if err.kind() == io::ErrorKind::NotFound {
                "xdg-open is missing".to_string()
            } else {
                format!("could not run xdg-open: {err}")
            };
            format!("{reason}: open ~/.config/yutani/config.ron manually")
        })
    }
}

impl cosmic::Application for Applet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Msg;
    /// Must match the basename of the `.desktop` file `yutani applet
    /// install` writes, which is how cosmic-panel identifies the applet.
    const APP_ID: &'static str = "com.yutani.Applet";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Msg>) {
        let mut applet = Applet {
            core,
            popup: None,
            status: None,
            rates: Rates::default(),
            sampler: Sampler::default(),
            started: Instant::now(),
            pending: None,
            poll: Poll::default(),
            accounts_open: false,
            note: None,
        };
        // Through the guard like every other poll, so the very first reply
        // releases it instead of finding it never armed.
        let first = applet.poll();
        (applet, first)
    }

    fn on_close_requested(&self, id: Id) -> Option<Msg> {
        Some(Msg::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Msg> {
        cosmic::iced::time::every(poll_interval(self.popup.is_some())).map(|_| Msg::Tick)
    }

    fn update(&mut self, message: Msg) -> Task<Msg> {
        // Whatever it says, the request this reply answers is no longer
        // outstanding — and anything deferred behind it goes out now.
        let after_reply =
            if matches!(message, Msg::Status(_)) { self.replied() } else { Task::none() };
        match message {
            Msg::Tick => {
                if let Some(note) = self.note.as_ref()
                    && !note_visible(note.at_ms, self.now_ms())
                {
                    self.note = None;
                }
                if self.poll.tick() { Self::status_task() } else { Task::none() }
            }
            Msg::Status(Ok(status)) => {
                let live = status.tunnel.connected;
                if live {
                    let now = self.now_ms();
                    self.rates =
                        self.sampler.push(status.tunnel.rx_bytes, status.tunnel.tx_bytes, now);
                } else {
                    // Counters freeze and rates read zero while down; the
                    // next connect must not show one huge catch-up spike.
                    self.sampler.reset();
                    self.rates = Rates::default();
                }
                // The deadline is the upper bound; the daemon agreeing ends
                // it sooner, which is the common case.
                if let Some((want, until)) = self.pending
                    && !still_pending(until, Instant::now(), pending_done(want, live))
                {
                    self.pending = None;
                }
                self.status = Some(status);
                after_reply
            }
            Msg::Status(Err(IpcError::Offline)) => {
                self.status = None;
                self.sampler.reset();
                self.rates = Rates::default();
                self.pending = None;
                after_reply
            }
            Msg::Status(Err(IpcError::Failed(msg))) => {
                // A failed poll after a success keeps the last totals
                // (spec §7) — they are counters — but the reply is stale,
                // so it must stop claiming a live tunnel and live rates.
                if let Some(status) = self.status.as_mut() {
                    degrade(status);
                }
                self.sampler.reset();
                self.rates = Rates::default();
                self.note(msg, None);
                after_reply
            }
            Msg::Press(Action::Preferences) => {
                if let Err(msg) = Self::open_preferences() {
                    self.note(msg, Some(Action::Preferences));
                }
                Task::none()
            }
            Msg::Press(Action::StartDaemon) => {
                let mut cmd = Command::new(daemon_exe());
                match spawn_detached(&mut cmd) {
                    Ok(()) => self.poll(),
                    Err(err) => {
                        self.note(format!("cannot start yutani: {err}"), Some(Action::StartDaemon));
                        Task::none()
                    }
                }
            }
            Msg::Press(action) => {
                let Some(request) = action.request() else {
                    return Task::none();
                };
                if matches!(action, Action::Connect | Action::Disconnect) {
                    let want = matches!(action, Action::Connect);
                    self.pending = Some((want, Instant::now() + Duration::from_secs(PENDING_S)));
                }
                cosmic::task::future(async move {
                    let result =
                        client::send(request).await.map(|_| ()).map_err(|err| err.to_string());
                    Msg::Done(action, result)
                })
            }
            Msg::Done(_, Ok(())) => self.poll(),
            Msg::Done(action, Err(msg)) => {
                if matches!(action, Action::Connect | Action::Disconnect) {
                    self.pending = None;
                }
                self.note(msg, Some(action));
                self.poll()
            }
            Msg::ToggleAccounts => {
                self.accounts_open = !self.accounts_open;
                Task::none()
            }
            Msg::Surface(action) => cosmic::task::message(cosmic::Action::Surface(action)),
            Msg::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.accounts_open = false;
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> cosmic::Element<'_, Msg> {
        view::panel_button(self)
    }

    /// The popup's contents come from `app_popup`'s view closure, so no
    /// other surface is ever rendered.
    fn view_window(&self, _id: Id) -> cosmic::Element<'_, Msg> {
        cosmic::widget::text("").into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

/// Open the popup under the panel button. `bounds` and `offset` come from
/// the button's own `on_press_with_rectangle`, which is the only way to
/// learn where the button sits on the panel surface.
pub fn open_popup_message(bounds: Rectangle, offset: cosmic::iced::Vector) -> Msg {
    Msg::Surface(app_popup::<Applet>(
        |_| Default::default(),
        move |state: &mut Applet| {
            let new_id = Id::unique();
            state.popup = Some(new_id);
            // Never `unwrap`: this closure has to return popup settings, so
            // a panel applet whose main surface has not been announced yet
            // cannot bail out — it falls back to the very id libcosmic gives
            // an applet's own window (`app/mod.rs` sets `main_window_id` to
            // `Id::RESERVED`), which is what its internals fall back to too.
            let parent = state.core.main_window_id().unwrap_or(Id::RESERVED);
            let mut settings =
                state.core.applet.get_popup_settings(parent, new_id, None, None, None);
            settings.positioner.anchor_rect = Rectangle {
                x: (bounds.x - offset.x) as i32,
                y: (bounds.y - offset.y) as i32,
                width: bounds.width as i32,
                height: bounds.height as i32,
            };
            settings
        },
        Some(Box::new(move |state: &Applet| {
            cosmic::Element::from(state.core.applet.popup_container(view::popup(state)))
                .map(cosmic::Action::App)
        })),
    ))
}

/// Close the open popup.
pub fn close_popup_message(id: Id) -> Msg {
    Msg::Surface(destroy_popup(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many of this process's children `/proc` currently lists as
    /// zombies (state `Z`) — i.e. exited but not yet `wait`ed on.
    fn zombie_children_of(parent: u32) -> usize {
        let Ok(entries) = std::fs::read_dir("/proc") else { return 0 };
        entries
            .flatten()
            .filter(|entry| {
                let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                    return false;
                };
                // `pid (comm) state ppid ...` — `comm` may itself contain
                // spaces or parens, so split after its closing `)`.
                let Some(after_comm) = stat.rsplit_once(')') else { return false };
                let fields: Vec<&str> = after_comm.1.split_whitespace().collect();
                let (Some(&state), Some(ppid)) =
                    (fields.first(), fields.get(1).and_then(|p| p.parse::<u32>().ok()))
                else {
                    return false;
                };
                state == "Z" && ppid == parent
            })
            .count()
    }

    /// `/bin/true` exits immediately; a well-behaved detached spawn leaves
    /// no trace of it once its reaper thread has had a moment to run.
    #[test]
    fn spawn_detached_runs_the_child_and_reaps_it_without_blocking() {
        let before = zombie_children_of(std::process::id());

        let mut cmd = Command::new("/bin/true");
        let result = spawn_detached(&mut cmd);
        assert!(result.is_ok(), "{result:?}");

        // spawn_detached must not itself block on the child, so this line
        // is reached immediately; give the background reaper thread a
        // moment to run before checking for a zombie.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            zombie_children_of(std::process::id()),
            before,
            "the detached child must be reaped, not left as a zombie"
        );
    }

    /// A command that cannot even start (no such binary) is a plain
    /// `spawn` error, not a panic or a hang.
    #[test]
    fn spawn_detached_reports_a_missing_binary_as_an_error() {
        let mut cmd = Command::new("/no/such/binary-yutani-test");
        let err = spawn_detached(&mut cmd).expect_err("must not exist");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
