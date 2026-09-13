//! The applet's `cosmic::Application`: poll `status`, keep the last reply,
//! render it, send actions back. No domain state of its own.

use std::io::{self, Read as _};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use cosmic::app::{Core, Task};
use cosmic::iced::window::Id;
use cosmic::iced::{Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};

use yutani::applet::client::{self, IpcError};
use yutani::applet::display::{Display, degrade, display};
use yutani::applet::rate::{Rates, Sampler};
use yutani::applet::{
    Action, PENDING_S, Poll, clip_note, note_visible, pending_done, poll_interval, start_command,
    still_pending, waiting_note,
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
    /// A `quit` was acknowledged: every `status` until the daemon is gone
    /// (or this deadline passes) describes a tunnel on its way down, so it
    /// is degraded rather than believed. See `Msg::Done(Action::Quit, ..)`.
    pub quitting: Option<Instant>,
    pub accounts_open: bool,
    /// The last `err …` reply, shown for 3 s — or what a tunnel action is
    /// still doing, shown until the daemon answers it.
    pub note: Option<Note>,
}

/// A one-line note under a menu row (spec §7): what went wrong, when it was
/// said, and which row it belongs under. `action` is `None` for a failed
/// poll, which belongs to no row and sits at the foot of the menu instead.
/// A `progress` note is not an error but what a slow action is doing while
/// it is awaited (`waiting_note`); it is muted rather than red and lives
/// until the action's reply replaces or clears it, not for `NOTE_MS`.
pub struct Note {
    pub text: String,
    pub at_ms: u64,
    pub action: Option<Action>,
    pub progress: bool,
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

/// How long "Start Yutani" watches the child before taking silence as
/// success. `systemctl start` returns within this; an in-process daemon
/// that is going to die at startup (config parse error, "already running",
/// the compositor refusing it) does so within this too.
const START_WINDOW: Duration = Duration::from_secs(2);

/// How much of the child's stderr is kept for the note. The rest is read
/// and dropped, so a chatty daemon never blocks on a full pipe.
const STDERR_CAP: usize = 4096;

/// What a detached child did, once it has exited: its status and what it
/// wrote to stderr (the first [`STDERR_CAP`] bytes, lossily decoded).
#[derive(Debug)]
struct Exit {
    status: ExitStatus,
    stderr: String,
}

/// Spawn `cmd` detached from the applet: no inherited stdin/stdout, stderr
/// on a pipe the applet drains (so a noisy child cannot write to whatever
/// the applet's own stdio happens to be, and what it says as it dies is
/// not lost), its own process group (so a signal aimed at the applet's
/// process group — the panel's, at logout — does not also reach it), and
/// reaped on a dedicated thread so a finished child never sits as a zombie
/// under the applet's pid for as long as the applet keeps running. The
/// receiver reports the child's [`Exit`]; a child that outlives the
/// applet's interest simply finds it dropped. Used for the one spawn there
/// is: `Start Yutani`.
///
/// The thread drains stderr to EOF *before* waiting: the daemon's own
/// children are all short-lived or given their own stderr, so EOF follows
/// its exit closely, and reading first means the whole of a short dying
/// message is there when the status is.
fn spawn_detached(cmd: &mut Command) -> io::Result<oneshot::Receiver<Exit>> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).process_group(0);
    let mut child = cmd.spawn()?;
    let (tx, rx) = oneshot::channel();
    let reap = move || {
        let mut kept = Vec::new();
        if let Some(mut stderr) = child.stderr.take() {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf)
                && n > 0
            {
                let room = STDERR_CAP.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
        if let Ok(status) = child.wait() {
            let _ = tx.send(Exit { status, stderr: String::from_utf8_lossy(&kept).into_owned() });
        }
    };
    // A thread that cannot be spawned (`RLIMIT_NPROC`, cgroup `pids.max`)
    // is an error to show, not a panic that takes the applet down; the
    // child is left to init and its receiver reports nothing.
    if let Err(err) = std::thread::Builder::new().name("reap-yutani".into()).spawn(reap) {
        tracing::warn!("cannot spawn the reaper thread for yutani start: {err}");
    }
    Ok(rx)
}

