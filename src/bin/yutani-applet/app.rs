//! The applet's `cosmic::Application`: poll `status`, keep the last reply,
//! render it, send actions back. No domain state of its own.

use std::time::{Duration, Instant};

use cosmic::app::{Core, Task};
use cosmic::iced::window::Id;
use cosmic::iced::{Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};

use yutani::applet::client::{self, IpcError};
use yutani::applet::display::{Display, degrade, display};
use yutani::applet::rate::{Rates, Sampler};
use yutani::applet::{
    Action, PENDING_S, daemon_exe, note_visible, pending_done, poll_interval, should_poll,
    still_pending,
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
    /// A `status` request is outstanding. The poll timer skips a tick
    /// rather than stack a second request on a slow daemon.
    pub polling: bool,
    pub accounts_open: bool,
    /// `(message, set_at_ms)` — an `err …` reply, shown for 3 s.
    pub note: Option<(String, u64)>,
}

/// The menu rows Task 4 adds are what construct `Press`, `Done` and
/// `ToggleAccounts`; `update` already handles all three, so the arms are
/// written and tested against the protocol here rather than bolted on
/// later.
#[derive(Clone, Debug)]
pub enum Msg {
    /// The poll timer fired.
    Tick,
    /// A `status` request came back.
    Status(Result<Status, IpcError>),
    /// A menu row was pressed.
    #[allow(dead_code)] // constructed by Task 4's menu
    Press(Action),
    /// A pressed action finished.
    #[allow(dead_code)] // constructed by Task 4's menu
    Done(Action, Result<(), String>),
    #[allow(dead_code)] // constructed by Task 4's menu
    ToggleAccounts,
    /// Popup create/destroy, handled by libcosmic.
    Surface(cosmic::surface::Action<Msg>),
    PopupClosed(Id),
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

    fn poll(&mut self) -> Task<Msg> {
        self.polling = true;
        cosmic::task::future(async { Msg::Status(client::status().await) })
    }

    fn note(&mut self, message: String) {
        let at = self.now_ms();
        self.note = Some((message, at));
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
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map(|_| ())
            .map_err(|_| format!("open {} manually", path.display()))
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
        let applet = Applet {
            core,
            popup: None,
            status: None,
            rates: Rates::default(),
            sampler: Sampler::default(),
            started: Instant::now(),
            pending: None,
            polling: true,
            accounts_open: false,
            note: None,
        };
        let first = cosmic::task::future(async { Msg::Status(client::status().await) });
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
        // outstanding.
        if matches!(message, Msg::Status(_)) {
            self.polling = false;
        }
        match message {
            Msg::Tick => {
                if let Some((_, at)) = self.note
                    && !note_visible(at, self.now_ms())
                {
                    self.note = None;
                }
                if should_poll(self.polling) { self.poll() } else { Task::none() }
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
                Task::none()
            }
            Msg::Status(Err(IpcError::Offline)) => {
                self.status = None;
                self.sampler.reset();
                self.rates = Rates::default();
                self.pending = None;
                Task::none()
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
                self.note(msg);
                Task::none()
            }
            Msg::Press(Action::Preferences) => {
                if let Err(msg) = Self::open_preferences() {
                    self.note(msg);
                }
                Task::none()
            }
            Msg::Press(Action::StartDaemon) => match std::process::Command::new(daemon_exe()).spawn()
            {
                Ok(_) => self.poll(),
                Err(err) => {
                    self.note(format!("cannot start yutani: {err}"));
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
                self.note(msg);
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
            let mut settings = state.core.applet.get_popup_settings(
                state.core.main_window_id().unwrap(),
                new_id,
                None,
                None,
                None,
            );
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
