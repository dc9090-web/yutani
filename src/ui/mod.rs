//! libcosmic application: no main window. One overlay layer surface per
//! shown EVE client, each showing that client's live captured frame.
//! Floating mode: the user places them (drag/pin/persist). Dock mode: the
//! same surfaces, auto-arranged and centred along `dock_edge` (see `dock`).

use cosmic::app::ApplicationExt;
use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::{Connection, Proxy, protocol::wl_output::WlOutput};
use cosmic::iced::event::wayland::{Event as WaylandEvent, LayerEvent, OutputEvent};
use cosmic::iced::mouse;
use cosmic::iced::core::layout::Limits;
use cosmic::iced::platform_specific::shell::commands::activation;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, set_anchor, set_exclusive_zone, set_margin, set_size,
};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{self, Length, Point, Subscription};
use cosmic::{Application, Element, Task, widget};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::adopt;
use crate::backend::{self, CaptureImage, ClientInfo, Cmd, Event, Handle};
use crate::model::client::Login;
use crate::model::config::{Config, Mode};
use crate::model::layout::{self, Layout, Rect, ThumbPos};

pub mod characters;
pub mod config_watch;
pub mod dock;
pub mod ipc;
pub mod pointer;
pub mod rules;
pub mod settings;
pub mod thumbnail;
pub mod tunnel_page;

/// The daemon's flags. `Config` itself lives in the library now, and the
/// orphan rule forbids implementing libcosmic's `CosmicFlags` for a foreign
/// type here, so it travels into `run_single_instance` in this newtype. The
/// trait is deliberately *not* implemented in the library: single-instance
/// is the daemon's concern and the applet must never use it.
pub struct AppFlags(pub Config);

/// `Config` carries no CLI-parsed subcommand/args; only its file contents
/// matter, so this satisfies `run_single_instance`'s bound trivially.
impl cosmic::app::CosmicFlags for AppFlags {
    type SubCommand = String;
    type Args = Vec<String>;
}

pub fn run(config: Config) -> iced::Result {
    cosmic::app::run_single_instance::<App>(
        cosmic::app::Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        AppFlags(config),
    )
}

#[derive(Clone, Debug)]
pub struct Output {
    pub handle: WlOutput,
    pub name: String,
    pub logical_size: (i32, i32),
    pub scale: i32,
}

#[derive(Debug)]
pub struct Client {
    pub info: ClientInfo,
    pub image: Option<CaptureImage>,
    pub surface: Option<SurfaceId>,
    /// Top-left position on its output, logical pixels.
    pub position: (i32, i32),
    pub pinned: bool,
    /// Last surface-local cursor position we saw for this client, since
    /// `ButtonPressed` does not carry one.
    pub last_cursor: Point,
    /// Cursor is currently over this thumbnail; surface is zoomed.
    pub hovered: bool,
    /// Last capture attempt for this client failed; show a placeholder
    /// instead of a stale or absent frame.
    pub unavailable: bool,
    /// `create_surface` has placed this client at least once; on a later
    /// destroy+recreate cycle, reuse `position`/`pinned` instead of falling
    /// back to a saved or freshly computed slot.
    pub placed: bool,
    /// Last size sent via `set_size` (or the surface's creation size), so a
    /// resize to the same size can be skipped.
    pub last_size: Option<(u32, u32)>,
    /// Backend capture is paused for this client (last command we sent).
    /// Pause/resume is an edge on this, not on whether a surface exists, so
    /// a client born hidden is paused too.
    pub paused: bool,
    /// Connector name of the output this client's surface was created on;
    /// empty when it has none. Applying a layout compares this with the
    /// layout's output: a layer surface is bound to one output for life, so
    /// a different one means destroy + recreate, not `set_margin`.
    pub output: String,
}

pub struct App {
    core: cosmic::app::Core,
    pub config: Config,
    pub conn: Option<Connection>,
    pub cmd: Option<calloop::channel::Sender<Cmd>>,
    pub clients: HashMap<Handle, Client>,
    pub outputs: Vec<Output>,
    pub layout: Layout,
    /// Set at startup when `current.ron` exists but failed to parse (spec
    /// §10). While true, `self.layout` is an empty stand-in and nothing may
    /// overwrite the file on disk — `save_current_layout` refuses every
    /// write. It re-checks the file on each attempt, so fixing or deleting
    /// it by hand resumes saving without a restart.
    pub layout_poisoned: bool,
    /// Whether the one-time "layout is poisoned, not saving" warning has
    /// already been logged, so repeated saves (one per client add/remove)
    /// don't spam the log.
    layout_poison_warned: bool,
    pub drag: Option<pointer::DragState>,
    /// IPC-toggled visibility: when true, no thumbnail is shown regardless
    /// of `Visibility`/`hide_active`. The panel applet and the CLI both go
    /// through IPC, so this is the only route in; only `set_hidden` writes
    /// it.
    pub hidden: bool,
    /// When an EVE client was last seen activated. `EveFocusedOnly` keeps
    /// the thumbnails for [`rules::FOCUS_GRACE`] after that, so a click
    /// from one EVE window to another does not destroy and recreate every
    /// surface (see `rules::FOCUS_GRACE` for why that matters).
    pub last_eve_focus: Option<Instant>,
    /// The settings window while it is open (spec §6).
    pub settings: Option<settings::State>,
    /// A permanent 1×1 `Layer::Background` surface, created on the first
    /// output before any thumbnail and never destroyed by us. libcosmic
    /// binds its one clipboard to the first layer surface it creates and
    /// drops it (worker thread, display connection, every `WlOutput`
    /// binding) when that surface is destroyed, reconnecting on the next
    /// create; with a thumbnail as that first surface, every hide of it
    /// (EveFocusedOnly, hide_active, logout) was a reconnect — the churn
    /// behind the SCTK-thread use-after-free. Its id is never a client's,
    /// so `client_for_surface` and the pointer path ignore it.
    pub keepalive: Option<SurfaceId>,
    /// When `save_config` last wrote `config.ron`, and what it wrote. The
    /// watcher re-reads our own write within its 200 ms debounce; a
    /// live-only change (a slider still being dragged) made meanwhile is
    /// not in the file, and applying the re-read would revert it. A
    /// `ConfigChanged` carrying exactly what we wrote, this soon after, is
    /// that echo and is ignored (see `OWN_WRITE_ECHO`).
    pub last_config_write: Option<(Instant, Config)>,
    /// The `.conf` a tunnel install/uninstall/connect/disconnect started
    /// from, while that action runs on the blocking pool. It lives here and
    /// not in the window's state because the window can be closed and
    /// reopened while pkexec is still asking for the password: the new
    /// window must start out busy, must not start a second pkexec, and the
    /// note at the end must name the file the action *started* with.
    pub tunnel_in_flight: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Wayland(WaylandEvent),
    Backend(Event),
    Pointer(SurfaceId, mouse::Event),
    ConfigChanged(Config),
    /// `config.ron` changed on disk but cannot be read or parsed: the live
    /// config stays, and the settings window (if open) shows the reason.
    ConfigBroken(String),
    Ipc(ipc::IpcEvent),
    /// The answer to an IPC request that could not be produced on the update
    /// thread (see [`Reply::Later`]). Carries the request's one-shot reply
    /// handle, so the answer still reaches exactly the client that asked.
    IpcReplyLater(ipc::Responder, Result<Option<String>, String>),
    /// A `quit` that had a live tunnel to wind down first: the
    /// `systemctl stop` has finished (or failed), and the daemon may now
    /// drop its socket and exit. The client that asked was answered `ok`
    /// the moment the request arrived — it is not waiting on this.
    QuitAfterTunnel(Result<(), String>),
    /// [`rules::FOCUS_GRACE`] has passed since focus left EVE: hide the
    /// thumbnails if it has not come back.
    FocusGraceOver,
    Adopt(adopt::AdoptEvent),
    /// Files were dropped on one of our windows. Only the settings window
    /// cares (the Tunnel page takes a `.conf` this way); `FileHovered` is
    /// deliberately *not* a message — it repeats for every pointer motion
    /// while the drag is over the window.
    FileDropped(SurfaceId, Vec<PathBuf>),
    Settings(settings::Msg),
}

/// How an IPC request is answered. Every request produces exactly one
/// `Response`: either here on the update thread, or — for requests that
/// would otherwise block it — from the `Msg::IpcReplyLater` the returned
/// `Task` resolves to. If the app quits before that task resolves, the task
/// (and with it the `Responder`) is dropped, the one-shot sender closes and
/// the waiting client is told "no reply from app"; nothing panics.
enum Reply {
    /// Answer now, from this call.
    Now(Result<Option<String>, String>),
    /// The `Task` returned alongside owns the reply handle and will answer.
    Later,
}

fn response_of(result: Result<Option<String>, String>) -> crate::ipc::Response {
    match result {
        Ok(None) => crate::ipc::Response::Ok,
        Ok(Some(data)) => crate::ipc::Response::OkData(data),
        Err(m) => crate::ipc::Response::Err(m),
    }
}

/// How long after our own `save_config` a `ConfigChanged` carrying exactly
/// what we wrote is taken for the watcher's echo of that write (its
/// debounce is 200 ms) rather than an edit by hand.
const OWN_WRITE_ECHO: Duration = Duration::from_millis(500);

impl App {
    /// Hand a command to the backend. `false` when it could not be sent —
    /// before the backend has handed over its channel (the first moments
    /// after start), or after it closed — so a caller that answers someone
    /// can say so instead of reporting success for nothing.
    fn send(&self, cmd: Cmd) -> bool {
        match &self.cmd {
            Some(sender) => match sender.send(cmd) {
                Ok(()) => true,
                Err(err) => {
                    tracing::error!("backend command channel closed: {err}");
                    false
                }
            },
            None => {
                tracing::warn!("backend not ready; dropping command");
                false
            }
        }
    }

    /// Integer scale of the output the thumbnail actually sits on (floating
    /// mode may place it on a different output than the client's own
    /// window, e.g. a saved position on another connector), falling back to
    /// the client's own output when the thumbnail's isn't resolvable.
    fn scale_for(&self, handle: &Handle, client: &Client) -> i32 {
        self.output_for_thumb(handle)
            .or_else(|| self.output_for(&client.info))
            .and_then(|handle| self.outputs.iter().find(|o| o.handle == handle))
            .map_or(1, |o| o.scale)
    }

    /// Tell the backend the physical size to render this client's frames at
    /// and the mask radius to use, both scaled by the client's own output
    /// (outputs may differ in scale, so the radius is per client too).
    /// Nothing is sent before the backend has handed over its channel; the
    /// `CmdSender` handler replays every sized client then.
    fn send_thumb_size(&self, handle: &Handle, logical: (u32, u32)) {
        if self.cmd.is_none() {
            return;
        }
        let Some(client) = self.clients.get(handle) else { return };
        let s = self.scale_for(handle, client) as u32;
        let physical = (logical.0 * s, logical.1 * s);
        let radius = self.config.corner_radius * s;
        tracing::debug!(?handle, ?logical, scale = s, ?physical, radius, "thumb size");
        self.send(Cmd::SetThumbSize(handle.clone(), physical, radius));
    }

    /// Re-send `send_thumb_size` for every client that has a surface (its
    /// `last_size` is the logical size it was created or resized at). Used
    /// when something that feeds the command changes globally: the channel
    /// arriving, or the configured corner radius.
    fn resend_thumb_sizes(&self) {
        for (h, c) in &self.clients {
            if let Some(size) = c.last_size.filter(|_| c.surface.is_some()) {
                self.send_thumb_size(h, size);
            }
        }
    }

    fn on_output(&mut self, event: OutputEvent, output: WlOutput) {
        // Recover iced's connection from the first output we see.
        if self.conn.is_none()
            && let Some(backend) = output.backend().upgrade()
        {
            self.conn = Some(Connection::from_backend(backend));
        }
        match event {
            OutputEvent::Created(Some(info)) | OutputEvent::InfoUpdate(info) => {
                let name = info.name.clone().unwrap_or_else(|| format!("output-{}", info.id));
                let logical_size = info.logical_size.unwrap_or((0, 0));
                let scale = info.scale_factor.max(1);
                if let Some(existing) = self.outputs.iter_mut().find(|o| o.handle == output) {
                    existing.name = name;
                    existing.logical_size = logical_size;
                    existing.scale = scale;
                } else {
                    tracing::info!(%name, ?logical_size, scale, "output");
                    self.outputs.push(Output { handle: output, name, logical_size, scale });
                }
            }
            OutputEvent::Created(None) => {}
            OutputEvent::Removed => self.outputs.retain(|o| o.handle != output),
        }
    }

