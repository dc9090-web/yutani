//! libcosmic application: no main window; one overlay layer surface per
//! EVE client, each showing that client's live captured frame.

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::{Connection, Proxy, protocol::wl_output::WlOutput};
use cosmic::iced::event::wayland::{Event as WaylandEvent, LayerEvent, OutputEvent};
use cosmic::iced::mouse;
use cosmic::iced::core::layout::Limits;
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

use crate::backend::{self, CaptureImage, ClientInfo, Cmd, Event, Handle};
use crate::model::client::Login;
use crate::model::config::Config;
use crate::model::layout::{self, Layout, Rect, ThumbPos};

pub mod config_watch;
pub mod pointer;
pub mod thumbnail;

/// `Config` carries no CLI-parsed subcommand/args; only its file contents
/// matter, so this satisfies `run_single_instance`'s bound trivially.
impl cosmic::app::CosmicFlags for Config {
    type SubCommand = String;
    type Args = Vec<String>;
}

pub fn run(config: Config) -> iced::Result {
    cosmic::app::run_single_instance::<App>(
        cosmic::app::Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        config,
    )
}

#[derive(Clone, Debug)]
pub struct Output {
    pub handle: WlOutput,
    pub name: String,
    pub logical_size: (i32, i32),
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
}

pub struct App {
    core: cosmic::app::Core,
    pub config: Config,
    pub conn: Option<Connection>,
    pub cmd: Option<calloop::channel::Sender<Cmd>>,
    pub clients: HashMap<Handle, Client>,
    pub outputs: Vec<Output>,
    pub layout: Layout,
    pub drag: Option<pointer::DragState>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Wayland(WaylandEvent),
    Backend(Event),
    Pointer(SurfaceId, mouse::Event),
    ConfigChanged(Config),
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
                if let Some(existing) = self.outputs.iter_mut().find(|o| o.handle == output) {
                    existing.name = name;
                    existing.logical_size = logical_size;
                } else {
                    tracing::info!(%name, ?logical_size, "output");
                    self.outputs.push(Output { handle: output, name, logical_size });
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
        use crate::model::config::Visibility;
        let visible = match self.config.visibility {
            Visibility::Always => true,
            Visibility::EveFocusedOnly => self.any_client_activated(),
        };
        visible && !(self.config.hide_active && client.info.activated)
    }

    /// Make every client's surface existence match `should_show`.
    fn reconcile_surfaces(&mut self) -> Task<cosmic::Action<Msg>> {
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        let mut tasks = Vec::new();
        for h in handles {
            let show = self.should_show(&self.clients[&h]);
            let has = self.clients[&h].surface.is_some();
            if show && !has {
                tasks.push(self.create_surface(&h));
            } else if !show && has {
                tasks.push(self.destroy_surface(&h));
            }
        }
        Task::batch(tasks)
    }

    /// Output to show a client on: the one it is on, else the first known.
    fn output_for(&self, info: &ClientInfo) -> Option<WlOutput> {
        info.outputs
            .iter()
            .find(|o| self.outputs.iter().any(|known| known.handle == **o))
            .cloned()
            .or_else(|| self.outputs.first().map(|o| o.handle.clone()))
    }

    /// Next free slot: a row along the top, left to right, filling any gap
    /// left by a removed client rather than always appending.
    fn next_position(&self) -> (i32, i32) {
        let (w, _) = thumbnail::size(&self.config, None);
        let taken: Vec<i32> = self
            .clients
            .values()
            .filter(|c| c.surface.is_some())
            .map(|c| c.position.0)
            .collect();
        (thumbnail::next_free_x(&taken, 40, w as i32 + 16), 40)
    }

    /// Layer-surface size for `client`, zoomed if hovered.
    fn surface_size(&self, client: &Client) -> (u32, u32) {
        thumbnail::zoomed_size(&self.config, client.image.as_ref(), client.hovered)
    }

