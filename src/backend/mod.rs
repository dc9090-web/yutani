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
use anyhow::Context as _;
use calloop_wayland_source::WaylandSource;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{hash::Hash, thread};

use crate::model::client::{Login, classify};
use buffer::Buffer;

mod toplevels;
mod buffer;
mod capture;
mod dmabuf;
mod gbm_devices;
pub mod gl;

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
    PauseCapture(Handle),
    ResumeCapture(Handle),
    /// Physical-pixel size of this client's thumbnail surface; the GL pass
    /// renders into buffers of exactly this size.
    SetThumbSize(Handle, (u32, u32)),
    /// Corner mask radius in physical pixels (0 = square).
    SetCornerRadius(u32),
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

/// The GL thumbnail pass is created lazily on the first frame for which the
/// UI has sent a thumbnail size (it needs the compositor's main device) and
/// disabled for good after repeated failures.
pub enum GlState {
    Untried,
    Ready { gl: gl::Gl, consecutive_failures: u32 },
    Disabled,
}

/// Consecutive per-frame GL failures before the pass is switched off.
pub const GL_MAX_FAILURES: u32 = 3;

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
    pub gl: GlState,
    pub thumb_sizes: HashMap<Handle, (u32, u32)>,
    pub corner_radius_px: u32,
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

    pub fn handle_cmd(&mut self, cmd: Cmd, conn: &Connection) {
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
            Cmd::PauseCapture(handle) => {
                self.set_paused(&handle, true, conn);
            }
            Cmd::ResumeCapture(handle) => {
                self.set_paused(&handle, false, conn);
            }
            Cmd::SetThumbSize(handle, size) => {
                if size.0 == 0 || size.1 == 0 {
                    tracing::debug!("ignoring zero-sized thumb size {size:?} for {handle:?}");
                    return;
                }
                self.thumb_sizes.insert(handle, size);
            }
            Cmd::SetCornerRadius(px) => {
                self.corner_radius_px = px;
            }
        }
    }

    /// The GL pass, initialising it on first call. `false` when unavailable.
    fn gl_init(&mut self) -> bool {
        if !matches!(self.gl, GlState::Untried) {
            return matches!(self.gl, GlState::Ready { .. });
        }
        // No feedback yet (or the compositor never sent one): stay `Untried`
        // rather than `Disabled` so a later frame, once feedback arrives,
        // retries this instead of being stuck without GL for the session.
        let Some(dev) = self.dmabuf_feedback.as_ref().map(|f| f.main_device()) else { return false };
        let gbm = match self.gbm_devices.gbm_device(dev) {
            Ok(Some((_, gbm))) => gbm,
            Ok(None) => {
                tracing::warn!("no gbm device for the compositor's main device; thumbnails will have square corners");
                self.gl = GlState::Disabled;
                return false;
            }
            Err(err) => {
                tracing::warn!("cannot open gbm device: {err}; thumbnails will have square corners");
                self.gl = GlState::Disabled;
                return false;
            }
        };
        match gl::Gl::new(gbm) {
            Ok(gl) => {
                self.gl = GlState::Ready { gl, consecutive_failures: 0 };
                true
            }
            Err(err) => {
                tracing::warn!("GL thumbnail pass unavailable: {err:#}; thumbnails will have square corners");
                self.gl = GlState::Disabled;
                false
            }
        }
    }

    /// Record a per-frame GL failure; after `GL_MAX_FAILURES` in a row the
    /// pass is disabled for the rest of the session.
    fn gl_failed(&mut self, err: anyhow::Error) {
        if let GlState::Ready { consecutive_failures, .. } = &mut self.gl {
            *consecutive_failures += 1;
            if *consecutive_failures >= GL_MAX_FAILURES {
                tracing::warn!("GL thumbnail pass failed {GL_MAX_FAILURES} times ({err:#}); disabling, thumbnails will have square corners");
                self.gl = GlState::Disabled;
            } else {
                tracing::debug!("GL thumbnail pass failed: {err:#}; raw frame this time");
            }
        }
    }

    /// Modifiers the compositor accepts for ABGR8888, from its feedback.
    fn thumb_modifiers(&self) -> Vec<u64> {
        let Some(fb) = self.dmabuf_feedback.as_ref() else { return Vec::new() };
        let table: Vec<(u32, u64)> = fb.format_table().iter().map(|f| (f.format, f.modifier)).collect();
        let tranches: Vec<&[u16]> = fb.tranches().iter().map(|t| t.formats.as_slice()).collect();
        gl::modifiers_for(&table, &tranches, gl::ABGR8888)
    }

    /// Run the GL pass for `front` into `thumb` (allocating or re-allocating
    /// the pool for `size`), returning the buffer to ship. `Err` means the
    /// caller ships the raw frame.
    fn gl_process(
        &mut self,
        front: &mut Buffer,
        thumb: &mut Option<capture::ThumbPool>,
        size: (u32, u32),
        transform: wl_output::Transform,
    ) -> anyhow::Result<Arc<cosmic::iced::platform_specific::shell::subsurface_widget::BufferSource>> {
        if !self.gl_init() {
            anyhow::bail!("GL pass unavailable");
        }
        // Only (re)computed when the pool is about to be (re)allocated: this
        // walks the compositor's whole format table/tranches, which is
        // wasted work on the common per-frame path where the pool is reused.
        let needs_pool = thumb.as_ref().is_none_or(|t| t.size != size);
        let modifiers = if needs_pool { self.thumb_modifiers() } else { Vec::new() };
        let dev = self.dmabuf_feedback.as_ref().map(|f| f.main_device()).context("no dmabuf feedback")?;
        let radius = self.corner_radius_px;
        let AppData { gl, gbm_devices, .. } = self;
        let GlState::Ready { gl, consecutive_failures } = gl else { anyhow::bail!("GL pass unavailable") };
        let (_, gbm) = gbm_devices.gbm_device(dev)?.context("gbm device vanished")?;

        if needs_pool {
            *thumb = Some(capture::ThumbPool {
                size,
                targets: [gl.create_target(gbm, &modifiers, size)?, gl.create_target(gbm, &modifiers, size)?],
                release: None,
            });
        }
        let pool = thumb.as_mut().unwrap();
        if front.source.is_none() {
            front.source = Some(gl.import_source(&front.backing)?);
        }
        // Render into the back target, then make it the front.
        gl.render(front.source.as_ref().unwrap(), &pool.targets[1], transform, radius)?;
        pool.targets.rotate_left(1);
        *consecutive_failures = 0;
        Ok(pool.targets[0].backing.clone())
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
                    gl: GlState::Untried,
                    thumb_sizes: HashMap::new(),
                    corner_radius_px: 8,
                };

                let (cmd_sender, cmd_channel) = calloop::channel::channel();
                app_data.send_event(Event::CmdSender(cmd_sender));

                let mut event_loop = calloop::EventLoop::try_new().expect("calloop");
                // `handle_cmd` needs a `Connection` to flush after commands like
                // pause/resume; clone it before `conn` is moved into the source.
                let cmd_conn = conn.clone();
                WaylandSource::new(conn, event_queue)
                    .insert(event_loop.handle())
                    .expect("wayland source");
                event_loop
                    .handle()
                    .insert_source(cmd_channel, move |event, _, app_data: &mut AppData| {
                        if let calloop::channel::Event::Msg(cmd) = event {
                            app_data.handle_cmd(cmd, &cmd_conn);
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