    fn any_client_activated(&self) -> bool {
        self.clients.values().any(|c| c.info.activated)
    }

    /// EVE has focus, or had it within [`rules::FOCUS_GRACE`].
    fn eve_focused(&self) -> bool {
        rules::eve_focused(
            self.any_client_activated(),
            self.last_eve_focus.map(|t| t.elapsed()),
            rules::FOCUS_GRACE,
        )
    }

    /// After a client update: remember an activation, and when focus has
    /// just left EVE (`was_focused`: a client was activated before the
    /// update), start the grace *now* and arrange a second look once it is
    /// over (the surfaces are kept until then). See
    /// [`rules::grace_after_update`] for why the stamp has to be taken on
    /// the transition and not left at the last event seen while focused.
    fn note_activation(&mut self, was_focused: bool) -> Task<cosmic::Action<Msg>> {
        let (stamp, timer) = rules::grace_after_update(self.any_client_activated(), was_focused, self.eve_focused());
        if stamp {
            self.last_eve_focus = Some(Instant::now());
        }
        if !timer {
            return Task::none();
        }
        // The sleep is created inside the future: `tokio::time::sleep`
        // wants a runtime at construction, and this runs on the update
        // thread (and in tests, where there is none).
        cosmic::iced::Task::perform(
            async { tokio::time::sleep(rules::FOCUS_GRACE + Duration::from_millis(20)).await },
            |()| cosmic::Action::App(Msg::FocusGraceOver),
        )
    }

    fn should_show(&self, client: &Client) -> bool {
        rules::should_show(
            self.config.visibility,
            self.config.hide_active,
            self.hidden,
            self.eve_focused(),
            client.info.activated,
        )
    }

    /// Bring the backend's capture state for `h` in line with `show`: pause
    /// when it should not be shown but is running, resume when it should be
    /// but is paused. An edge on `Client::paused` (what we last told the
    /// backend), so a client that is born hidden gets paused, and a client
    /// that merely moves between floating and dock gets nothing.
    fn sync_capture(&mut self, h: &Handle, show: bool) {
        let Some(c) = self.clients.get(h) else { return };
        if let Some(pause) = rules::capture_transition(show, c.paused) {
            self.send(if pause { Cmd::PauseCapture(h.clone()) } else { Cmd::ResumeCapture(h.clone()) });
            self.clients.get_mut(h).unwrap().paused = pause;
        }
    }

    /// Make what is on screen match `should_show`: every shown client has a
    /// surface, no other client does. Capture is paused for a client when it
    /// should not be shown and resumed when it should (see `sync_capture`).
    /// In dock mode the surviving surfaces are then re-laid out along the
    /// edge, since the set on each output may have changed.
    fn reconcile_surfaces(&mut self) -> Task<cosmic::Action<Msg>> {
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        let mut tasks = Vec::new();
        for h in handles {
            let show = self.should_show(&self.clients[&h]);
            self.sync_capture(&h, show);
            if show && self.clients[&h].surface.is_none() {
                tasks.push(self.create_surface(&h));
            } else if !show && self.clients[&h].surface.is_some() {
                tasks.push(self.destroy_surface(&h));
            }
        }
        tasks.push(self.relayout_dock());
        Task::batch(tasks)
    }

    /// Layout order (spec §7) of every client for which `keep` is true,
    /// honouring the layout's recorded `order` in dock mode.
    fn ordered(&self, mode: Mode, keep: impl Fn(&Handle, &Client) -> bool) -> Vec<Handle> {
        let items = self
            .clients
            .iter()
            .filter(|(h, c)| keep(h, c))
            .map(|(h, c)| rules::FocusItem {
                handle: h.clone(),
                label: c.info.login.label().to_string(),
                output: self.output_name_of(h).unwrap_or_default(),
                position: c.position,
            })
            .collect();
        rules::focus_order(mode, &self.layout.order, items)
    }

    /// Layout order of every known client (spec §7).
    fn focus_order(&self) -> Vec<Handle> {
        self.ordered(self.config.mode, |_, _| true)
    }

    /// Character names of every known client in layout order — the `order`
    /// list `current.ron` records (spec §9).
    fn layout_order_names(&self) -> Vec<String> {
        self.focus_order()
            .iter()
            .filter_map(|h| match &self.clients.get(h)?.info.login {
                Login::LoggedIn(name) => Some(name.clone()),
                Login::LoggingIn => None,
            })
            .collect()
    }

    /// Write `current.ron` — spec §9 auto-saves it on every drag, pin or
    /// order change — refreshing the recorded `order` and
    /// `new_client_anchor` first. Refuses outright while the file on disk
    /// is poisoned (spec §10: never overwrite a hand-edited file that
    /// failed to parse), warning about it exactly once.
    ///
    /// The refresh happens whether or not the write does: `settings_save_as`
    /// copies `self.layout` to a named file, which is a different file and
    /// so always allowed — it must not carry a stale order or anchor just
    /// because `current.ron` is poisoned.
    fn save_current_layout(&mut self) {
        let live = self.layout_order_names();
        let (order, anchor) = layout::refresh_order(&live, &self.layout.order, &self.layout.thumbs);
        self.layout.order = order;
        self.layout.new_client_anchor = anchor;
        // Only worth a stat+parse while we believe the file is poisoned;
        // a hand that fixed (or deleted) it must not have to restart us.
        let readable_now = self.layout_poisoned.then(|| Layout::try_load().is_ok());
        match Layout::save_gate(self.layout_poisoned, self.layout_poison_warned, readable_now) {
            layout::SaveGate::RefuseAndWarn => {
                tracing::warn!(
                    "{} is unparseable; not overwriting it — fix or delete it by hand and saving resumes",
                    layout::current_path().display()
                );
                self.layout_poison_warned = true;
                return;
            }
            layout::SaveGate::RefuseSilently => return,
            layout::SaveGate::Unpoison => {
                tracing::info!("{} parses again; resuming layout saves", layout::current_path().display());
                self.layout_poisoned = false;
                self.layout_poison_warned = false;
                if let Some(state) = self.settings.as_mut() {
                    state.layout_error = None;
                }
            }
            layout::SaveGate::Proceed => {}
        }
        if let Err(e) = self.layout.save() {
            tracing::warn!("cannot save layout: {e:#}");
        }
    }

    fn active_client(&self) -> Option<Handle> {
        self.clients.iter().find(|(_, c)| c.info.activated).map(|(h, _)| h.clone())
    }

