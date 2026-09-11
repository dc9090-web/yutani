//! libcosmic application: no main window; one overlay layer surface per
//! EVE client, each showing that client's live captured frame.

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::{Connection, Proxy, protocol::wl_output::WlOutput};
use cosmic::iced::event::wayland::{Event as WaylandEvent, OutputEvent};
use cosmic::iced::mouse;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, set_margin, set_size,
};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{self, Point, Subscription};
use cosmic::{Application, Element, Task};
use std::collections::HashMap;

use crate::backend::{self, CaptureImage, ClientInfo, Cmd, Event, Handle};
use crate::model::client::Login;
use crate::model::config::Config;
use crate::model::layout::{self, Layout, Rect, ThumbPos};

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

    fn create_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        if client.surface.is_some() {
            return Task::none();
        }
        let Some(output) = self.output_for(&client.info) else {
            tracing::debug!("no outputs yet; deferring surface");
            return Task::none();
        };
        // Prefer a saved position for this character over the next free slot.
        let saved = match &client.info.login {
            Login::LoggedIn(name) => self.layout.thumbs.get(name).cloned(),
            Login::LoggingIn => None,
        };
        let position = saved.as_ref().map(|s| (s.x, s.y)).unwrap_or_else(|| self.next_position());
        let pinned = saved.as_ref().map(|s| s.pinned).unwrap_or(false);
        let (width, height) = thumbnail::size(&self.config, client.image.as_ref());
        let id = SurfaceId::unique();
        let client = self.clients.get_mut(handle).unwrap();
        client.surface = Some(id);
        client.position = position;
        client.pinned = pinned;
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
            ..Default::default()
        })
    }

    fn destroy_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        match self.clients.get_mut(handle).and_then(|c| c.surface.take()) {
            Some(id) => destroy_layer_surface(id),
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
                if self.drag.is_none() {
                    let c = &self.clients[&handle];
                    // Position of the cursor at press time is not part of ButtonPressed;
                    // use the last CursorMoved we saw for this surface.
                    let cursor = c.last_cursor;
                    self.drag = pointer::on_press(id, button, cursor, c.position, c.pinned);
                }
                Task::none()
            }
            mouse::Event::CursorMoved { position } => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.last_cursor = position;
                }
                let Some(drag) = self.drag.as_mut().filter(|d| d.surface == id) else { return Task::none() };
                let current = self.clients[&handle].position;
                match pointer::on_move(drag, position, current) {
                    pointer::Outcome::Move(raw) => {
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
                        let (x, y) = (x.max(0), y.max(0));
                        let (w, h) = (me.w, me.h);
                        let output_size = self
                            .output_for(&self.clients[&handle].info)
                            .and_then(|o| self.outputs.iter().find(|out| out.handle == o).map(|out| out.logical_size));
                        let (x, y) = match output_size {
                            Some((ow, oh)) if ow > 0 && oh > 0 => {
                                (x.min((ow - w).max(0)), y.min((oh - h).max(0)))
                            }
                            _ => (x, y),
                        };
                        self.clients.get_mut(&handle).unwrap().position = (x, y);
                        set_margin(id, y, 0, 0, x)
                    }
                    _ => Task::none(),
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if !self.drag.as_ref().is_some_and(|d| d.surface == id && d.button == button) {
                    return Task::none();
                }
                let drag = self.drag.take().unwrap();
                match pointer::on_release(drag) {
                    pointer::Outcome::Click(mouse::Button::Left) => {
                        self.send(Cmd::Activate(handle));
                    }
                    pointer::Outcome::Click(mouse::Button::Right) => {
                        self.send(Cmd::Minimize(handle));
                    }
                    pointer::Outcome::DragEnd => self.persist_position(&handle),
                    _ => {}
                }
                Task::none()
            }
            mouse::Event::CursorLeft => {
                // Releasing outside is delivered to us anyway (implicit grab); nothing to do.
                Task::none()
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
                });
                let was_named = matches!(entry.info.login, Login::LoggedIn(_));
                let had_surface = entry.surface.is_some();
                entry.info = info;
                let became_named = !was_named && matches!(entry.info.login, Login::LoggedIn(_));
                let create = self.create_surface(&handle);
                if became_named && had_surface {
                    Task::batch([create, self.apply_saved_position(&handle)])
                } else {
                    create
                }
            }
            Event::ClientRemoved(handle) => {
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                task
            }
            Event::Frame(handle, image) => {
                let Some(client) = self.clients.get_mut(&handle) else { return Task::none() };
                let old = thumbnail::size(&self.config, client.image.as_ref());
                let new = thumbnail::size(&self.config, Some(&image));
                client.image = Some(image);
                match client.surface {
                    Some(id) if old != new => set_size(id, Some(new.0), Some(new.1)),
                    Some(_) => Task::none(),
                    None => self.create_surface(&handle),
                }
            }
            Event::CaptureUnavailable(handle) => {
                if let Some(c) = self.clients.get(&handle) {
                    tracing::warn!(label = c.info.login.label(), "capture unavailable");
                }
                Task::none()
            }
        }
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
                Task::none()
            }
            Msg::Wayland(_) => Task::none(),
            Msg::Backend(event) => self.on_backend(event),
            Msg::Pointer(id, event) => self.on_pointer(id, event),
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let events = iced::event::listen_with(|event, _status, id| match event {
            iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(
                event @ WaylandEvent::Output(..),
            )) => Some(Msg::Wayland(event)),
            iced::Event::Mouse(m) => Some(Msg::Pointer(id, m)),
            // Every other event (RequestResize, Frame, keyboard, …) must not become
            // a message: update → redraw → same event again is a hot loop.
            _ => None,
        });
        let mut subs = vec![events];
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

    fn view_window(&self, id: SurfaceId) -> Element<'_, Msg> {
        match self.clients.iter().find(|(_, c)| c.surface == Some(id)) {
            Some((_, client)) => thumbnail::view(client, &self.config),
            None => cosmic::widget::text("").into(),
        }
    }
}
