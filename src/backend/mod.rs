//! Compositor thread: a second event queue on iced's Wayland connection,
//! driven by calloop. Tracks toplevels, activates them, and (Task 5)
//! captures them. Modelled on cosmic-workspaces' backend.

use cosmic::cctk::{
    self,
    sctk::{
        registry::{ProvidesRegistryState, RegistryState},
        seat::{SeatHandler, SeatState},
    },
    toplevel_info::{ToplevelInfo, ToplevelInfoState},
    toplevel_management::ToplevelManagerState,
    wayland_client::{
        Connection, QueueHandle,
        globals::registry_queue_init,
        protocol::{wl_output::{self, WlOutput}, wl_seat},
    },
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
};
use cosmic::cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State;
use cosmic::cctk::{
    screencopy::ScreencopyState,
    sctk::{
        dmabuf::{DmabufFeedback, DmabufState},
        shm::{Shm, ShmHandler},
    },
};
use cosmic::iced::futures::executor::ThreadPool;
use cosmic::iced::platform_specific::shell::subsurface_widget::SubsurfaceBuffer;
use cosmic::iced::{
    self,
    futures::{FutureExt, SinkExt, channel::mpsc, executor::block_on},
};
use cosmic::cctk::cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1;
use calloop_wayland_source::WaylandSource;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{hash::Hash, thread};

use crate::model::client::{Login, classify};

mod toplevels;
mod buffer;
mod capture;
mod dmabuf;
mod gbm_devices;

pub type Handle = ExtForeignToplevelHandleV1;

#[derive(Clone, Debug, PartialEq)]
pub struct ClientInfo {
    pub login: Login,
    pub activated: bool,
    pub minimized: bool,
    pub outputs: Vec<WlOutput>,
}

#[derive(Clone, Debug)]
pub struct CaptureImage {
    pub buffer: SubsurfaceBuffer,
    pub width: u32,
    pub height: u32,
    pub transform: wl_output::Transform,
}

#[derive(Clone, Debug)]
pub enum Event {
    /// Sent once at startup so the UI can send commands.
    CmdSender(calloop::channel::Sender<Cmd>),
    ClientAdded(Handle, ClientInfo),
    ClientUpdated(Handle, ClientInfo),
    ClientRemoved(Handle),
    Frame(Handle, CaptureImage),
    /// Capture for this client could not be (re)started; the UI should grey it out
    /// until the next `Frame`.
    CaptureUnavailable(Handle),
}

#[derive(Debug)]
pub enum Cmd {
    Activate(Handle),
    Minimize(Handle),
    SetAppIds(Vec<String>),
    SetFps(u32),
}

/// iced subscription that owns the backend thread for the app's lifetime.
pub fn subscription(conn: Connection, app_ids: Vec<String>, fps: u32) -> iced::Subscription<Event> {
    #[derive(Clone)]
    struct Key {
        conn: Connection,
        app_ids: Vec<String>,
        fps: u32,
    }
    impl Hash for Key {
        // Hash only the display id on purpose: config changes must go
        // through `Cmd::SetAppIds`/`SetFps`, not restart the backend.
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.conn.backend().display_id().hash(state);
        }
    }
    // `Subscription::run_with` (this pinned iced) takes a bare `fn(&D) -> S`,
    // so the per-call `app_ids`/`fps` must travel inside the hashable `Key`
    // rather than being captured by a closure. Boxing the stream sidesteps
    // an HRTB mismatch between the `impl Trait` return type and the fn
    // pointer `run_with` expects.
    fn run(key: &Key) -> iced::futures::stream::BoxStream<'static, Event> {
        let conn = key.conn.clone();
        let app_ids = key.app_ids.clone();
        let fps = key.fps;
        Box::pin(async move { start(conn, app_ids, fps) }.flatten_stream())
    }
    iced::Subscription::run_with(Key { conn, app_ids, fps }, run)
}

pub struct AppData {
    pub qh: QueueHandle<Self>,
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub toplevel_info_state: ToplevelInfoState,
    pub toplevel_manager_state: Option<ToplevelManagerState>,
    pub screencopy_state: ScreencopyState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_feedback: Option<DmabufFeedback>,
    pub gbm_devices: gbm_devices::GbmDevices,
    pub shm_state: Shm,
    pub captures: HashMap<Handle, Arc<capture::Capture>>,
    pub thread_pool: ThreadPool,
    pub sender: mpsc::Sender<Event>,
    pub app_ids: Vec<String>,
    pub fps: Arc<AtomicU32>,
    pub capabilities: HashSet<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>,
}

impl AppData {
    pub fn send_event(&mut self, event: Event) {
        let _ = block_on(self.sender.send(event));
    }

    /// `None` when the toplevel is not an EVE client.
    pub fn client_info(&self, info: &ToplevelInfo) -> Option<ClientInfo> {
        let login = classify(&self.app_ids, &info.app_id, &info.title)?;
        Some(ClientInfo {
            login,
            activated: info.state.contains(&State::Activated),
            minimized: info.state.contains(&State::Minimized),
            outputs: info.output.iter().cloned().collect(),
        })
    }