/// What "Start Yutani" makes of the child's fate at the end of
/// [`START_WINDOW`]: `None` (still running) or a clean exit is success —
/// the polls that follow show the daemon — and a failed exit is the last
/// thing it said, or its status when it said nothing.
fn start_outcome(exit: Option<Exit>) -> Result<(), String> {
    let Some(exit) = exit else { return Ok(()) };
    if exit.status.success() {
        return Ok(());
    }
    let last_line = exit.stderr.lines().rev().map(str::trim).find(|l| !l.is_empty());
    Err(last_line.map_or_else(|| format!("yutani start: {}", exit.status), str::to_string))
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
        self.note = Some(Note { text: clip_note(&text), at_ms, action, progress: false });
    }

    /// Say what `action` is doing while its reply is awaited, if it is one
    /// of the slow ones.
    fn waiting(&mut self, action: Action) {
        if let Some(text) = waiting_note(action) {
            let at_ms = self.now_ms();
            self.note = Some(Note { text, at_ms, action: Some(action), progress: true });
        }
    }

    /// The reply to `action` is in: whatever it said, the wait is over.
    fn done_waiting(&mut self, action: Action) {
        if self.note.as_ref().is_some_and(|n| n.progress && n.action == Some(action)) {
            self.note = None;
        }
    }

    /// The note the menu shows right now: an error for `NOTE_MS` after it
    /// was set, a progress note for as long as it is there.
    pub fn visible_note(&self) -> Option<&Note> {
        self.note.as_ref().filter(|n| n.progress || note_visible(n.at_ms, self.now_ms()))
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
            quitting: None,
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
                if self.note.is_some() && self.visible_note().is_none() {
                    self.note = None;
                }
                if self.poll.tick() { Self::status_task() } else { Task::none() }
            }
            Msg::Status(Ok(mut status)) => {
                // The daemon is still answering while it winds down after
                // a quit, and says "Connected" until the iface is gone.
                if let Some(until) = self.quitting {
                    if still_pending(until, Instant::now(), false) {
                        degrade(&mut status);
                    } else {
                        self.quitting = None;
                    }
                }
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
                self.quitting = None;
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
                // A wait that is still on outranks a poll error: its reply
                // is what ends it, and it must find its own note to clear.
                if !self.note.as_ref().is_some_and(|n| n.progress) {
                    self.note(msg, None);
                }
                after_reply
            }
            Msg::Press(Action::StartDaemon) => match spawn_detached(&mut start_command()) {
                // Watch it for the window off the UI thread: a start that
                // fails is a note under the row, not a menu stuck on
                // "Start Yutani" with the reason in /dev/null.
                Ok(exit) => Task::batch([
                    self.poll(),
                    cosmic::task::future(async move {
                        let exit = tokio::time::timeout(START_WINDOW, exit).await.ok().and_then(Result::ok);
                        Msg::Done(Action::StartDaemon, start_outcome(exit))
                    }),
                ]),
                Err(err) => {
                    self.note(format!("cannot start yutani: {err}"), Some(Action::StartDaemon));
                    Task::none()
                }
            },
            Msg::Press(action) => {
                let Some(request) = action.request() else {
                    return Task::none();
                };
                if matches!(action, Action::Connect | Action::Disconnect) {
                    let want = matches!(action, Action::Connect);
                    self.pending = Some((want, Instant::now() + Duration::from_secs(PENDING_S)));
                }
                self.waiting(action);
                cosmic::task::future(async move {
                    let result =
                        client::send(request).await.map(|_| ()).map_err(|err| err.to_string());
                    Msg::Done(action, result)
                })
            }
            // The daemon acknowledges `quit` immediately but may spend up
            // to ~10 s stopping the tunnel before it goes away, so `status`
            // keeps answering — and keeps saying "Connected" — the whole
            // time. The tunnel is on its way down, so say so now rather
            // than showing a live link that no longer has an owner, and
            // keep saying so through the polls that follow (`quitting`)
            // until the socket is gone — or, should the daemon never go,
            // for as long as it is given to stop.
            Msg::Done(Action::Quit, Ok(())) => {
                if let Some(status) = self.status.as_mut() {
                    degrade(status);
                }
                self.quitting = Some(Instant::now() + Duration::from_secs(PENDING_S));
                self.sampler.reset();
                self.rates = Rates::default();
                self.poll()
            }
            Msg::Done(action, Ok(())) => {
                self.done_waiting(action);
                self.poll()
            }
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
            // Opening the popup polls at once: the timer's first tick after
            // its 5 s → 1 s switch is a full second away, and what it would
            // show meanwhile is up to 5 s old. (`popup` is set by the open
            // action's own closure, later, so it is still `None` here; on
            // close it is `Some`.)
            Msg::Surface(action) => {
                let surface = cosmic::task::message(cosmic::Action::Surface(action));
                if self.popup.is_none() { Task::batch([surface, self.poll()]) } else { surface }
            }
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
    use cosmic::Application as _;
    use yutani::applet::{NOTE_MAX_CHARS, NOTE_MS};
    use yutani::tunnel::status::Status;

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

    /// A current-thread runtime, as `client.rs`'s tests use.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap().block_on(f)
    }

    /// `/bin/true` exits immediately; a well-behaved detached spawn leaves
    /// no trace of it once its reaper thread has had a moment to run.
    #[test]
    fn spawn_detached_runs_the_child_and_reaps_it_without_blocking() {
        let before = zombie_children_of(std::process::id());

        let mut cmd = Command::new("/bin/true");
        let result = spawn_detached(&mut cmd);
        assert!(result.is_ok(), "{result:?}");
        let exit = block_on(result.unwrap()).expect("the reaper reports the exit");
        assert!(exit.status.success());
        assert_eq!(exit.stderr, "");

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

    /// I1: a daemon that dies at startup (config parse error, "yutani is
    /// already running", a unit that fails to start) used to vanish into
    /// /dev/null with the menu stuck on "Start Yutani". Its exit status and
    /// what it said on stderr now come back, and the last line of that is
    /// the note under the row.
    #[test]
    fn spawn_detached_reports_a_startup_failure_with_its_stderr() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo first >&2; echo yutani is already running >&2; exit 3"]);
        let exit = block_on(spawn_detached(&mut cmd).unwrap()).unwrap();
        assert_eq!(exit.status.code(), Some(3));
        assert_eq!(exit.stderr, "first\nyutani is already running\n");
        assert_eq!(start_outcome(Some(exit)), Err("yutani is already running".to_string()));
    }

    /// stderr is kept only up to `STDERR_CAP` but *drained* past it, so a
    /// chatty child never blocks on a full pipe — and the child's exit is
    /// still reported. A failure that said nothing is reported by status.
    #[test]
    fn spawn_detached_bounds_the_captured_stderr_without_blocking_the_child() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "head -c 200000 /dev/zero | tr '\\0' x >&2; exit 1"]);
        let exit = block_on(spawn_detached(&mut cmd).unwrap()).unwrap();
        assert_eq!(exit.status.code(), Some(1));
        assert_eq!(exit.stderr.len(), STDERR_CAP);
        assert!(exit.stderr.bytes().all(|b| b == b'x'));
        assert_eq!(start_outcome(Some(exit)).unwrap_err().len(), STDERR_CAP, "the last line, however long");

        let mut silent = Command::new("/bin/sh");
        silent.args(["-c", "exit 2"]);
        let exit = block_on(spawn_detached(&mut silent).unwrap()).unwrap();
        assert_eq!(start_outcome(Some(exit)), Err("yutani start: exit status: 2".to_string()));
    }

    /// Still running when the window closes — the normal in-process start
    /// — or exited cleanly (`systemctl start` returned 0): both are "it
    /// started", and the polls that follow show the daemon.
    #[test]
    fn a_daemon_still_running_after_the_window_or_exited_cleanly_has_started() {
        assert_eq!(start_outcome(None), Ok(()));
        let exit = block_on(spawn_detached(&mut Command::new("/bin/true")).unwrap()).unwrap();
        assert_eq!(start_outcome(Some(exit)), Ok(()));
    }

    fn applet() -> Applet {
        Applet::init(Core::default(), ()).0
    }

    /// M3: the timer's first tick after the 5 s → 1 s switch is a full
    /// second away, so opening the popup polls at once rather than showing
    /// up to 5 s old data for that second. Closing it polls nothing.
    #[test]
    fn opening_the_popup_polls_at_once_and_closing_it_does_not() {
        let mut applet = applet();
        let _ = applet.update(Msg::Status(Ok(connected())));
        assert!(!applet.poll.in_flight(), "init's poll has been answered");

        let bounds = Rectangle { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        let _ = applet.update(open_popup_message(bounds, cosmic::iced::Vector::default()));
        assert!(applet.poll.in_flight(), "a poll goes out with the open");

        let _ = applet.update(Msg::Status(Ok(connected())));
        let id = Id::unique();
        applet.popup = Some(id);
        let _ = applet.update(close_popup_message(id));
        assert!(!applet.poll.in_flight(), "closing polls nothing");
    }

    fn connected() -> Status {
        use yutani::tunnel::status::{Status, TunnelStatus};
        let tunnel = TunnelStatus { installed: true, connected: true, handshake_age_s: Some(4), ..Default::default() };
        Status { clients: vec![], hidden: false, tunnel }
    }

    /// M1: `quit` is acknowledged at once but the daemon spends up to 10 s
    /// stopping the tunnel and answers `status` with "Connected" the whole
    /// time — so the degrade on Quit must hold across those polls, not be
    /// undone by the very poll it issues, until the daemon is gone.
    #[test]
    fn the_quit_degrade_holds_until_the_daemon_is_gone() {
        let mut applet = applet();
        let _ = applet.update(Msg::Status(Ok(connected())));
        assert!(applet.status.as_ref().unwrap().tunnel.connected);

        let _ = applet.update(Msg::Done(Action::Quit, Ok(())));
        assert!(!applet.status.as_ref().unwrap().tunnel.connected, "said to be going down at once");
        let _ = applet.update(Msg::Status(Ok(connected())));
        assert!(!applet.status.as_ref().unwrap().tunnel.connected, "and still, while the daemon winds down");
        assert!(applet.status.as_ref().unwrap().tunnel.handshake_age_s.is_none());

        let _ = applet.update(Msg::Status(Err(IpcError::Offline)));
        assert!(applet.status.is_none());
        // A daemon started afresh is believed again.
        let _ = applet.update(Msg::Status(Ok(connected())));
        assert!(applet.status.as_ref().unwrap().tunnel.connected, "the quit is over once the daemon was gone");
    }

    /// M1: a daemon that never goes away (a quit it did not act on) is
    /// believed again after the same 10 s bound the sync icon uses.
    #[test]
    fn the_quit_degrade_gives_up_after_the_daemon_stop_timeout() {
        let mut applet = applet();
        let _ = applet.update(Msg::Status(Ok(connected())));
        let _ = applet.update(Msg::Done(Action::Quit, Ok(())));
        applet.quitting = Some(Instant::now() - Duration::from_secs(1));
        let _ = applet.update(Msg::Status(Ok(connected())));
        assert!(applet.status.as_ref().unwrap().tunnel.connected);
        assert!(applet.quitting.is_none());
    }

    /// M2: whatever a poll or an action says, the note under the row is
    /// clipped to one readable line.
    #[test]
    fn a_long_error_is_clipped_in_the_note() {
        let mut applet = applet();
        let long = "malformed reply ".to_string() + &"x".repeat(2_000);
        let _ = applet.update(Msg::Status(Err(IpcError::Failed(long.clone()))));
        assert_eq!(applet.note.as_ref().unwrap().text.chars().count(), NOTE_MAX_CHARS + 1);
        let _ = applet.update(Msg::Done(Action::Connect, Err(long)));
        assert_eq!(applet.note.as_ref().unwrap().text.chars().count(), NOTE_MAX_CHARS + 1);
    }

    /// I2: a connect/disconnect can take up to 15 s to be answered, so the
    /// menu says what is happening under the row the whole time — a note
    /// that, unlike an error, does not expire after `NOTE_MS` — and the
    /// daemon's answer replaces it: nothing on success, the error on failure.
    #[test]
    fn a_tunnel_press_says_what_is_happening_until_the_daemon_answers() {
        let mut applet = applet();
        let _ = applet.update(Msg::Press(Action::Connect));
        let note = applet.note.as_ref().expect("a waiting note");
        assert_eq!(note.action, Some(Action::Connect));
        assert!(note.progress, "waiting, not an error");
        assert!(note.text.contains("onnecting"), "{}", note.text);
        assert!(applet.visible_note().is_some(), "shown at once");
        // Well past NOTE_MS, still waiting: still shown, and a tick keeps it.
        applet.started = Instant::now() - Duration::from_millis(NOTE_MS * 3);
        let _ = applet.update(Msg::Tick);
        assert!(applet.visible_note().is_some(), "a waiting note does not expire");

        let _ = applet.update(Msg::Done(Action::Connect, Ok(())));
        assert!(applet.note.is_none(), "the answer ends the wait");

        let _ = applet.update(Msg::Press(Action::Disconnect));
        assert!(applet.note.as_ref().is_some_and(|n| n.progress && n.text.contains("isconnecting")));
        let _ = applet.update(Msg::Done(Action::Disconnect, Err("timeout after 15000ms".into())));
        let note = applet.note.as_ref().expect("the error replaces the wait");
        assert!(!note.progress);
        assert_eq!(note.action, Some(Action::Disconnect));
        assert!(note.text.contains("timeout"), "{}", note.text);
    }

    /// A poll that fails while a connect is in flight (a 3 s status
    /// timeout, a `socket_path` error) must not replace "connecting…" with
    /// its own error: the wait is still on, and the daemon's answer would
    /// then find no progress note to clear and leave the red poll error
    /// standing. With no wait on, the poll error is shown as before.
    #[test]
    fn a_failed_poll_does_not_overwrite_a_progress_note() {
        let mut applet = applet();
        let _ = applet.update(Msg::Press(Action::Connect));
        let _ = applet.update(Msg::Status(Err(IpcError::Failed("timeout after 3000ms".into()))));
        let note = applet.note.as_ref().expect("still waiting");
        assert!(note.progress, "the poll error must not replace the wait: {}", note.text);
        assert_eq!(note.action, Some(Action::Connect));
        let _ = applet.update(Msg::Done(Action::Connect, Ok(())));
        assert!(applet.note.is_none(), "the answer ends the wait");

        let _ = applet.update(Msg::Status(Err(IpcError::Failed("timeout after 3000ms".into()))));
        let note = applet.note.as_ref().expect("a poll error with nothing in flight is shown");
        assert!(!note.progress && note.action.is_none(), "{}", note.text);
    }
}