    /// Execute one IPC request. `Err` is the text sent back after `err `.
    fn handle_request(
        &mut self,
        request: &crate::ipc::Request,
        reply: &ipc::Responder,
    ) -> (Reply, Task<cosmic::Action<Msg>>) {
        use crate::ipc::Request;
        match request {
            Request::Focus(n) => {
                let order = self.focus_order();
                match n.checked_sub(1).and_then(|i| order.get(i)) {
                    Some(h) => (Reply::Now(self.activate(h.clone())), Task::none()),
                    None => (Reply::Now(Err(format!("no client {n} ({} known)", order.len()))), Task::none()),
                }
            }
            Request::Next | Request::Prev => {
                let order = self.focus_order();
                let active = self.active_client();
                match rules::step(&order, active.as_ref(), matches!(request, Request::Next)) {
                    Some(h) => (Reply::Now(self.activate(h)), Task::none()),
                    None => (Reply::Now(Err("no clients".into())), Task::none()),
                }
            }
            Request::Show => (Reply::Now(Ok(None)), self.set_hidden(false)),
            Request::Hide => (Reply::Now(Ok(None)), self.set_hidden(true)),
            Request::Toggle => {
                let h = !self.hidden;
                (Reply::Now(Ok(None)), self.set_hidden(h))
            }
            Request::Layout(name) => match self.apply_layout(name) {
                Ok(task) => (Reply::Now(Ok(None)), task),
                Err(msg) => (Reply::Now(Err(msg)), Task::none()),
            },
            Request::Layouts => match serde_json::to_string(&layout::list_names()) {
                Ok(json) => (Reply::Now(Ok(Some(json))), Task::none()),
                Err(e) => (Reply::Now(Err(format!("layouts: {e}"))), Task::none()),
            },
            Request::Settings => (Reply::Now(Ok(None)), self.open_settings()),
            // Quitting takes the tunnel with it (applet spec §4.5). The
            // client is answered `ok` before anything slow happens: a
            // `systemctl stop` can take the unit's whole TimeoutStopSec
            // (10 s), and neither the caller nor the thumbnails may hang on
            // it, so the stop runs on the blocking pool and the exit itself
            // waits for `Msg::QuitAfterTunnel` (see `quit`).
            Request::Quit => {
                // Two `stat`s decide the plan, not the full status: that
                // one can spawn `systemctl`, which has no place here.
                let plan = crate::tunnel::control::quit_plan(
                    crate::tunnel::control::installed(),
                    crate::tunnel::control::iface_present(),
                );
                self.quit(plan, reply)
            }
            // The applet polls this every 5 s (1 s with its popup open),
            // and the tunnel half can spawn `systemctl is-failed` (unit
            // installed, link down — the ordinary state): a fork/exec and a
            // D-Bus round trip, up to a second when systemd is slow. The
            // client list is taken here; the rest runs on the blocking pool
            // and the answer comes back as `IpcReplyLater`.
            Request::Status => {
                let order = self.focus_order();
                let clients: Vec<crate::tunnel::status::ClientStatus> = order
                    .iter()
                    .filter_map(|h| self.clients.get(h))
                    .map(|c| crate::tunnel::status::ClientStatus { name: c.info.login.label().to_string(), active: c.info.activated })
                    .collect();
                let hidden = self.hidden;
                let location = self.config.tunnel.location.clone();
                let reply = reply.clone();
                let task = cosmic::iced::Task::perform(
                    async move {
                        let tunnel =
                            tokio::task::spawn_blocking(move || crate::tunnel::control::current_tunnel_status(&location))
                                .await
                                .map_err(|e| format!("status task failed: {e}"))?;
                        let status = crate::tunnel::status::Status { clients, hidden, tunnel };
                        serde_json::to_string(&status).map(Some).map_err(|e| format!("status: {e}"))
                    },
                    move |result| cosmic::Action::App(Msg::IpcReplyLater(reply, result)),
                );
                (Reply::Later, task)
            }
            // `systemctl start|stop` is a synchronous subprocess that can
            // take up to the unit's TimeoutStopSec (10 s). Running it here
            // would freeze every thumbnail for that long, so it goes to the
            // blocking pool and the answer comes back as `IpcReplyLater`.
            Request::TunnelConnect | Request::TunnelDisconnect => {
                let connect = matches!(request, Request::TunnelConnect);
                let reply = reply.clone();
                let task = cosmic::iced::Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            if connect {
                                crate::tunnel::control::connect()
                            } else {
                                crate::tunnel::control::disconnect()
                            }
                        })
                        .await
                        .map_err(|e| format!("tunnel task failed: {e}"))
                        .and_then(|r| r.map(|()| None).map_err(|e| format!("{e:#}")))
                    },
                    move |result| cosmic::Action::App(Msg::IpcReplyLater(reply, result)),
                );
                (Reply::Later, task)
            }
        }
    }

    /// `focus`/`next`/`prev`: the activation, as an IPC answer. A hotkey
    /// pressed before the backend is ready does nothing, and must say so.
    fn activate(&self, handle: Handle) -> Result<Option<String>, String> {
        if self.send(Cmd::Activate(handle)) { Ok(None) } else { Err("backend not ready".into()) }
    }

    /// IPC `quit`, once the plan is known.
    fn quit(&self, plan: crate::tunnel::control::QuitPlan, reply: &ipc::Responder) -> (Reply, Task<cosmic::Action<Msg>>) {
        match plan {
            // Answered from the task that exits, not from this update: the
            // connection task still has to write the reply, and an exit
            // returned alongside it raced that write — `yutani quit` could
            // see EOF ("no reply") and exit 1 after a successful quit. A
            // short sleep lets the write happen; then the socket goes and
            // the exit follows.
            crate::tunnel::control::QuitPlan::ExitNow => {
                let reply = reply.clone();
                let task = cosmic::iced::Task::perform(
                    async move {
                        reply.respond(crate::ipc::Response::Ok);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        ipc::remove_socket();
                    },
                    |()| cosmic::Action::None,
                )
                .chain(cosmic::iced::exit());
                (Reply::Later, task)
            }
            crate::tunnel::control::QuitPlan::DisconnectThenExit => {
                let task = cosmic::iced::Task::perform(
                    async move {
                        tokio::task::spawn_blocking(crate::tunnel::control::disconnect)
                            .await
                            .map_err(|e| format!("tunnel task failed: {e}"))
                            .and_then(|r| r.map_err(|e| format!("{e:#}")))
                    },
                    |result| cosmic::Action::App(Msg::QuitAfterTunnel(result)),
                );
                (Reply::Now(Ok(None)), task)
            }
        }
    }

    /// The one place `hidden` changes (every IPC request comes through here).
    fn set_hidden(&mut self, hidden: bool) -> Task<cosmic::Action<Msg>> {
        if self.hidden == hidden {
            return Task::none();
        }
        self.hidden = hidden;
        self.reconcile_surfaces()
    }

    /// Output to show a client on: the one it is on, else the first known.
    fn output_for(&self, info: &ClientInfo) -> Option<WlOutput> {
        info.outputs
            .iter()
            .find(|o| self.outputs.iter().any(|known| known.handle == **o))
            .cloned()
            .or_else(|| self.outputs.first().map(|o| o.handle.clone()))
    }

    /// Where a thumbnail whose character is not known yet goes (spec §4):
    /// the layout's `new_client_anchor`, then `STACK_STEP` px down-right
    /// per occupied slot — occupied being any shown thumbnail **and** any
    /// saved position, so a new client no longer lands on top of a
    /// character's saved spot.
    fn next_position(&self) -> (i32, i32) {
        let anchor = &self.layout.new_client_anchor;
        let mut taken: Vec<(i32, i32)> =
            self.clients.values().filter(|c| c.surface.is_some()).map(|c| c.position).collect();
        taken.extend(self.layout.thumbs.values().map(|t| (t.x, t.y)));
        layout::stacked_position((anchor.x, anchor.y), &taken, layout::STACK_STEP)
    }

    /// Layer-surface size for `client`, zoomed if hovered.
    fn surface_size(&self, client: &Client) -> (u32, u32) {
        thumbnail::zoomed_size(&self.config, client.image.as_ref(), client.hovered)
    }

    /// Floating position for `handle`: this client's own previous placement
    /// (destroy+recreate cycles like hide_active, EveFocusedOnly, or output
    /// loss should put the thumbnail back where it was even if never
    /// persisted); otherwise a saved position for this character; otherwise
    /// the next free slot.
    fn floating_position(&self, handle: &Handle) -> ((i32, i32), bool) {
        let client = &self.clients[handle];
        let saved = match &client.info.login {
            Login::LoggedIn(name) => self.layout.thumbs.get(name).cloned(),
            Login::LoggingIn => None,
        };
        rules::choose_position(
            client.placed.then_some((client.position, client.pinned)),
            saved.as_ref(),
            self.next_position(),
        )
    }

    fn create_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        if client.surface.is_some() || !self.should_show(client) {
            return Task::none();
        }
        let Some(output) = self.output_for_thumb(handle) else {
            tracing::debug!("no outputs yet; deferring surface");
            return Task::none();
        };
        let output_name =
            self.outputs.iter().find(|o| o.handle == output).map(|o| o.name.clone()).unwrap_or_default();
        let (width, height) = self.surface_size(client);
        let id = SurfaceId::unique();
        let position = match self.config.mode {
            Mode::Floating => {
                let (position, pinned) = self.floating_position(handle);
                let client = self.clients.get_mut(handle).unwrap();
                client.pinned = pinned;
                client.placed = true;
                position
            }
            Mode::Dock => {
                // Register the surface first so the layout counts this
                // client among its output's docked ones. `placed` stays
                // false: dock coordinates must never be reused as floating
                // ones.
                self.clients.get_mut(handle).unwrap().surface = Some(id);
                self.dock_position_of(handle)
            }
        };
        let client = self.clients.get_mut(handle).unwrap();
        client.surface = Some(id);
        client.position = position;
        client.last_size = Some((width, height));
        client.output = output_name;
        tracing::info!(?id, x = position.0, y = position.1, width, height, "create_surface");
        self.send_thumb_size(handle, (width, height));
        let create = get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: IcedOutput::Output(output),
            namespace: "yutani".into(),
            margin: IcedMargin { top: position.1, left: position.0, ..Default::default() },
            size: Some((Some(width), Some(height))),
            // Floating: -1, so the surface may overlap the panel's exclusive
            // strip and reach the screen's real edge; Dock: 0, so it does
            // not. Changed in place by `apply_config` on a mode switch.
            exclusive_zone: self.config.mode.exclusive_zone(),
            // This pinned iced ignores size_limits for layer surfaces; NONE is harmless.
            size_limits: Limits::NONE,
            ..Default::default()
        });
        create
    }

    /// Put up the keepalive surface (see `App::keepalive`) if it is not up
    /// and there is an output to put it on. Batched *ahead of* the reconcile
    /// that follows an output event, so its `get_layer_surface` reaches
    /// libcosmic before any thumbnail's and the clipboard binds to it once:
    /// `Task::batch` hands immediately-ready actions on in push order.
    /// 1×1 on the background layer, top-left, no keyboard: the compositor
    /// never shows it and the pointer never finds it.
    fn ensure_keepalive(&mut self) -> Task<cosmic::Action<Msg>> {
        if self.keepalive.is_some() || self.outputs.is_empty() {
            return Task::none();
        }
        let id = SurfaceId::unique();
        self.keepalive = Some(id);
        tracing::info!(?id, "keepalive surface");
        get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Background,
            keyboard_interactivity: KeyboardInteractivity::None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: IcedOutput::Active,
            namespace: "yutani-keepalive".into(),
            size: Some((Some(1), Some(1))),
            exclusive_zone: -1,
            size_limits: Limits::NONE,
            ..Default::default()
        })
    }

    fn destroy_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        match self.forget_surface(handle) {
            Some(id) => {
                tracing::info!(?id, "destroy_surface");
                destroy_layer_surface(id)
            }
            None => Task::none(),
        }
    }

    /// Clear `client.surface` (if set) along with any hover/drag state tied
    /// to it, returning the surface id that was there. A surface that goes
    /// away — destroyed by us or closed by the compositor — must not leave
    /// `hovered` set, or the next `create_surface` for this client comes back
    /// zoomed; and a drag on it must not linger and block future drags.
    fn forget_surface(&mut self, handle: &Handle) -> Option<SurfaceId> {
        let id = self.clients.get_mut(handle).and_then(|c| c.surface.take())?;
        if self.drag.as_ref().is_some_and(|d| d.surface == id) {
            self.drag = None;
        }
        if let Some(c) = self.clients.get_mut(handle) {
            c.hovered = false;
            c.last_cursor = Point::ORIGIN;
            c.output.clear();
        }
        Some(id)
    }

    fn client_for_surface(&self, id: SurfaceId) -> Option<Handle> {
        self.clients.iter().find(|(_, c)| c.surface == Some(id)).map(|(h, _)| h.clone())
    }

    fn rect_of(&self, client: &Client) -> Rect {
        let (w, h) = thumbnail::size(&self.config, client.image.as_ref());
        Rect { x: client.position.0, y: client.position.1, w: w as i32, h: h as i32 }
    }

    /// The output this client's thumbnail belongs on. In floating mode the
    /// layout decides: the character's saved connector, else the anchor's,
    /// resolved against what is actually connected — an unplugged
    /// connector falls back to the primary output at the same x/y (spec
    /// §9). Otherwise (dock mode, or nothing saved) the output the
    /// client's own window is on, else the first output we know of.
    fn output_for_thumb(&self, handle: &Handle) -> Option<WlOutput> {
        let client = self.clients.get(handle)?;
        if self.config.mode == Mode::Floating {
            let saved = match &client.info.login {
                Login::LoggedIn(name) => self.layout.thumbs.get(name).map(|t| t.output.as_str()),
                Login::LoggingIn => None,
            };
            let anchor = Some(self.layout.new_client_anchor.output.as_str()).filter(|o| !o.is_empty());
            if let Some(wanted) = saved.or(anchor) {
                let connected: Vec<String> = self.outputs.iter().map(|o| o.name.clone()).collect();
                if let Some(resolved) = layout::resolve_output(wanted, &connected) {
                    return self.outputs.iter().find(|o| o.name == resolved).map(|o| o.handle.clone());
                }
            }
        }
        self.output_for(&client.info)
    }

    fn output_name_of(&self, handle: &Handle) -> Option<String> {
        self.output_for_thumb(handle)
            .and_then(|o| self.outputs.iter().find(|k| k.handle == o).map(|k| k.name.clone()))
    }

    /// Remember this client's position (and pin state) under its character
    /// name, then rewrite `current.ron`. Floating only: dock positions come
    /// from the layout policy.
    fn persist_position(&mut self, handle: &Handle) {
        if self.config.mode == Mode::Dock {
            return;
        }
        let Some(client) = self.clients.get(handle) else { return };
        let Login::LoggedIn(name) = client.info.login.clone() else {
            tracing::debug!("position not saved: character name not resolved yet");
            return;
        };
        let (position, pinned) = (client.position, client.pinned);
        // Under the output the surface is actually on (see `recorded_output`).
        let Some(output) = rules::recorded_output(&client.output, self.output_name_of(handle)) else { return };
        self.layout
            .thumbs
            .insert(name, ThumbPos { output, x: position.0, y: position.1, pinned });
        self.save_current_layout();
    }

    /// If we have a saved position for this character, move there. Floating
    /// only: in dock mode the saved spot applies when (if) the mode changes.
    /// The same resolution as `reposition_to_layout`: a layer surface is
    /// bound to one output for life, so a saved spot on another output
    /// (the thumbnail went up on the client's own output while the name
    /// was unknown) is a destroy + recreate there, not a `set_margin` that
    /// would put DP-2 coordinates on DP-1.
    fn apply_saved_position(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        if self.config.mode == Mode::Dock {
            return Task::none();
        }
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Login::LoggedIn(name) = &client.info.login else { return Task::none() };
        let Some(saved) = self.layout.thumbs.get(name).cloned() else { return Task::none() };
        let connected: Vec<String> = self.outputs.iter().map(|o| o.name.clone()).collect();
        let Some(p) = layout::placement(&saved, &connected, &client.output) else { return Task::none() };
        let client = self.clients.get_mut(handle).unwrap();
        client.position = (p.x, p.y);
        client.pinned = p.pinned;
        let surface = client.surface;
        match surface {
            // `output_for_thumb` now resolves to the saved output, so the
            // reconcile recreates it there.
            Some(_) if p.recreate => {
                let destroy = self.destroy_surface(handle);
                Task::batch([destroy, self.reconcile_surfaces()])
            }
            // A drag canvas surface must keep its enlarged size until the
            // drag ends; a margin here would shrink it out from under the drag.
            Some(id) if !self.in_canvas(id) => set_margin(id, p.y, 0, 0, p.x),
            _ => Task::none(),
        }
    }

    /// Apply a named layout (spec §9): copy it to `current.ron` and move
    /// every live thumbnail to the spot it names. `Err` is the text the IPC
    /// reply sends after `err `.
    fn apply_layout(&mut self, name: &str) -> Result<Task<cosmic::Action<Msg>>, String> {
        let loaded = Layout::load_named(name)?;
        tracing::info!(name, thumbs = loaded.thumbs.len(), "applying layout");
        self.layout = loaded;
        match self.layout.save() {
            // Applying a layout deliberately replaces `current.ron`, so a
            // file that failed to parse at startup is gone now: un-poison,
            // and re-arm the one-time warning for a future poisoning.
            Ok(()) => {
                self.layout_poisoned = false;
                self.layout_poison_warned = false;
                if let Some(state) = self.settings.as_mut() {
                    state.layout_error = None;
                }
            }
            // The write failed, so whatever was on disk is still there —
            // stay poisoned if we were.
            Err(e) => tracing::warn!("cannot copy the layout to current.ron: {e:#}"),
        }
        Ok(self.reposition_to_layout())
    }

    /// Move every live thumbnail to where `self.layout` puts it. A
    /// thumbnail whose output changed is destroyed and left to
    /// `reconcile_surfaces`, which recreates it — `output_for_thumb` now
    /// resolves to the layout's output. In dock mode positions are a
    /// policy, so only the arrangement (the recorded `order`) changes.
    fn reposition_to_layout(&mut self) -> Task<cosmic::Action<Msg>> {
        if self.config.mode == Mode::Dock {
            return self.relayout_dock();
        }
        let connected: Vec<String> = self.outputs.iter().map(|o| o.name.clone()).collect();
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        let mut tasks = Vec::new();
        for h in handles {
            let Some(client) = self.clients.get(&h) else { continue };
            let Login::LoggedIn(name) = client.info.login.clone() else { continue };
            let current = client.output.clone();
            let Some(saved) = self.layout.thumbs.get(&name).cloned() else { continue };
            let Some(p) = layout::placement(&saved, &connected, &current) else { continue };
            let surface = {
                let c = self.clients.get_mut(&h).unwrap();
                c.position = (p.x, p.y);
                c.pinned = p.pinned;
                c.placed = true;
                c.surface
            };
            match surface {
                Some(_) if p.recreate => tasks.push(self.destroy_surface(&h)),
                // A drag canvas keeps its margin; `leave_canvas` applies the
                // new position at drag end.
                Some(id) if !self.in_canvas(id) => tasks.push(set_margin(id, p.y, 0, 0, p.x)),
                _ => {}
            }
        }
        tasks.push(self.reconcile_surfaces());
        Task::batch(tasks)
    }

    /// Leaving dock mode: every surface goes back to a floating spot — the
    /// character's saved position, else the next free slot. `placed` was
    /// cleared on entering dock mode, so dock coordinates are never reused.
    /// Handles are visited in dock order so slot assignment is deterministic.
    fn refloat_surfaces(&mut self) -> Task<cosmic::Action<Msg>> {
        let handles = self.ordered(Mode::Dock, |_, c| c.surface.is_some());
        let mut tasks = Vec::new();
        for h in handles {
            let (position, pinned) = self.floating_position(&h);
            let c = self.clients.get_mut(&h).unwrap();
            c.position = position;
            c.pinned = pinned;
            c.placed = true;
            let id = c.surface.unwrap();
            tracing::info!(?id, x = position.0, y = position.1, "leaving dock: floating position");
            if !self.in_canvas(id) {
                tasks.push(set_margin(id, position.1, 0, 0, position.0));
            }
        }
        Task::batch(tasks)
    }

    /// Enlarge the surface to cover its whole output so that, while dragging,
    /// surface-local pointer coordinates are absolute. The surface itself no
    /// longer moves; `view_window` draws the thumbnail at `client.position`.
    fn enter_canvas(id: SurfaceId) -> Task<cosmic::Action<Msg>> {
        Task::batch([
            set_anchor(id, Anchor::all()),
            set_margin(id, 0, 0, 0, 0),
            set_size(id, None, None),
        ])
    }

    /// Undo `enter_canvas`: back to a thumbnail-sized surface at the client's
    /// current position. Order matters: anchor, size, margin.
    fn leave_canvas(&mut self, id: SurfaceId, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        // `client.hovered` is set true before this is called (the cursor is
        // over the thumbnail at drag end), so this restores at zoomed size.
        let (w, h) = self.surface_size(client);
        let (x, y) = client.position;
        self.clients.get_mut(handle).unwrap().last_size = Some((w, h));
        self.send_thumb_size(handle, (w, h));
        Task::batch([
            set_anchor(id, Anchor::TOP | Anchor::LEFT),
            set_size(id, Some(w), Some(h)),
            set_margin(id, y, 0, 0, x),
        ])
    }

    /// True while `id` is enlarged to a drag canvas: a drag on this surface
    /// has crossed the arming threshold (`phase != Idle`) and is not pinned.
    fn in_canvas(&self, id: SurfaceId) -> bool {
        self.drag.as_ref().is_some_and(|d| d.surface == id && !d.pinned && d.phase != pointer::Phase::Idle)
    }

    /// Resize `handle`'s surface to its current target size, but only if it
    /// isn't already that size and it isn't a full-output drag canvas.
    /// `None` when nothing was sent — the size is unchanged — so a caller
    /// can skip the dock relayout that only a size change can move.
    fn resize_if_needed(&mut self, handle: &Handle) -> Option<Task<cosmic::Action<Msg>>> {
        let client = self.clients.get(handle)?;
        let id = client.surface?;
        if self.in_canvas(id) {
            return None;
        }
        let size = self.surface_size(client);
        if client.last_size == Some(size) {
            return None;
        }
        self.clients.get_mut(handle).unwrap().last_size = Some(size);
        self.send_thumb_size(handle, size);
        Some(set_size(id, Some(size.0), Some(size.1)))
    }

    fn on_pointer(&mut self, id: SurfaceId, event: mouse::Event) -> Task<cosmic::Action<Msg>> {
        let Some(handle) = self.client_for_surface(id) else { return Task::none() };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Middle) => {
                // No pins in dock mode: positions come from the layout.
                if self.config.mode == Mode::Dock {
                    return Task::none();
                }
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.pinned = !c.pinned;
                }
                self.persist_position(&handle);
                Task::none()
            }
            mouse::Event::ButtonPressed(button) => {
                if self.drag.is_some() {
                    return Task::none();
                }
                let c = &self.clients[&handle];
                // Position of the cursor at press time is not part of ButtonPressed;
                // use the last CursorMoved we saw for this surface. The surface is
                // still thumbnail-sized here, so local + position is absolute.
                let cursor = c.last_cursor;
                // Dock mode: no dragging, but a release must still be a
                // click — a pinned `DragState` never leaves `Idle` and yields
                // `Click` on release.
                let pinned = c.pinned || self.config.mode == Mode::Dock;
                self.drag = pointer::on_press(id, button, cursor, c.position, pinned);
                // Idle: the canvas isn't entered until motion crosses the
                // threshold (see CursorMoved), so a plain click never
                // touches the surface at all.
                Task::none()
            }
            mouse::Event::CursorMoved { position } => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.last_cursor = position;
                }
                let Some(drag) = self.drag.as_mut().filter(|d| d.surface == id) else { return Task::none() };
                match drag.phase {
                    pointer::Phase::Idle => match pointer::on_move_local(drag, position) {
                        pointer::Outcome::StartDrag => {
                            tracing::debug!(?id, "drag: threshold crossed; arming canvas");
                            Self::enter_canvas(id)
                        }
                        _ => Task::none(),
                    },
                    pointer::Phase::Arming => Task::none(),
                    pointer::Phase::Dragging => match pointer::on_move(drag, position) {
                        pointer::Outcome::Move(raw) => {
                            let canvas = drag.canvas;
                            let me = self.rect_of(&self.clients[&handle]);
                            let others: Vec<Rect> = self
                                .clients
                                .iter()
                                .filter(|(h, c)| *h != &handle && c.surface.is_some())
                                .map(|(_, c)| self.rect_of(c))
                                .collect();
                            let grid = self.config.snap_grid.then_some(layout::SNAP_GRID);
                            let edges = self.config.snap_edges.then_some(12);
                            let (x, y) = layout::snap(Rect { x: raw.0, y: raw.1, ..me }, &others, grid, edges);
                            // Keep the thumbnail fully inside the canvas (== the output).
                            let x = x.clamp(0, (canvas.0 - me.w).max(0));
                            let y = y.clamp(0, (canvas.1 - me.h).max(0));
                            self.clients.get_mut(&handle).unwrap().position = (x, y);
                            // No set_margin: the surface stays put; the view draws the offset.
                            Task::none()
                        }
                        _ => Task::none(),
                    },
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if !self.drag.as_ref().is_some_and(|d| d.surface == id && d.button == button) {
                    return Task::none();
                }
                let drag = self.drag.take().unwrap();
                // The canvas is only entered once motion crosses the threshold
                // (`drag.moved`); if it never did, the surface never changed
                // and there's nothing to restore.
                let entered_canvas = !drag.pinned && drag.moved;
                match pointer::on_release(drag) {
                    pointer::Outcome::Click(mouse::Button::Left) => {
                        self.send(Cmd::Activate(handle.clone()));
                    }
                    pointer::Outcome::Click(mouse::Button::Right) => {
                        self.send(Cmd::Minimize(handle.clone()));
                    }
                    pointer::Outcome::DragEnd => self.persist_position(&handle),
                    _ => {}
                }
                if entered_canvas {
                    // The cursor is over the thumbnail at drag end (no CursorEntered
                    // fires for a surface that was already under the pointer), so mark
                    // it hovered before computing the restore size — leave_canvas then
                    // restores at zoomed size instead of snapping small first.
                    self.clients.get_mut(&handle).unwrap().hovered = true;
                    tracing::debug!(?id, pos = ?self.clients[&handle].position, "drag: leaving canvas");
                    self.leave_canvas(id, &handle)
                } else {
                    Task::none()
                }
            }
            mouse::Event::CursorEntered => {
                // While dragging the surface is already zoomed (canvas mode);
                // leave size/hovered alone so drag end restores correctly.
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    return Task::none();
                }
                self.clients.get_mut(&handle).unwrap().hovered = true;
                // Dock mode: a zoomed thumbnail shifts its neighbours.
                let resize = self.resize_if_needed(&handle).unwrap_or_else(Task::none);
                Task::batch([resize, self.relayout_dock()])
            }
            mouse::Event::CursorLeft => {
                // Releasing outside is delivered to us anyway (implicit grab); nothing
                // to do while dragging — canvas mode owns the size until release.
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    return Task::none();
                }
                self.clients.get_mut(&handle).unwrap().hovered = false;
                let resize = self.resize_if_needed(&handle).unwrap_or_else(Task::none);
                Task::batch([resize, self.relayout_dock()])
            }
            _ => Task::none(),
        }
    }

    fn on_backend(&mut self, event: Event) -> Task<cosmic::Action<Msg>> {
        match event {
            Event::CmdSender(sender) => {
                self.cmd = Some(sender);
                // Clients (and so surfaces) come from backend events that
                // follow this one, so nothing should be pending; replaying is
                // cheap and removes the ordering hazard should a surface ever
                // be sized before the channel exists (its send was dropped).
                self.resend_thumb_sizes();
                Task::none()
            }
            Event::ClientAdded(handle, info) | Event::ClientUpdated(handle, info) => {
                // Taken before the update lands: whether focus *leaves*
                // EVE with it is what starts the grace.
                let was_focused = self.any_client_activated();
                let entry = self.clients.entry(handle.clone()).or_insert_with(|| Client {
                    info: info.clone(),
                    image: None,
                    surface: None,
                    position: (0, 0),
                    pinned: false,
                    last_cursor: Point::ORIGIN,
                    hovered: false,
                    unavailable: false,
                    placed: false,
                    last_size: None,
                    paused: false,
                    output: String::new(),
                });
                let was_named = matches!(entry.info.login, Login::LoggedIn(_));
                entry.info = info;
                let became_named = !was_named && matches!(entry.info.login, Login::LoggedIn(_));
                let grace = self.note_activation(was_focused);
                // An activation change on one client can hide/show others, so
                // reconcile every client's surface, not just this one's.
                let reconciled = Task::batch([grace, self.reconcile_surfaces()]);
                if became_named {
                    // A new character name changes the layout order, and a
                    // saved position must apply even if the surface was just
                    // created (or doesn't exist yet): `apply_saved_position`
                    // updates position/pinned regardless, and is a no-op on
                    // margin if there's no surface.
                    self.save_current_layout();
                    Task::batch([reconciled, self.apply_saved_position(&handle)])
                } else {
                    reconciled
                }
            }
            Event::ClientRemoved(handle) => {
                let was_focused = self.any_client_activated();
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                // The activated client may be the one that closed.
                let grace = self.note_activation(was_focused);
                // The layout order changed; the saved position stays, so the
                // character comes back to the same spot next launch.
                self.save_current_layout();
                // Dock mode: its neighbours close the gap.
                Task::batch([task, grace, self.relayout_dock()])
            }
            Event::Frame(handle, image) => {
                let Some(client) = self.clients.get_mut(&handle) else { return Task::none() };
                client.image = Some(image);
                client.unavailable = false;
                if client.surface.is_some() {
                    // `resize_if_needed` skips a drag canvas and dedupes
                    // against the last size actually sent. Dock mode: only
                    // a new aspect changes this thumbnail's size and so
                    // moves its neighbours — at fps × clients frames a
                    // second, an unchanged size must not cost a relayout.
                    match self.resize_if_needed(&handle) {
                        Some(resize) => Task::batch([resize, self.relayout_dock()]),
                        None => Task::none(),
                    }
                } else {
                    // Dock mode: a first frame sizes the thumbnail.
                    let create = self.create_surface(&handle);
                    Task::batch([create, self.relayout_dock()])
                }
            }
            Event::CaptureUnavailable(handle) => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    tracing::warn!(label = c.info.login.label(), "capture unavailable");
                    c.unavailable = true;
                }
                Task::none()
            }
        }
    }

    /// Apply a freshly re-read (and validated) config. Border colours and
    /// names apply on the next redraw automatically because
    /// `view_window` reads `self.config`; sizes and visibility need pushing.
    fn apply_config(&mut self, new: Config) -> Task<cosmic::Action<Msg>> {
        if new == self.config {
            return Task::none();
        }
        tracing::info!("config changed; applying");
        if new.app_ids != self.config.app_ids {
            self.send(Cmd::SetAppIds(new.app_ids.clone()));
        }
        if new.fps != self.config.fps {
            self.send(Cmd::SetFps(new.fps));
        }
        if new.corner_radius != self.config.corner_radius {
            self.config.corner_radius = new.corner_radius;
            self.resend_thumb_sizes();
        }
        let mode_changed = new.mode != self.config.mode;
        self.config = new;
        let mut tasks = Vec::new();
        if mode_changed {
            // Surfaces survive a mode switch, so the zone they were created
            // with has to follow the mode (see `Mode::exclusive_zone`).
            let zone = self.config.mode.exclusive_zone();
            tasks.extend(self.clients.values().filter_map(|c| c.surface).map(|id| set_exclusive_zone(id, zone)));
            match self.config.mode {
                // Positions now come from the layout; forget the floating
                // ones so a later switch back doesn't reuse dock coordinates
                // (`refloat_surfaces` falls back to saved/next-slot instead).
                Mode::Dock => {
                    self.clients.values_mut().for_each(|c| c.placed = false);
                    // A button held down right now must not turn into a drag
                    // (a canvas already entered is left alone: its release
                    // path restores the surface at the dock position).
                    if let Some(d) = self.drag.as_mut().filter(|d| d.phase == pointer::Phase::Idle) {
                        d.pinned = true;
                    }
                }
                Mode::Floating => tasks.push(self.refloat_surfaces()),
            }
        }
        // Sizes and visibility may have changed.
        tasks.push(self.reconcile_surfaces());
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        for h in handles {
            // `resize_if_needed` skips a drag canvas (must keep its size
            // until the drag ends — `leave_canvas` applies the current size
            // then) and dedupes against the last size actually sent.
            tasks.extend(self.resize_if_needed(&h));
        }
        // Dock mode: `reconcile_surfaces` already re-laid out every surface
        // at its new size (the layout reads `surface_size`, not `last_size`)
        // and along the new edge.
        Task::batch(tasks)
    }

    /// Open the settings window, or raise the one already open (spec §6: a
    /// second `settings` request focuses it).
    fn open_settings(&mut self) -> Task<cosmic::Action<Msg>> {
        if let Some(state) = &self.settings {
            // `window::gain_focus` does nothing on Wayland; raising a
            // window is an xdg-activation token handed back to the
            // compositor (the same dance libcosmic does for `Activate`).
            let id = state.window;
            return activation::request_token(Some(Self::APP_ID.to_string()), Some(id))
                .map(|token| cosmic::Action::App(Msg::Settings(settings::Msg::Raise(token))));
        }
        let (id, open) = cosmic::iced::window::open(settings::window_settings(Self::APP_ID));
        let mut state = settings::State::new(id, &self.config);
        // A tunnel action outlives the window it was started from.
        state.tunnel.busy = self.tunnel_in_flight.is_some();
        self.settings = Some(state);
        let title = self.set_window_title("Yutani Settings".to_string(), id);
        Task::batch([title, open.map(|_| cosmic::Action::App(Msg::Settings(settings::Msg::Opened)))])
    }

    fn on_settings(&mut self, msg: settings::Msg) -> Task<cosmic::Action<Msg>> {
        use settings::Msg as S;
        // The window is gone: only `Closed` still means anything.
        if matches!(msg, S::Closed) {
            self.settings = None;
            return Task::none();
        }
        let Some(window) = self.settings.as_ref().map(|s| s.window) else { return Task::none() };

        // Messages that only move the window around.
        match &msg {
            S::Close => return cosmic::iced::window::close(window),
            S::Drag => return cosmic::iced::window::drag(window),
            S::Raise(Some(token)) => {
                // A second `settings` request also re-reads what lives
                // outside `Config` — the user may have fixed (or broken)
                // `config.ron`, or saved a layout, since the window opened.
                if let Some(state) = self.settings.as_mut() {
                    state.refresh();
                }
                // …and EVE's files with it: the Characters page is as stale
                // as the rest after the window has been sitting open.
                let characters = self.refresh_characters();
                // …and so is the tunnel: it can have been installed,
                // connected or torn down from the CLI meanwhile.
                let tunnel = self.refresh_tunnel();
                // Un-minimize first, then activate: a window the compositor
                // minimised stays minimised if it is only activated. The
                // same chain libcosmic's own `Action::Activate` does.
                return Task::batch([
                    characters,
                    tunnel,
                    cosmic::iced::window::minimize(window, false)
                        .chain(activation::activate(window, token.clone())),
                ]);
            }
            _ => {}
        }

        // Messages that only touch the window's own state.
        if let Some(state) = self.settings.as_mut() {
            match &msg {
                S::Opened | S::Raise(None) | S::Recheck => {
                    if settings::clears_note(&msg) {
                        state.note = None;
                    }
                    state.refresh();
                    // EVE's files are outside `State::refresh` (they are not
                    // ours, and the names lookup is a task) — and so is the
                    // tunnel, which is systemd's state, not a file of ours.
                    return Task::batch([self.refresh_characters(), self.refresh_tunnel()]);
                }
                S::ActiveBorder(text) => state.active_border_field = text.clone(),
                S::InactiveBorder(text) => state.inactive_border_field = text.clone(),
                S::NextKey(text) => state.next_field = text.clone(),
                S::PrevKey(text) => state.prev_field = text.clone(),
                S::Name(text) => {
                    state.name_field = text.clone();
                    return Task::none();
                }
                S::TunnelConfPath(text) => {
                    state.tunnel.conf_path = text.clone();
                    return Task::none();
                }
                S::TunnelStatus(status) => {
                    state.tunnel.status = Some((**status).clone());
                    return Task::none();
                }
                // The chooser answers with a path or with nothing
                // (cancelled); either way the note is the only feedback.
                S::TunnelConfChosen(path) => {
                    if let Some(path) = path {
                        state.tunnel.conf_path = path.display().to_string();
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        state.note = Some(format!("chose {name}"));
                    }
                    return Task::none();
                }
                // Clamped to the listing the window is showing: libcosmic
                // publishes an index past the end on ctrl+scroll, and a
                // refresh can shrink the list under a queued message.
                S::SourceCharacter(i) => {
                    if state.characters.listing.as_ref().is_some_and(|l| *i < l.characters.len()) {
                        state.characters.source_character = *i;
                    }
                    return Task::none();
                }
                S::SourceAccount(i) => {
                    if state.characters.listing.as_ref().is_some_and(|l| *i < l.accounts.len()) {
                        state.characters.source_account = *i;
                    }
                    return Task::none();
                }
                S::CopyAccount(on) => {
                    state.characters.copy_account = *on;
                    return Task::none();
                }
                S::Names(names, error) => {
                    state.characters.names_arrived(names.clone(), error.clone());
                    return Task::none();
                }
                _ => {}
            }
        }

        // Pages and actions that are not config fields.
        match &msg {
            S::Page(entity) => {
                if let Some(state) = self.settings.as_mut() {
                    state.pages.activate(*entity);
                    // The note was about the page being left behind.
                    if settings::clears_note(&msg) {
                        state.note = None;
                    }
                }
                return Task::none();
            }
            S::SaveAs => {
                self.settings_save_as();
                return Task::none();
            }
            S::Apply(name) => return self.settings_apply_layout(name.clone()),
            S::Rename(from) => {
                self.settings_rename_layout(from.clone());
                return Task::none();
            }
            S::Delete(name) => {
                self.settings_delete_layout(name.clone());
                return Task::none();
            }
            S::InstallShortcuts => {
                self.settings_shortcuts(true);
                return Task::none();
            }
            S::UninstallShortcuts => {
                self.settings_shortcuts(false);
                return Task::none();
            }
            // The note *is* the feedback for this press — there is nothing
            // else on screen to change — so it is set before the clipboard
            // task is handed back.
            S::CopySteamArgs => {
                self.settings_note("copied".to_string());
                return cosmic::iced::clipboard::write(yutani::STEAM_LAUNCH_ARGS.to_string());
            }
            // The window's own drop target answered: the Wayland route
            // into the same handler the X11 window event uses.
            S::FilesDropped(paths) => {
                self.on_files_dropped(paths);
                return Task::none();
            }
            S::RefreshCharacters => return self.refresh_characters(),
            S::RefreshTunnel => return self.refresh_tunnel(),
            S::BrowseTunnelConf => return self.browse_tunnel_conf(),
            // Re-checked here and not only on the button: the file behind
            // the enabled state can have been moved since the last redraw.
            S::InstallTunnel => {
                let blocked =
                    self.settings.as_ref().and_then(|s| tunnel_page::install_blocker(&s.tunnel));
                if let Some(reason) = blocked {
                    self.settings_note(reason.to_string());
                    return Task::none();
                }
                return self.run_tunnel_action(settings::TunnelAction::Install);
            }
            S::UninstallTunnel => return self.run_tunnel_action(settings::TunnelAction::Uninstall),
            S::TunnelConnect => return self.run_tunnel_action(settings::TunnelAction::Connect),
            S::TunnelDisconnect => return self.run_tunnel_action(settings::TunnelAction::Disconnect),
            // Whatever happened, the action is over: the buttons come back
            // and the state on screen is re-read from systemd.
            S::TunnelDone(action, result) => {
                let note = self.tunnel_done(*action, result.clone());
                self.settings_note(note);
                return self.refresh_tunnel();
            }
            // Both set the note themselves, then the listing (and the
            // newest backup) are re-read: the files on disk just changed.
            S::CopyCharacters => {
                self.settings_copy_characters();
                return self.refresh_characters();
            }
            S::RestoreBackup => {
                self.settings_restore_backup();
                return self.refresh_characters();
            }
            _ => {}
        }

        if matches!(msg, S::Commit) {
            self.save_config();
            return Task::none();
        }

        // A config field: apply live, and write `config.ron` unless this is
        // a slider still being dragged (its release sends `Commit`).
        let mut config = self.config.clone();
        match settings::apply_config_field(&mut config, &msg) {
            Err(note) => {
                self.settings_note(note);
                Task::none()
            }
            // Not a change — but a correction back to the previous valid
            // value must still drop the note the bad one left behind.
            Ok(false) => {
                self.settings_note_clear();
                Task::none()
            }
            Ok(true) => {
                self.settings_note_clear();
                let task = self.apply_config(config);
                if !settings::is_live_only(&msg) {
                    self.save_config();
                }
                task
            }
        }
    }

    /// Layouts page: save the current arrangement under the typed name.
    /// `current.ron` is refreshed first so the copy carries today's order
    /// and anchor.
    fn settings_save_as(&mut self) {
        let typed = self.settings.as_ref().map(|s| s.name_field.clone()).unwrap_or_default();
        // Checked before anything is written — including the `current.ron`
        // refresh below, which is a file operation too.
        let name = match layout::validate_name(&typed) {
            Ok(name) => name,
            Err(e) => return self.settings_note(e),
        };
        self.save_current_layout();
        match self.layout.save_named(&name) {
            Ok(()) => {
                let note = format!("saved layout {name:?}");
                if let Some(state) = self.settings.as_mut() {
                    state.name_field.clear();
                    state.layouts = layout::list_names();
                    state.note = Some(note);
                }
            }
            Err(e) => self.settings_note(e),
        }
    }

    fn settings_apply_layout(&mut self, name: String) -> Task<cosmic::Action<Msg>> {
        match self.apply_layout(&name) {
            Ok(task) => {
                self.settings_note(format!("applied layout {name:?}"));
                task
            }
            Err(e) => {
                self.settings_note(e);
                Task::none()
            }
        }
    }

    fn settings_rename_layout(&mut self, from: String) {
        let to = self.settings.as_ref().map(|s| s.name_field.clone()).unwrap_or_default();
        match layout::rename_named(&from, &to) {
            Ok(()) => {
                let note = format!("renamed {:?} to {:?}", from, to.trim());
                if let Some(state) = self.settings.as_mut() {
                    state.name_field.clear();
                    state.layouts = layout::list_names();
                    state.note = Some(note);
                }
            }
            Err(e) => self.settings_note(e),
        }
    }

    fn settings_delete_layout(&mut self, name: String) {
        match layout::delete_named(&name) {
            Ok(()) => {
                let note = format!("deleted layout {name:?}");
                if let Some(state) = self.settings.as_mut() {
                    state.layouts = layout::list_names();
                    state.note = Some(note);
                }
            }
            Err(e) => self.settings_note(e),
        }
    }

    /// Behavior page: the *Install shortcuts* / *Uninstall shortcuts*
    /// buttons of spec §6, over the same code path as
    /// `yutani shortcuts install|uninstall`.
    fn settings_shortcuts(&mut self, install: bool) {
        let note = if install {
            match crate::shortcuts::install(&self.config.shortcuts) {
                Ok((installed, wanted)) if installed == wanted => format!("installed {installed} shortcuts"),
                Ok((installed, wanted)) => {
                    format!("installed {installed} of {wanted} shortcuts (the rest are bound by something else)")
                }
                Err(e) => format!("{e:#}"),
            }
        } else {
            match crate::shortcuts::uninstall() {
                Ok(n) => format!("removed {n} shortcuts"),
                Err(e) => format!("{e:#}"),
            }
        };
        self.settings_note(note);
    }

    /// Characters page: re-read the profile listing, the newest backup and
    /// the name cache, then ask ESI for any id still unnamed (blocking pool).
    fn refresh_characters(&mut self) -> Task<cosmic::Action<Msg>> {
        // No window, nothing to refresh: the listing walk and the cache
        // read would be thrown away.
        if self.settings.is_none() {
            return Task::none();
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let override_dir = self.config.eve_settings_dir.clone();
        let listing = yutani::eve_settings::discover(override_dir.as_deref().map(Path::new), &home)
            .and_then(|dir| yutani::eve_settings::list(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display())));
        let backups = yutani::eve_settings::copy::backups_dir(&dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")));
        let cache = yutani::eve_settings::names::cache_path(&dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")));
        let cached = yutani::eve_settings::names::load_cache(&cache);
        let Some(state) = self.settings.as_mut() else { return Task::none() };
        state.characters.set_listing(listing);
        state.characters.last_backup = yutani::eve_settings::copy::latest_backup(&backups);
        state.characters.names.extend(cached);
        let missing = state.characters.unnamed();
        // Nothing to ask for, or an answer is already on its way: asking
        // twice would hit ESI twice for the same ids.
        if missing.is_empty() || state.characters.fetching {
            return Task::none();
        }
        state.characters.asking(&missing);
        cosmic::iced::Task::perform(
            async move {
                tokio::task::spawn_blocking(move || yutani::eve_settings::names::resolve(missing, cache))
                    .await
                    .unwrap_or_else(|e| (Default::default(), Some(format!("names task failed: {e}"))))
            },
            |(names, error)| cosmic::Action::App(Msg::Settings(settings::Msg::Names(names, error))),
        )
    }

    /// Characters page: the copy itself. Refused while any EVE toplevel
    /// exists — the client writes these files on logout.
    fn settings_copy_characters(&mut self) {
        let Some(state) = self.settings.as_ref() else { return };
        // Re-checked here and not only on the button: the listing behind
        // the disabled state can be a redraw old.
        if let Some(reason) = characters::copy_blocker(&state.characters, !self.clients.is_empty()) {
            return self.settings_note(characters::blocker_note(reason));
        }
        let Some(listing) = state.characters.listing.as_ref() else { return };
        let Some(character) = state.characters.selected_character() else {
            return self.settings_note(characters::blocker_note(characters::NO_SELECTION));
        };
        let account = state.characters.account_to_copy();
        // Asked for an account copy, there is one to copy to, and yet no
        // account is named: copying without it would silently do half the
        // job the toggle promised.
        if state.characters.copy_account && account.is_none() && listing.accounts.len() >= 2 {
            return self.settings_note(characters::blocker_note(characters::NO_ACCOUNT_SELECTION));
        }
        let source = state.characters.source_label();
        let backups = yutani::eve_settings::copy::backups_dir(&dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")));
        let backup = backups.join(yutani::eve_settings::copy::backup_name(SystemTime::now()));
        let note = match yutani::eve_settings::copy::plan(listing, character, account) {
            Err(e) => e,
            Ok(plan) => match yutani::eve_settings::copy::execute(&plan, &backup) {
                Ok(report) => characters::copy_note(&source, &report),
                Err(e) => characters::copy_failure_note(&e, &backup),
            },
        };
        self.settings_note(note);
    }

    /// Characters page: put the newest backup back over the profile —
    /// after backing up the live files it is about to replace. Without
    /// that, a restore is the one operation here with no way back: the
    /// settings written since the backup would be gone unrecorded.
    fn settings_restore_backup(&mut self) {
        use yutani::eve_settings::copy;
        let Some(state) = self.settings.as_ref() else { return };
        if !self.clients.is_empty() {
            return self.settings_note(characters::blocker_note(characters::RUNNING_CLIENT));
        }
        let (Some(backup), Some(listing)) = (state.characters.last_backup.clone(), state.characters.listing.as_ref())
        else {
            return self.settings_note("nothing to restore".to_string());
        };
        let dir = listing.dir.clone();
        let backups = copy::backups_dir(&dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")));
        let saved = backups.join(copy::backup_name(SystemTime::now()));
        let note = match copy::restore_plan(&backup, &dir) {
            Err(e) => format!("restore failed: {e}"),
            // Nothing in the backup matches a file in the profile. Making
            // an empty backup directory here would hide the real newest
            // backup behind it, so do nothing at all.
            Ok(targets) if targets.is_empty() => {
                format!("nothing to restore: no file in {} is in {}", backup.display(), dir.display())
            }
            Ok(targets) => match copy::backup_files(&targets, &saved).and_then(|_| copy::restore(&backup, &dir)) {
                Ok(n) => format!(
                    "restored {n} file{} from {}; the files it replaced are in {}",
                    if n == 1 { "" } else { "s" },
                    backup.display(),
                    saved.display()
                ),
                Err(e) => format!("restore failed: {e}"),
            },
        };
        self.settings_note(note);
    }

    /// Files dropped on the settings window — from its own drag-and-drop
    /// destination widget (`settings::Msg::FilesDropped`, the Wayland
    /// route) or from the X11/XWayland `FileDropped` window event. Only a
    /// `.conf` is taken: the Tunnel page's third way of naming the
    /// WireGuard file. The file itself is never opened here — the page
    /// shows its name, and `install` (as root) is the only thing that reads
    /// what is inside it.
    fn on_files_dropped(&mut self, paths: &[PathBuf]) {
        let Some(conf) = tunnel_page::conf_candidate(paths) else {
            // The drop was accepted (the cursor said so); say why nothing
            // happened rather than leaving the user to wonder.
            self.settings_note(tunnel_page::NOT_A_CONF_DROP.to_string());
            return;
        };
        let name = conf.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tab = settings::page_index(settings::Page::Tunnel);
        if let Some(state) = self.settings.as_mut() {
            state.tunnel.conf_path = conf.display().to_string();
            // The file landed on a page that does not mention it otherwise;
            // show the one that does.
            if let Some(tab) = tab {
                state.pages.activate_position(tab as u16);
            }
            state.note = Some(format!("dropped {name}; press Install tunnel"));
        }
    }

    /// One finished tunnel action: the buttons come back — whatever
    /// happened, and on every path through here — and the note says what
    /// happened. It never names anything from inside the `.conf`, only its
    /// file name.
    fn tunnel_done(&mut self, action: settings::TunnelAction, result: Result<(), String>) -> String {
        // The file the action *started* with, not whatever the field holds
        // now: the path can have been re-typed or dropped on while pkexec
        // was asking for a password, and the note would then name a file
        // that was never installed. Taken whether or not the window is
        // still there — the action is over either way.
        let conf = self.tunnel_in_flight.take().unwrap_or_default();
        let Some(state) = self.settings.as_mut() else {
            tracing::info!(?action, ok = result.is_ok(), "tunnel action finished after its window closed");
            return String::new();
        };
        state.tunnel.busy = false;
        match (action, result) {
            (settings::TunnelAction::Install, result) => tunnel_page::install_note(&conf, result),
            (_, Err(e)) => e,
            (settings::TunnelAction::Uninstall, Ok(())) => "tunnel uninstalled".to_string(),
            // `systemctl start` returns once the unit is up, but the
            // handshake behind it is not: the status line says when it is.
            (settings::TunnelAction::Connect, Ok(())) => "tunnel connecting…".to_string(),
            (settings::TunnelAction::Disconnect, Ok(())) => "tunnel disconnected".to_string(),
        }
    }

    /// Read the tunnel state off the UI thread (`is-failed` may spawn
    /// systemctl). Runs when the window opens or is raised, when Re-check is
    /// pressed, and after every action.
    fn refresh_tunnel(&self) -> Task<cosmic::Action<Msg>> {
        if self.settings.is_none() {
            return Task::none();
        }
        let location = self.config.tunnel.location.clone();
        cosmic::iced::Task::perform(
            async move {
                tokio::task::spawn_blocking(move || crate::tunnel::control::current_tunnel_status(&location))
                    .await
                    .ok()
            },
            |status| match status {
                Some(t) => cosmic::Action::App(Msg::Settings(settings::Msg::TunnelStatus(Box::new(t)))),
                None => cosmic::Action::None,
            },
        )
    }

    /// One tunnel action on the blocking pool; the page is `busy` until the
    /// `TunnelDone` it resolves to. `install` shells out to `pkexec` (the
    /// polkit agent's password prompt) and connect/disconnect to
    /// `systemctl`, either of which can take seconds — none of it may
    /// happen on the thread that draws the thumbnails.
    ///
    /// The IPC `tunnel connect|disconnect` requests run the same functions
    /// and are not covered by this flag: systemd serialises the two, and
    /// the status refresh at the end reports whatever actually happened.
    fn run_tunnel_action(&mut self, action: settings::TunnelAction) -> Task<cosmic::Action<Msg>> {
        // One at a time: a message that slips in while pkexec is up (from a
        // window reopened mid-prompt, say) must not start a second prompt.
        if self.tunnel_in_flight.is_some() {
            self.settings_note(tunnel_page::BUSY.to_string());
            return Task::none();
        }
        let Some(state) = self.settings.as_mut() else { return Task::none() };
        state.tunnel.busy = true;
        let conf = PathBuf::from(state.tunnel.conf_path.trim());
        // What the note at the end has to name; cleared by `tunnel_done`.
        self.tunnel_in_flight = Some(conf.clone());
        let (servers, domains) = (self.config.tunnel.dns_servers.clone(), self.config.tunnel.dns_domains.clone());
        cosmic::iced::Task::perform(
            async move {
                tokio::task::spawn_blocking(move || match action {
                    settings::TunnelAction::Install => crate::tunnel::install::install(&conf, &servers, &domains),
                    settings::TunnelAction::Uninstall => crate::tunnel::install::uninstall(),
                    settings::TunnelAction::Connect => crate::tunnel::control::connect(),
                    settings::TunnelAction::Disconnect => crate::tunnel::control::disconnect(),
                })
                .await
                .map_err(|e| format!("tunnel task failed: {e}"))
                .and_then(|r| r.map_err(|e| format!("{e:#}")))
            },
            move |result| cosmic::Action::App(Msg::Settings(settings::Msg::TunnelDone(action, result))),
        )
    }

    /// The XDG file-chooser portal, filtered to `*.conf`. A cancelled or
    /// unavailable dialog answers `None` and changes nothing.
    fn browse_tunnel_conf(&self) -> Task<cosmic::Action<Msg>> {
        use cosmic::dialog::file_chooser::{self, FileFilter};
        cosmic::iced::Task::perform(
            async {
                let dialog = file_chooser::open::Dialog::new()
                    .title("WireGuard configuration")
                    .filter(FileFilter::new("WireGuard config").glob("*.conf"));
                match dialog.open_file().await {
                    // `FileResponse::url()` panics when nothing was
                    // selected; the URI list behind it is empty instead.
                    Ok(response) => response.0.uris().first().and_then(|url| url.to_file_path().ok()),
                    // Cancelling is an answer, not a fault: only a portal
                    // that failed is worth a line in the log.
                    Err(file_chooser::Error::Cancelled) => None,
                    Err(why) => {
                        tracing::warn!("file chooser: {why}");
                        None
                    }
                }
            },
            |path| cosmic::Action::App(Msg::Settings(settings::Msg::TunnelConfChosen(path))),
        )
    }

    fn settings_note(&mut self, note: String) {
        if let Some(state) = self.settings.as_mut() {
            state.note = Some(note);
        }
    }

    fn settings_note_clear(&mut self) {
        if let Some(state) = self.settings.as_mut() {
            state.note = None;
        }
    }

    /// Write `config.ron` (atomically, through `model::write_atomic`) —
    /// unless the file on disk does not parse, in which case it is left
    /// exactly as the user wrote it (spec §9/§10).
    fn save_config(&mut self) {
        // The cached answer is up to a watcher debounce (200 ms) old, and
        // this is the last moment before the file is overwritten: ask the
        // file itself, and keep what it says on the window.
        let error = Config::try_load().err();
        let broken = error.is_some();
        if let Some(state) = self.settings.as_mut() {
            state.config_error = error;
        }
        if broken {
            return;
        }
        match self.config.save() {
            Ok(()) => self.last_config_write = Some((Instant::now(), self.config.clone())),
            Err(e) => {
                tracing::warn!("cannot save config: {e:#}");
                self.settings_note(format!("cannot save config.ron: {e:#}"));
            }
        }
    }
}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = AppFlags;
    type Message = Msg;
    const APP_ID: &'static str = "io.github.yutani";

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn init(core: cosmic::app::Core, flags: AppFlags) -> (Self, Task<cosmic::Action<Msg>>) {
        let (layout, layout_poisoned) = match Layout::try_load() {
            Ok(Some(layout)) => (layout, false),
            Ok(None) => (Layout::default(), false),
            Err(msg) => {
                tracing::warn!("{msg}; starting with an empty layout and refusing to save until it's fixed");
                (Layout::default(), true)
            }
        };
        let app = App {
            core,
            config: flags.0,
            conn: None,
            cmd: None,
            clients: HashMap::new(),
            outputs: Vec::new(),
            layout,
            layout_poisoned,
            layout_poison_warned: false,
            drag: None,
            hidden: false,
            last_eve_focus: None,
            tunnel_in_flight: None,
            settings: None,
            keepalive: None,
            last_config_write: None,
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Msg) -> Task<cosmic::Action<Msg>> {
        match message {
            Msg::Wayland(WaylandEvent::Output(event, output)) => {
                // The first output lets deferred surfaces be created; a
                // removal does not itself recreate anything (the compositor
                // sends Layer(Done, ..) for each surface on that output, and
                // that handler recreates them via output_for); a size change
                // moves the dock layout. `reconcile_surfaces` covers all.
                self.on_output(event, output);
                // A scale change is picked up at the next size send.
                let keepalive = self.ensure_keepalive();
                Task::batch([keepalive, self.reconcile_surfaces()])
            }
            Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, _, id)) => {
                // The compositor closed this surface (its output went away).
                if self.keepalive == Some(id) {
                    // Only on output loss; a new one goes up on whatever is
                    // left (or with the next output), still ahead of any
                    // thumbnail that is recreated for it.
                    self.keepalive = None;
                    return self.ensure_keepalive();
                }
                if let Some(handle) = self.client_for_surface(id) {
                    tracing::info!("layer surface closed by compositor; recreating");
                    self.forget_surface(&handle);
                    return self.reconcile_surfaces();
                }
                Task::none()
            }
            Msg::Wayland(WaylandEvent::Layer(..)) => Task::none(),
            Msg::Wayland(_) => Task::none(),
            Msg::Backend(event) => self.on_backend(event),
            Msg::Pointer(id, event) => self.on_pointer(id, event),
            Msg::ConfigChanged(config) => {
                // The watcher re-reading what we just wrote: a live-only
                // change made since (a slider still being dragged) is not
                // in the file and must not be reverted by it.
                let echo = self
                    .last_config_write
                    .as_ref()
                    .is_some_and(|(at, written)| at.elapsed() < OWN_WRITE_ECHO && *written == config);
                let task = if echo { Task::none() } else { self.apply_config(config) };
                // A hand edit may have fixed or broken the file; the text
                // fields deliberately keep whatever is being typed.
                if let Some(state) = self.settings.as_mut() {
                    state.refresh();
                }
                task
            }
            Msg::ConfigBroken(error) => {
                tracing::warn!("{error}; keeping the current config");
                if let Some(state) = self.settings.as_mut() {
                    state.config_error = Some(error);
                }
                Task::none()
            }
            // The X11/XWayland route only (see `subscription`); the drop
            // the settings window itself receives comes through
            // `settings::Msg::FilesDropped`. Both end in the same handler.
            Msg::FileDropped(id, paths) => {
                if self.settings.as_ref().map(|s| s.window) == Some(id) {
                    self.on_files_dropped(&paths);
                }
                Task::none()
            }
            Msg::Settings(msg) => self.on_settings(msg),
            Msg::Ipc(ev) => {
                let (reply, task) = self.handle_request(&ev.request, &ev.reply);
                if let Reply::Now(result) = reply {
                    ev.reply.respond(response_of(result));
                }
                task
            }
            Msg::IpcReplyLater(reply, result) => {
                reply.respond(response_of(result));
                Task::none()
            }
            // A failed stop is logged, not fatal: the user asked to quit,
            // and refusing to would leave them with a daemon they cannot
            // close. `yutani tunnel disconnect` still works afterwards.
            Msg::FocusGraceOver => self.reconcile_surfaces(),
            Msg::QuitAfterTunnel(result) => {
                if let Err(err) = result {
                    tracing::warn!("stopping the tunnel before quitting failed: {err}");
                } else {
                    tracing::info!("tunnel stopped; quitting");
                }
                ipc::remove_socket();
                cosmic::iced::exit()
            }
            Msg::Adopt(ev) => {
                match ev.result {
                    Ok(()) => tracing::info!(pid = ev.pid, name = %ev.name, "adopted into yutani-eve.slice"),
                    Err(e) => tracing::warn!(pid = ev.pid, name = %ev.name, "adoption failed: {e}"),
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let events = iced::event::listen_with(|event, _status, id| match event {
            iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(
                event @ (WaylandEvent::Output(..) | WaylandEvent::Layer(..)),
            )) => Some(Msg::Wayland(event)),
            iced::Event::Mouse(m) => Some(Msg::Pointer(id, m)),
            // winit emits this on X11, macOS and Windows only — never on
            // Wayland, where a drop reaches the client through the data
            // device and so through the settings window's own drag-and-drop
            // destination widget. Kept for XWayland/X11 sessions.
            iced::Event::Window(iced::window::Event::FileDropped(paths)) => Some(Msg::FileDropped(id, paths)),
            // Every other event (RequestResize, Frame, keyboard, …) must not become
            // a message: update → redraw → same event again is a hot loop.
            _ => None,
        });
        let mut subs = vec![
            events,
            config_watch::subscription().map(|loaded| match loaded {
                Ok(config) => Msg::ConfigChanged(config),
                Err(error) => Msg::ConfigBroken(error),
            }),
            ipc::subscription().map(Msg::Ipc),
        ];
        if let Some(conn) = self.conn.clone() {
            subs.push(
                backend::subscription(conn, self.config.app_ids.clone(), self.config.fps)
                    .map(Msg::Backend),
            );
        }
        if self.config.tunnel.auto_adopt {
            subs.push(adopt::subscription(self.config.tunnel.adopt_processes.clone()).map(Msg::Adopt));
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Msg> {
        unreachable!("no main window")
    }

    /// `run_single_instance` activates the already-running instance and lets
    /// this dbus-activation request arrive here instead of spawning a second
    /// process. In practice this never fires: the second-instance case is
    /// already handled before `run_single_instance` runs, by the socket
    /// check in `main.rs` (`cli::is_running`). This activation is a no-op.
    fn dbus_activation(&mut self, _msg: cosmic::dbus_activation::Message) -> Task<cosmic::Action<Msg>> {
        tracing::info!("activation request received (another yutani instance was launched)");
        Task::none()
    }

    /// Layer-surface configures arrive here. A drag starts by enlarging the
    /// surface to the output; only once that is confirmed are pointer
    /// coordinates absolute and motion may be applied.
    fn on_window_resize(&mut self, id: SurfaceId, width: f32, height: f32) {
        let Some(drag) = self.drag.as_mut().filter(|d| d.surface == id) else { return };
        if drag.phase != pointer::Phase::Arming || drag.pinned {
            return;
        }
        let Some(client) = self.clients.values().find(|c| c.surface == Some(id)) else { return };
        // Any configure wider than every possible thumbnail size (unzoomed or zoomed)
        // must be the full-output canvas. Ignore resizes to zoomed thumbnail size.
        let unzoomed = thumbnail::zoomed_size(&self.config, client.image.as_ref(), false).0 as f32;
        let zoomed = thumbnail::zoomed_size(&self.config, client.image.as_ref(), true).0 as f32;
        let largest_thumb = unzoomed.max(zoomed);
        tracing::debug!(?id, width, height, unzoomed, zoomed, largest_thumb, "drag: resize while arming");
        if width > largest_thumb + 1.0 {
            pointer::on_armed(drag, (width as i32, height as i32));
            tracing::debug!(?id, canvas = ?drag.canvas, "drag: armed");
        } else {
            tracing::debug!(width, largest_thumb, "drag: resize while arming (thumbnail-sized ack, ignoring)");
        }
    }

    /// Fires when a window is actually gone (libcosmic maps
    /// `window::Event::Closed` here). `exit_on_close(false)` means closing
    /// the settings window never exits the daemon.
    fn on_close_requested(&self, id: SurfaceId) -> Option<Msg> {
        self.settings.as_ref().filter(|s| s.window == id).map(|_| Msg::Settings(settings::Msg::Closed))
    }

    fn view_window(&self, id: SurfaceId) -> Element<'_, Msg> {
        if let Some(state) = self.settings.as_ref().filter(|s| s.window == id) {
            let focused = self.core.focused_window() == Some(id);
            return settings::view(state, &self.config, focused, !self.clients.is_empty());
        }
        let Some((_, client)) = self.clients.iter().find(|(_, c)| c.surface == Some(id)) else {
            return widget::text("").into();
        };
        if self.in_canvas(id) {
            // Full-output canvas: draw the thumbnail at its position inside it.
            let (w, h) = thumbnail::size(&self.config, client.image.as_ref());
            let (x, y) = client.position;
            widget::container(
                widget::container(thumbnail::view(client, &self.config))
                    .width(Length::Fixed(w as f32))
                    .height(Length::Fixed(h as f32)),
            )
            // top, right, bottom, left
            .padding([y as f32, 0.0, 0.0, x as f32])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            thumbnail::view(client, &self.config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::cctk::wayland_client::protocol::wl_registry::WlRegistry;
    use cosmic::cctk::wayland_client::protocol::wl_surface::WlSurface;
    use cosmic::cctk::wayland_client::{EventQueue, QueueHandle, backend::Backend, delegate_noop};
    use std::os::unix::net::UnixStream;

    use crate::model::config::Visibility;

    /// A Wayland connection with nobody on the other end: enough to mint
    /// proxies (outputs, toplevel handles) for an `App` without a
    /// compositor. Requests go into the socket's buffer and nothing ever
    /// answers them, which is fine — the tests only look at the app's
    /// state, never at what the (absent) compositor would do.
    struct Fake {
        qh: QueueHandle<Nop>,
        registry: WlRegistry,
        _queue: EventQueue<Nop>,
        _peer: UnixStream,
    }

    struct Nop;
    delegate_noop!(Nop: ignore WlRegistry);
    delegate_noop!(Nop: ignore WlOutput);
    delegate_noop!(Nop: ignore Handle);
    delegate_noop!(Nop: ignore WlSurface);

    impl Fake {
        fn new() -> Fake {
            let (ours, peer) = UnixStream::pair().unwrap();
            let conn = Connection::from_backend(Backend::connect(ours).unwrap());
            let queue = conn.new_event_queue::<Nop>();
            let qh = queue.handle();
            let registry = conn.display().get_registry(&qh, ());
            Fake { qh, registry, _queue: queue, _peer: peer }
        }

        fn output(&self) -> WlOutput {
            self.registry.bind::<WlOutput, _, _>(1, 4, &self.qh, ())
        }

        /// A toplevel handle. The compositor would normally create these;
        /// binding one as a global is nonsense on the wire but yields a
        /// perfectly good proxy to key `clients` by.
        fn handle(&self) -> Handle {
            self.registry.bind::<Handle, _, _>(1, 1, &self.qh, ())
        }

        /// A `wl_surface` proxy, for the layer events that carry one.
        fn wl_surface(&self) -> WlSurface {
            self.registry.bind::<WlSurface, _, _>(1, 1, &self.qh, ())
        }
    }

    fn app(config: Config) -> App {
        App {
            core: cosmic::app::Core::default(),
            config,
            conn: None,
            cmd: None,
            clients: HashMap::new(),
            outputs: Vec::new(),
            layout: Layout::default(),
            layout_poisoned: false,
            layout_poison_warned: false,
            drag: None,
            hidden: false,
            last_eve_focus: None,
            settings: None,
            tunnel_in_flight: None,
            keepalive: None,
            last_config_write: None,
        }
    }

    /// Announce an output to the app the way libcosmic does, and hand back
    /// its handle so clients can be placed on it. sctk's `OutputInfo` is
    /// `#[non_exhaustive]`, so the event carries no info and the record
    /// `on_output` would have built from it is registered by hand — then
    /// the same reconcile the event handler runs.
    fn add_output(app: &mut App, fake: &Fake, name: &str) -> WlOutput {
        let output = fake.output();
        app.outputs.push(Output { handle: output.clone(), name: name.to_string(), logical_size: (2560, 1440), scale: 1 });
        let _ = app.update(Msg::Wayland(WaylandEvent::Output(OutputEvent::Created(None), output.clone())));
        output
    }

    fn info(activated: bool, outputs: Vec<WlOutput>) -> ClientInfo {
        ClientInfo { login: Login::LoggingIn, activated, minimized: false, outputs }
    }

    fn surface_of(app: &App, h: &Handle) -> Option<SurfaceId> {
        app.clients[h].surface
    }

    /// [I2] `status` is what the applet polls every 5 s (1 s with its popup
    /// open), and the tunnel half of it can spawn `systemctl is-failed`
    /// (installed unit, link down — the ordinary state). That must never
    /// run on the thread that drives every thumbnail frame, so the request
    /// is answered later, from a task, and nothing reaches the client from
    /// this call.
    #[test]
    fn status_is_answered_off_the_update_thread() {
        let mut app = app(Config::default());
        let (reply, mut rx) = ipc::Responder::detached();
        let (how, _task) = app.handle_request(&crate::ipc::Request::Status, &reply);
        assert!(matches!(how, Reply::Later), "answered on the update thread");
        assert!(rx.try_recv().is_err(), "the reply must come from the task, not from this call");
    }

    /// [I6] The first layer surface we create is the permanent keepalive,
    /// never a thumbnail: libcosmic binds its clipboard to that first
    /// surface and reconnects (a new worker thread, display connection and
    /// output bindings, dropping the old ones) whenever it is destroyed.
    /// So it exists before any client can have a surface, no client
    /// lookup ever resolves to it, hiding a thumbnail leaves it alone, and
    /// a `Done` from the compositor (its output went away) puts a new one
    /// up without touching any client.
    #[test]
    fn the_first_surface_is_the_permanent_keepalive_and_never_a_thumbnail() {
        let fake = Fake::new();
        let mut app = app(Config { visibility: Visibility::Always, ..Config::default() });
        assert_eq!(app.keepalive, None, "nothing before the first output");
        add_output(&mut app, &fake, "DP-1");
        let keepalive = app.keepalive.expect("created with the first output");
        assert!(app.clients.is_empty());

        let a = fake.handle();
        let _ = app.on_backend(Event::ClientAdded(a.clone(), info(true, Vec::new())));
        let thumb = surface_of(&app, &a).expect("shown");
        assert!(keepalive < thumb, "the keepalive was minted first");
        assert_eq!(app.client_for_surface(keepalive), None, "no client lookup resolves to it");

        let _ = app.set_hidden(true);
        assert_eq!(surface_of(&app, &a), None);
        assert_eq!(app.keepalive, Some(keepalive), "hiding every thumbnail leaves it alone");

        let _ = app.set_hidden(false);
        let shown = surface_of(&app, &a).expect("shown again");
        let _ = app.update(Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, fake.wl_surface(), keepalive)));
        assert!(app.keepalive.is_some_and(|k| k != keepalive), "replaced after the compositor closed it");
        assert_eq!(surface_of(&app, &a), Some(shown), "and no client was touched");
    }

    /// [I4] A character logs in on DP-1 (its thumbnail is created there,
    /// on the client's own output) with a saved position on DP-2. When the
    /// name resolves, the saved x/y must not be applied with a margin on
    /// DP-1 — a layer surface is bound to one output for life — but the
    /// surface destroyed and recreated on DP-2, as applying a layout does.
    #[test]
    fn a_saved_position_on_another_output_recreates_the_surface_there() {
        let fake = Fake::new();
        let mut app = app(Config { mode: Mode::Floating, visibility: Visibility::Always, ..Config::default() });
        let dp1 = add_output(&mut app, &fake, "DP-1");
        let _dp2 = add_output(&mut app, &fake, "DP-2");
        app.layout.thumbs.insert("Aria".into(), ThumbPos { output: "DP-2".into(), x: 100, y: 100, pinned: false });
        let a = fake.handle();
        let _ = app.on_backend(Event::ClientAdded(a.clone(), info(true, vec![dp1])));
        let before = surface_of(&app, &a).expect("shown");
        assert_eq!(app.clients[&a].output, "DP-1", "created on the client's own output while unnamed");

        app.clients.get_mut(&a).unwrap().info.login = Login::LoggedIn("Aria".into());
        let _ = app.apply_saved_position(&a);

        let client = &app.clients[&a];
        assert_eq!(client.output, "DP-2", "the thumbnail belongs on the saved output");
        assert_eq!(client.position, (100, 100));
        assert!(client.surface.is_some_and(|id| id != before), "recreated, not margin-moved");
    }

    /// [M2] The watcher re-reads the file we just wrote (200 ms debounce).
    /// A live-only slider change made in that window is not in the file,
    /// so applying the re-read reverted it and the slider snapped back.
    /// The echo of our own write is ignored; a real edit — a different
    /// config, or one arriving long after our write — still applies.
    #[test]
    fn the_watchers_echo_of_our_own_write_does_not_revert_a_live_change() {
        let written = Config { thumb_width: 300, ..Config::default() };
        let mut app = app(written.clone());
        app.last_config_write = Some((Instant::now(), written.clone()));
        let live = Config { thumb_width: 400, ..written.clone() };
        let _ = app.apply_config(live.clone());
        let _ = app.update(Msg::ConfigChanged(written.clone()));
        assert_eq!(app.config.thumb_width, 400, "our own write, echoed back, must not win");

        // The same content long after our write is the user's edit.
        app.last_config_write = Some((Instant::now() - Duration::from_secs(5), written.clone()));
        let _ = app.update(Msg::ConfigChanged(written.clone()));
        assert_eq!(app.config.thumb_width, 300);

        // Different content inside the window is the user's edit too.
        app.last_config_write = Some((Instant::now(), written.clone()));
        let _ = app.update(Msg::ConfigChanged(Config { thumb_width: 500, ..written }));
        assert_eq!(app.config.thumb_width, 500);
    }

    /// [M3] Before the backend has handed over its command channel, a
    /// `focus`/`next`/`prev` cannot be carried out; answering `ok` then
    /// told the hotkey user nothing happened for no reason.
    #[test]
    fn focus_requests_fail_honestly_while_the_backend_is_not_ready() {
        let fake = Fake::new();
        let mut app = app(Config::default());
        let a = fake.handle();
        let _ = app.on_backend(Event::ClientAdded(a.clone(), info(false, Vec::new())));
        assert!(app.cmd.is_none());
        for request in [crate::ipc::Request::Focus(1), crate::ipc::Request::Next, crate::ipc::Request::Prev] {
            let (reply, _) = ipc::Responder::detached();
            let (how, _task) = app.handle_request(&request, &reply);
            assert!(matches!(how, Reply::Now(Err(ref m)) if m.contains("not ready")), "{request:?}: {}", match how {
                Reply::Now(r) => format!("{r:?}"),
                Reply::Later => "later".into(),
            });
        }
    }

    /// [P1] `relayout_dock` ran on every frame in dock mode (fps × clients
    /// a second), though the size — the only thing a frame can change —
    /// changes on the first frame or a new aspect only. `resize_if_needed`
    /// says whether it sent a size, and the frame handler relays out only
    /// then: unchanged is `None`, a zoom (hover) is `Some`.
    #[test]
    fn an_unchanged_size_sends_nothing_so_the_frame_path_skips_the_relayout() {
        let fake = Fake::new();
        let mut app = app(Config { visibility: Visibility::Always, zoom_factor: 1.5, ..Config::default() });
        add_output(&mut app, &fake, "DP-1");
        let a = fake.handle();
        let _ = app.on_backend(Event::ClientAdded(a.clone(), info(true, Vec::new())));
        assert!(surface_of(&app, &a).is_some());
        assert!(app.resize_if_needed(&a).is_none(), "same size as at creation");
        app.clients.get_mut(&a).unwrap().hovered = true;
        assert!(app.resize_if_needed(&a).is_some(), "zoomed: a new size");
        assert!(app.resize_if_needed(&a).is_none(), "and sent once");
    }

    /// [I3] The watcher's answer to a `config.ron` that does not parse:
    /// nothing changes live, and an open settings window shows why (the
    /// same field `save_config` uses, so the Display/Behavior pages give
    /// way to the reason and nothing is written over the file).
    #[test]
    fn a_broken_config_on_disk_leaves_the_live_config_alone_and_says_why() {
        let custom = Config { thumb_width: 400, ..Config::default() };
        let mut app = app(custom.clone());
        app.settings = Some(settings::State::new(SurfaceId::unique(), &custom));
        let _ = app.update(Msg::ConfigBroken("cannot parse config.ron: 3:1".into()));
        assert_eq!(app.config, custom);
        assert_eq!(app.settings.as_ref().unwrap().config_error.as_deref(), Some("cannot parse config.ron: 3:1"));
    }

    /// [M5] `quit` with nothing to wind down used to answer `ok` and return
    /// `exit()` from the same update: the connection task's write of that
    /// reply raced process teardown, and `yutani quit` could see EOF ("no
    /// reply") and exit 1 after a successful quit. The answer now comes
    /// from the task that exits, which writes it first.
    #[test]
    fn quit_answers_from_the_task_that_exits_so_the_reply_is_written_first() {
        let app = app(Config::default());
        let (reply, mut rx) = ipc::Responder::detached();
        let (how, _task) = app.quit(crate::tunnel::control::QuitPlan::ExitNow, &reply);
        assert!(matches!(how, Reply::Later), "answered in the same update as the exit");
        assert!(rx.try_recv().is_err(), "the reply must come from the task, not from this call");
    }

    /// [I1] Play in client A for a minute (no toplevel events), then click
    /// client B. cosmic-comp refreshes toplevel state in list order, so A's
    /// deactivation lands first and B's activation a few milliseconds
    /// later. The grace has to start the moment focus *leaves* — not date
    /// from the last event that happened to arrive while EVE was focused —
    /// or every surface is destroyed and recreated across that gap.
    #[test]
    fn a_click_from_one_eve_window_to_another_keeps_every_surface() {
        let fake = Fake::new();
        let mut app = app(Config { visibility: Visibility::EveFocusedOnly, ..Config::default() });
        add_output(&mut app, &fake, "DP-1");
        let (a, b) = (fake.handle(), fake.handle());
        let _ = app.on_backend(Event::ClientAdded(a.clone(), info(true, Vec::new())));
        let _ = app.on_backend(Event::ClientAdded(b.clone(), info(false, Vec::new())));
        let before = (surface_of(&app, &a), surface_of(&app, &b));
        assert!(before.0.is_some() && before.1.is_some(), "shown while EVE is focused");
        // A quiet minute in A: nothing stamped `last_eve_focus` since.
        app.last_eve_focus = Some(Instant::now().checked_sub(Duration::from_secs(60)).unwrap());

        let _ = app.on_backend(Event::ClientUpdated(a.clone(), info(false, Vec::new())));
        std::thread::sleep(Duration::from_millis(5));
        let _ = app.on_backend(Event::ClientUpdated(b.clone(), info(true, Vec::new())));

        let after = (surface_of(&app, &a), surface_of(&app, &b));
        assert_eq!(after, before, "no surface was destroyed and recreated across the click");
    }

    /// The other half of the grace: once it has passed with nobody
    /// activated, `FocusGraceOver` does take the surfaces down.
    #[test]
    fn the_surfaces_go_once_the_grace_passes_with_eve_unfocused() {
        let fake = Fake::new();
        let mut app = app(Config { visibility: Visibility::EveFocusedOnly, ..Config::default() });
        add_output(&mut app, &fake, "DP-1");
        let a = fake.handle();
        let _ = app.on_backend(Event::ClientAdded(a.clone(), info(true, Vec::new())));
        let _ = app.on_backend(Event::ClientUpdated(a.clone(), info(false, Vec::new())));
        assert!(surface_of(&app, &a).is_some(), "kept for the grace");
        app.last_eve_focus = Some(Instant::now().checked_sub(rules::FOCUS_GRACE + Duration::from_millis(100)).unwrap());
        let _ = app.update(Msg::FocusGraceOver);
        assert_eq!(surface_of(&app, &a), None);
    }
}