    fn cosmic_handle(
        &self,
        handle: &Handle,
    ) -> Option<cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>
    {
        self.toplevel_info_state
            .info(handle)
            .and_then(|info| info.cosmic_toplevel.clone())
    }

    pub fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Activate(handle) => {
                let Some(cosmic) = self.cosmic_handle(&handle) else { return };
                let Some(manager) = &self.toplevel_manager_state else { return };
                for seat in self.seat_state.seats() {
                    manager.manager.activate(&cosmic, &seat);
                }
            }
            Cmd::Minimize(handle) => {
                use zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1 as Cap;
                if !self.capabilities.contains(&Cap::Minimize) {
                    tracing::warn!("compositor does not advertise the Minimize capability; ignoring");
                    return;
                }
                let Some(cosmic) = self.cosmic_handle(&handle) else { return };
                let Some(manager) = &self.toplevel_manager_state else { return };
                manager.manager.set_minimized(&cosmic);
            }
            Cmd::SetAppIds(app_ids) => {
                self.app_ids = app_ids;
                self.reclassify_all();
            }
            Cmd::SetFps(fps) => {
                self.fps.store(fps, Ordering::Relaxed);
            }
        }
    }
}

fn start(conn: Connection, app_ids: Vec<String>, fps: u32) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(64);

    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(result) => result,
        Err(err) => {
            tracing::error!("cannot initialize wayland registry: {err}");
            eprintln!("yutani: cannot initialize wayland registry: {err}");
            std::process::exit(1);
        }
    };
    let qh = event_queue.handle();

    // Fail fast rather than leave a windowless zombie process: if the
    // compositor cannot offer what Yutani needs, say so and exit instead of
    // spawning a backend thread that can never do anything useful.
    let advertised: Vec<(String, u32)> =
        globals.contents().clone_list().into_iter().map(|g| (g.interface, g.version)).collect();
    let checks = crate::doctor::evaluate(&advertised);
    if !crate::doctor::all_required_present(&checks) {
        let missing: Vec<&str> =
            checks.iter().filter(|c| c.required && c.found.is_none()).map(|c| c.interface).collect();
        tracing::error!("compositor is missing required protocols: {}", missing.join(", "));
        eprintln!("yutani: compositor is missing required protocols: {} (run `yutani doctor`)", missing.join(", "));
        std::process::exit(2);
    }

    thread::Builder::new()
        .name("yutani-backend".into())
        .spawn(move || {
            // A panic anywhere in the backend thread must not leave the UI
            // running windowless with a dead backend; bring the whole
            // process down instead.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let dmabuf_state = DmabufState::new(&globals, &qh);
                if let Err(err) = dmabuf_state.get_default_feedback(&qh) {
                    tracing::warn!("dmabuf feedback unsupported; shm only: {err}");
                }
                let registry_state = RegistryState::new(&globals);
                let mut app_data = AppData {
                    qh: qh.clone(),
                    seat_state: SeatState::new(&globals, &qh),
                    toplevel_info_state: ToplevelInfoState::new(&registry_state, &qh),
                    toplevel_manager_state: ToplevelManagerState::try_new(&registry_state, &qh),
                    screencopy_state: ScreencopyState::new(&globals, &qh),
                    dmabuf_state,
                    dmabuf_feedback: None,
                    gbm_devices: gbm_devices::GbmDevices::default(),
                    shm_state: Shm::bind(&globals, &qh).expect("wl_shm"),
                    captures: HashMap::new(),
                    thread_pool: ThreadPool::builder().pool_size(1).create().expect("thread pool"),
                    registry_state,
                    sender,
                    app_ids,
                    fps: Arc::new(AtomicU32::new(fps)),
                    capabilities: HashSet::new(),
                };

                let (cmd_sender, cmd_channel) = calloop::channel::channel();
                app_data.send_event(Event::CmdSender(cmd_sender));

                let mut event_loop = calloop::EventLoop::try_new().expect("calloop");
                WaylandSource::new(conn, event_queue)
                    .insert(event_loop.handle())
                    .expect("wayland source");
                event_loop
                    .handle()
                    .insert_source(cmd_channel, |event, _, app_data: &mut AppData| {
                        if let calloop::channel::Event::Msg(cmd) = event {
                            app_data.handle_cmd(cmd);
                        }
                    })
                    .expect("cmd channel");

                loop {
                    if let Err(err) = event_loop.dispatch(None, &mut app_data) {
                        tracing::error!("backend event loop failed: {err}");
                        std::process::exit(1);
                    }
                }
            }));
            if result.is_err() {
                tracing::error!("backend thread panicked");
                std::process::exit(1);
            }
        })
        .expect("spawn backend thread");

    receiver
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    // Deliberately no OutputState: all wl_output handles are iced's.
    cctk::sctk::registry_handlers!(SeatState);
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: cctk::sctk::seat::Capability,
    ) {
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: cctk::sctk::seat::Capability,
    ) {
    }
}

cctk::sctk::delegate_registry!(AppData);
cctk::sctk::delegate_seat!(AppData);

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

cctk::sctk::delegate_shm!(AppData);
