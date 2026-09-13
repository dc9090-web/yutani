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
    destroy_layer_surface, get_layer_surface, set_anchor, set_margin, set_size,
};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{self, Length, Point, Subscription};
use cosmic::{Application, Element, Task, widget};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// The settings window while it is open (spec §6).
    pub settings: Option<settings::State>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Wayland(WaylandEvent),
    Backend(Event),
    Pointer(SurfaceId, mouse::Event),
    ConfigChanged(Config),
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
    Adopt(adopt::AdoptEvent),
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

impl App {
    fn send(&self, cmd: Cmd) {
        match &self.cmd {
            Some(sender) => {
                if let Err(err) = sender.send(cmd) {
                    tracing::error!("backend command channel closed: {err}");
                }
            }
            None => tracing::warn!("backend not ready; dropping command"),
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

    fn should_show(&self, client: &Client) -> bool {
        rules::should_show(
            self.config.visibility,
            self.config.hide_active,
            self.hidden,
            self.any_client_activated(),
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
                    Some(h) => {
                        self.send(Cmd::Activate(h.clone()));
                        (Reply::Now(Ok(None)), Task::none())
                    }
                    None => (Reply::Now(Err(format!("no client {n} ({} known)", order.len()))), Task::none()),
                }
            }
            Request::Next | Request::Prev => {
                let order = self.focus_order();
                let active = self.active_client();
                match rules::step(&order, active.as_ref(), matches!(request, Request::Next)) {
                    Some(h) => {
                        self.send(Cmd::Activate(h));
                        (Reply::Now(Ok(None)), Task::none())
                    }
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
            // client is answered `ok` straight away either way: a
            // `systemctl stop` can take the unit's whole TimeoutStopSec
            // (10 s), and neither the caller nor the thumbnails may hang on
            // it, so the stop runs on the blocking pool and the exit itself
            // waits for `Msg::QuitAfterTunnel`.
            Request::Quit => {
                let tunnel = crate::tunnel::control::current_tunnel_status(&self.config.tunnel.location);
                match crate::tunnel::control::quit_plan(tunnel.installed, tunnel.connected) {
                    crate::tunnel::control::QuitPlan::ExitNow => {
                        ipc::remove_socket();
                        (Reply::Now(Ok(None)), cosmic::iced::exit())
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
            Request::Status => {
                let order = self.focus_order();
                let clients = order
                    .iter()
                    .filter_map(|h| self.clients.get(h))
                    .map(|c| crate::tunnel::status::ClientStatus { name: c.info.login.label().to_string(), active: c.info.activated })
                    .collect();
                let status = crate::tunnel::status::Status {
                    clients,
                    hidden: self.hidden,
                    tunnel: crate::tunnel::control::current_tunnel_status(&self.config.tunnel.location),
                };
                match serde_json::to_string(&status) {
                    Ok(json) => (Reply::Now(Ok(Some(json))), Task::none()),
                    Err(e) => (Reply::Now(Err(format!("status: {e}"))), Task::none()),
                }
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
            exclusive_zone: 0,
            // This pinned iced ignores size_limits for layer surfaces; NONE is harmless.
            size_limits: Limits::NONE,
            ..Default::default()
        });
        create
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
        let Some(output) = self.output_name_of(handle) else { return };
        self.layout
            .thumbs
            .insert(name, ThumbPos { output, x: position.0, y: position.1, pinned });
        self.save_current_layout();
    }

    /// If we have a saved position for this character, move there. Floating
    /// only: in dock mode the saved spot applies when (if) the mode changes.
    fn apply_saved_position(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        if self.config.mode == Mode::Dock {
            return Task::none();
        }
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Login::LoggedIn(name) = &client.info.login else { return Task::none() };
        let Some(saved) = self.layout.thumbs.get(name).cloned() else { return Task::none() };
        let client = self.clients.get_mut(handle).unwrap();
        client.position = (saved.x, saved.y);
        client.pinned = saved.pinned;
        let surface = client.surface;
        match surface {
            // A drag canvas surface must keep its enlarged size until the
            // drag ends; a margin here would shrink it out from under the drag.
            Some(id) if !self.in_canvas(id) => set_margin(id, saved.y, 0, 0, saved.x),
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
    fn resize_if_needed(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Some(id) = client.surface else { return Task::none() };
        if self.in_canvas(id) {
            return Task::none();
        }
        let size = self.surface_size(client);
        if client.last_size == Some(size) {
            return Task::none();
        }
        self.clients.get_mut(handle).unwrap().last_size = Some(size);
        self.send_thumb_size(handle, size);
        set_size(id, Some(size.0), Some(size.1))
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
                            let grid = self.config.snap_grid.then_some(32);
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
                Task::batch([self.resize_if_needed(&handle), self.relayout_dock()])
            }
            mouse::Event::CursorLeft => {
                // Releasing outside is delivered to us anyway (implicit grab); nothing
                // to do while dragging — canvas mode owns the size until release.
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    return Task::none();
                }
                self.clients.get_mut(&handle).unwrap().hovered = false;
                Task::batch([self.resize_if_needed(&handle), self.relayout_dock()])
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
                // An activation change on one client can hide/show others, so
                // reconcile every client's surface, not just this one's.
                let reconciled = self.reconcile_surfaces();
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
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                // The layout order changed; the saved position stays, so the
                // character comes back to the same spot next launch.
                self.save_current_layout();
                // Dock mode: its neighbours close the gap.
                Task::batch([task, self.relayout_dock()])
            }
            Event::Frame(handle, image) => {
                let Some(client) = self.clients.get_mut(&handle) else { return Task::none() };
                client.image = Some(image);
                client.unavailable = false;
                let task = if client.surface.is_some() {
                    // `resize_if_needed` skips a drag canvas and dedupes
                    // against the last size actually sent.
                    self.resize_if_needed(&handle)
                } else {
                    self.create_surface(&handle)
                };
                // Dock mode: a first frame (or a new aspect) changes this
                // thumbnail's size, which moves its neighbours.
                Task::batch([task, self.relayout_dock()])
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
            tasks.push(self.resize_if_needed(&h));
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
        self.settings = Some(settings::State::new(id, &self.config));
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
                // Un-minimize first, then activate: a window the compositor
                // minimised stays minimised if it is only activated. The
                // same chain libcosmic's own `Action::Activate` does.
                return Task::batch([
                    characters,
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
                    // ours, and the names lookup is a task).
                    return self.refresh_characters();
                }
                S::ActiveBorder(text) => state.active_border_field = text.clone(),
                S::InactiveBorder(text) => state.inactive_border_field = text.clone(),
                S::NextKey(text) => state.next_field = text.clone(),
                S::PrevKey(text) => state.prev_field = text.clone(),
                S::Name(text) => {
                    state.name_field = text.clone();
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
            S::RefreshCharacters => return self.refresh_characters(),
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
        if let Err(e) = self.config.save() {
            tracing::warn!("cannot save config: {e:#}");
            self.settings_note(format!("cannot save config.ron: {e:#}"));
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
            settings: None,
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
                self.reconcile_surfaces()
            }
            Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, _, id)) => {
                // The compositor closed this surface (its output went away).
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
                let task = self.apply_config(config);
                // A hand edit may have fixed or broken the file; the text
                // fields deliberately keep whatever is being typed.
                if let Some(state) = self.settings.as_mut() {
                    state.refresh();
                }
                task
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
            // Every other event (RequestResize, Frame, keyboard, …) must not become
            // a message: update → redraw → same event again is a hot loop.
            _ => None,
        });
        let mut subs = vec![
            events,
            config_watch::subscription().map(Msg::ConfigChanged),
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