    fn create_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        if client.surface.is_some() {
            return Task::none();
        }
        if !self.should_show(client) {
            return Task::none();
        }
        let Some(output) = self.output_for(&client.info) else {
            tracing::debug!("no outputs yet; deferring surface");
            return Task::none();
        };
        // Prefer this client's own previous placement (destroy+recreate cycles
        // like hide_active, EveFocusedOnly, or output loss should put the
        // thumbnail back where it was even if never persisted); otherwise a
        // saved position for this character; otherwise the next free slot.
        let (position, pinned) = if client.placed {
            (client.position, client.pinned)
        } else {
            let saved = match &client.info.login {
                Login::LoggedIn(name) => self.layout.thumbs.get(name).cloned(),
                Login::LoggingIn => None,
            };
            match saved {
                Some(s) => ((s.x, s.y), s.pinned),
                None => (self.next_position(), false),
            }
        };
        let (width, height) = self.surface_size(client);
        let id = SurfaceId::unique();
        let client = self.clients.get_mut(handle).unwrap();
        client.surface = Some(id);
        client.position = position;
        client.pinned = pinned;
        client.placed = true;
        tracing::info!(?id, x = position.0, y = position.1, width, height, "create_surface");
        get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: IcedOutput::Output(output),
            namespace: "yutani".into(),
            margin: IcedMargin { top: position.1, left: position.0, ..Default::default() },
            size: Some((Some(width), Some(height))),
            exclusive_zone: 0,
            // Default limits cap at 1920×1080, which would clip the full-output
            // drag canvas on larger outputs.
            size_limits: Limits::NONE,
            ..Default::default()
        })
    }

    fn destroy_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        match self.clients.get_mut(handle).and_then(|c| c.surface.take()) {
            Some(id) => {
                // A drag on a surface that goes away must not linger and block
                // future drags.
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    self.drag = None;
                }
                destroy_layer_surface(id)
            }
            None => Task::none(),
        }
    }

    fn client_for_surface(&self, id: SurfaceId) -> Option<Handle> {
        self.clients.iter().find(|(_, c)| c.surface == Some(id)).map(|(h, _)| h.clone())
    }

    fn rect_of(&self, client: &Client) -> Rect {
        let (w, h) = thumbnail::size(&self.config, client.image.as_ref());
        Rect { x: client.position.0, y: client.position.1, w: w as i32, h: h as i32 }
    }

    fn output_name_of(&self, client: &Client) -> Option<String> {
        self.output_for(&client.info)
            .and_then(|o| self.outputs.iter().find(|k| k.handle == o).map(|k| k.name.clone()))
    }

    /// Remember this client's position (and pin state) under its character name.
    fn persist_position(&mut self, handle: &Handle) {
        let Some(client) = self.clients.get(handle) else { return };
        let Login::LoggedIn(name) = &client.info.login else {
            tracing::debug!("position not saved: character name not resolved yet");
            return;
        };
        let Some(output) = self.output_name_of(client) else { return };
        self.layout.thumbs.insert(
            name.clone(),
            ThumbPos { output, x: client.position.0, y: client.position.1, pinned: client.pinned },
        );
        if let Err(e) = self.layout.save() {
            tracing::warn!("cannot save layout: {e}");
        }
    }

    /// If we have a saved position for this character, move there.
    fn apply_saved_position(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Login::LoggedIn(name) = &client.info.login else { return Task::none() };
        let Some(saved) = self.layout.thumbs.get(name).cloned() else { return Task::none() };
        let client = self.clients.get_mut(handle).unwrap();
        client.position = (saved.x, saved.y);
        client.pinned = saved.pinned;
        match client.surface {
            Some(id) => set_margin(id, saved.y, 0, 0, saved.x),
            None => Task::none(),
        }
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
    fn leave_canvas(&self, id: SurfaceId, client: &Client) -> Task<cosmic::Action<Msg>> {
        // `client.hovered` is set true before this is called (the cursor is
        // over the thumbnail at drag end), so this restores at zoomed size.
        let (w, h) = self.surface_size(client);
        let (x, y) = client.position;
        Task::batch([
            set_anchor(id, Anchor::TOP | Anchor::LEFT),
            set_size(id, Some(w), Some(h)),
            set_margin(id, y, 0, 0, x),
        ])
    }

    /// True while `id` is enlarged to a drag canvas (any phase).
    fn in_canvas(&self, id: SurfaceId) -> bool {
        self.drag.as_ref().is_some_and(|d| d.surface == id && !d.pinned)
    }

    fn on_pointer(&mut self, id: SurfaceId, event: mouse::Event) -> Task<cosmic::Action<Msg>> {
        let Some(handle) = self.client_for_surface(id) else { return Task::none() };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Middle) => {
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
                self.drag = pointer::on_press(id, button, cursor, c.position, c.pinned);
                match &self.drag {
                    // Pinned thumbnails only click; no need to enlarge.
                    Some(d) if !d.pinned => {
                        tracing::debug!(?id, press_abs = ?d.press_abs, "drag: arming canvas");
                        Self::enter_canvas(id)
                    }
                    _ => Task::none(),
                }
            }
            mouse::Event::CursorMoved { position } => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.last_cursor = position;
                }
                let Some(drag) = self.drag.as_mut().filter(|d| d.surface == id) else { return Task::none() };
                match pointer::on_move(drag, position) {
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
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if !self.drag.as_ref().is_some_and(|d| d.surface == id && d.button == button) {
                    return Task::none();
                }
                let drag = self.drag.take().unwrap();
                let was_canvas = !drag.pinned;
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
                if was_canvas {
                    // The cursor is over the thumbnail at drag end (no CursorEntered
                    // fires for a surface that was already under the pointer), so mark
                    // it hovered before computing the restore size — leave_canvas then
                    // restores at zoomed size instead of snapping small first.
                    self.clients.get_mut(&handle).unwrap().hovered = true;
                    let client = &self.clients[&handle];
                    tracing::debug!(?id, pos = ?client.position, "drag: leaving canvas");
                    self.leave_canvas(id, client)
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
                let c = self.clients.get_mut(&handle).unwrap();
                c.hovered = true;
                let (w, h) = self.surface_size(&self.clients[&handle]);
                set_size(id, Some(w), Some(h))
            }
            mouse::Event::CursorLeft => {
                // Releasing outside is delivered to us anyway (implicit grab); nothing
                // to do while dragging — canvas mode owns the size until release.
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    return Task::none();
                }
                let c = self.clients.get_mut(&handle).unwrap();
                c.hovered = false;
                let (w, h) = self.surface_size(&self.clients[&handle]);
                set_size(id, Some(w), Some(h))
            }
            _ => Task::none(),
        }
    }

    fn on_backend(&mut self, event: Event) -> Task<cosmic::Action<Msg>> {
        match event {
            Event::CmdSender(sender) => {
                self.cmd = Some(sender);
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
                });
                let was_named = matches!(entry.info.login, Login::LoggedIn(_));
                let had_surface = entry.surface.is_some();
                entry.info = info;
                let became_named = !was_named && matches!(entry.info.login, Login::LoggedIn(_));
                // An activation change on one client can hide/show others, so
                // reconcile every client's surface, not just this one's.
                let reconciled = self.reconcile_surfaces();
                if became_named && had_surface {
                    Task::batch([reconciled, self.apply_saved_position(&handle)])
                } else {
                    reconciled
                }
            }
            Event::ClientRemoved(handle) => {
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                task
            }
            Event::Frame(handle, image) => {
                let Some(client) = self.clients.get_mut(&handle) else { return Task::none() };
                let old = thumbnail::zoomed_size(&self.config, client.image.as_ref(), client.hovered);
                let new = thumbnail::zoomed_size(&self.config, Some(&image), client.hovered);
                client.image = Some(image);
                client.unavailable = false;
                let surface = client.surface;
                match surface {
                    // While enlarged to a drag canvas the surface must keep its
                    // size; `leave_canvas` applies the current size on release.
                    Some(id) if old != new && !self.in_canvas(id) => {
                        set_size(id, Some(new.0), Some(new.1))
                    }
                    Some(_) => Task::none(),
                    None => self.create_surface(&handle),
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

    /// Apply a freshly re-read (and validated) config. Border colours,
    /// opacity and names apply on the next redraw automatically because
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
        self.config = new;
        // Sizes and visibility may have changed.
        let mut tasks = vec![self.reconcile_surfaces()];
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        for h in handles {
            // A surface enlarged to a drag canvas must keep its size until the
            // drag ends (`leave_canvas` applies the current size then).
            if let Some(id) = self.clients[&h].surface.filter(|id| !self.in_canvas(*id)) {
                let (w, hgt) = self.surface_size(&self.clients[&h]);
                tasks.push(set_size(id, Some(w), Some(hgt)));
            }
        }
        Task::batch(tasks)
    }
}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = Config;
    type Message = Msg;
    const APP_ID: &'static str = "io.github.yutani";

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn init(core: cosmic::app::Core, config: Config) -> (Self, Task<cosmic::Action<Msg>>) {
        let app = App {
            core,
            config,
            conn: None,
            cmd: None,
            clients: HashMap::new(),
            outputs: Vec::new(),
            layout: Layout::load(),
            drag: None,
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Msg) -> Task<cosmic::Action<Msg>> {
        match message {
            Msg::Wayland(WaylandEvent::Output(event, output)) => {
                let had_outputs = !self.outputs.is_empty();
                let removed = matches!(event, OutputEvent::Removed);
                self.on_output(event, output);
                if !had_outputs && !self.outputs.is_empty() {
                    let pending: Vec<Handle> = self
                        .clients
                        .iter()
                        .filter(|(_, c)| c.surface.is_none())
                        .map(|(h, _)| h.clone())
                        .collect();
                    return Task::batch(pending.iter().map(|h| self.create_surface(h)));
                }
                if removed {
                    // This does not itself recreate anything: the compositor
                    // sends Layer(Done, ..) for each surface on the removed
                    // output, and that handler is what actually recreates
                    // them (on this or another output, via output_for).
                    return self.reconcile_surfaces();
                }
                Task::none()
            }
            Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, _, id)) => {
                // The compositor closed this surface (its output went away).
                if let Some(handle) = self.client_for_surface(id) {
                    tracing::info!("layer surface closed by compositor; recreating");
                    // A drag on a surface that goes away must not linger and
                    // block future drags (mirrors destroy_surface).
                    if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                        self.drag = None;
                    }
                    if let Some(c) = self.clients.get_mut(&handle) {
                        c.surface = None;
                    }
                    return self.reconcile_surfaces();
                }
                Task::none()
            }
            Msg::Wayland(WaylandEvent::Layer(..)) => Task::none(),
            Msg::Wayland(_) => Task::none(),
            Msg::Backend(event) => self.on_backend(event),
            Msg::Pointer(id, event) => self.on_pointer(id, event),
            Msg::ConfigChanged(config) => self.apply_config(config),
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
        let mut subs = vec![events, config_watch::subscription().map(Msg::ConfigChanged)];
        if let Some(conn) = self.conn.clone() {
            subs.push(
                backend::subscription(conn, self.config.app_ids.clone(), self.config.fps)
                    .map(Msg::Backend),
            );
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Msg> {
        unreachable!("no main window")
    }

    /// `run_single_instance` activates the already-running instance and lets
    /// this dbus-activation request arrive here instead of spawning a second
    /// process. Plan 4's IPC will give it a real message; for now, log so a
    /// second launch is visible rather than silently doing nothing.
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

    fn view_window(&self, id: SurfaceId) -> Element<'_, Msg> {
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
