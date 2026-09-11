//! libcosmic application: no main window; one overlay layer surface per
//! EVE client (Task 6). This task only wires events and logs them.

use cosmic::cctk::sctk::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use cosmic::cctk::wayland_client::{Connection, Proxy, protocol::wl_output::WlOutput};
use cosmic::iced::event::wayland::{Event as WaylandEvent, OutputEvent};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, set_size,
};
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{self, Subscription};
use cosmic::{Application, Element, Task};
use std::collections::HashMap;

use crate::backend::{self, CaptureImage, ClientInfo, Cmd, Event, Handle};
use crate::model::config::Config;

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
}

pub struct App {
    core: cosmic::app::Core,
    pub config: Config,
    pub conn: Option<Connection>,
    pub cmd: Option<calloop::channel::Sender<Cmd>>,
    pub clients: HashMap<Handle, Client>,
    pub outputs: Vec<Output>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Wayland(WaylandEvent),
    Backend(Event),
    Activate(Handle),
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

    /// Next free slot: a row along the top, left to right.
    fn next_position(&self) -> (i32, i32) {
        let (w, _) = thumbnail::size(&self.config, None);
        let used = self.clients.values().filter(|c| c.surface.is_some()).count() as i32;
        (40 + used * (w as i32 + 16), 40)
    }

    fn create_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        if client.surface.is_some() {
            return Task::none();
        }
        let Some(output) = self.output_for(&client.info) else {
            tracing::warn!("no outputs yet; deferring surface");
            return Task::none();
        };
        let position = self.next_position();
        let (width, height) = thumbnail::size(&self.config, client.image.as_ref());
        let id = SurfaceId::unique();
        let client = self.clients.get_mut(handle).unwrap();
        client.surface = Some(id);
        client.position = position;
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
                });
                entry.info = info;
                self.create_surface(&handle)
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
            Msg::Activate(handle) => {
                self.send(Cmd::Activate(handle));
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let wayland = iced::event::listen_with(|event, _, _| match event {
            iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(event)) => {
                Some(Msg::Wayland(event))
            }
            _ => None,
        });
        let mut subs = vec![wayland];
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
            Some((handle, client)) => thumbnail::view(client, &self.config, Msg::Activate(handle.clone())),
            None => cosmic::widget::text("").into(),
        }
    }
}
