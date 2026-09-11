//! Per-client capture session: 2-buffer swapchain, damage-driven, throttled
//! to `fps`. The front buffer is shipped to the UI as a `SubsurfaceBuffer`;
//! the next capture into it waits for the compositor's release.

use cosmic::cctk::{
    self,
    screencopy::{
        CaptureFrame, CaptureOptions, CaptureSession, CaptureSource, FailureReason, Formats, Frame,
        ScreencopyFrameData, ScreencopyFrameDataExt, ScreencopyHandler, ScreencopySessionData,
        ScreencopySessionDataExt, ScreencopyState,
    },
    wayland_client::{Connection, QueueHandle, WEnum},
};
use cosmic::iced::platform_specific::shell::subsurface_widget::{SubsurfaceBuffer, SubsurfaceBufferRelease};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::buffer::Buffer;
use super::{AppData, CaptureImage, Event, Handle};

const BUFFER_COUNT: usize = 2;

pub struct Capture {
    pub handle: Handle,
    pub session: Mutex<Option<ScreencopySession>>,
}

impl Capture {
    pub fn new(handle: Handle) -> Arc<Self> {
        Arc::new(Capture { handle, session: Mutex::new(None) })
    }

    pub fn for_session(session: &CaptureSession) -> Option<Arc<Self>> {
        session.data::<SessionData>()?.capture.upgrade()
    }

    pub fn start(self: &Arc<Self>, screencopy: &ScreencopyState, qh: &QueueHandle<AppData>) {
        let mut session = self.session.lock().unwrap();
        if session.is_none() {
            *session = ScreencopySession::new(self, screencopy, qh);
        }
    }

    pub fn stop(&self) {
        self.session.lock().unwrap().take();
    }
}

pub struct ScreencopySession {
    formats: Option<Formats>,
    /// [front, back]; rotated on every ready frame.
    buffers: Option<[Buffer; BUFFER_COUNT]>,
    session: CaptureSession,
    release: Option<SubsurfaceBufferRelease>,
    last_submit: Instant,
}

impl ScreencopySession {
    fn new(capture: &Arc<Capture>, screencopy: &ScreencopyState, qh: &QueueHandle<AppData>) -> Option<Self> {
        let udata = SessionData { session_data: Default::default(), capture: Arc::downgrade(capture) };
        let source = CaptureSource::Toplevel(capture.handle.clone());
        match screencopy.capturer().create_session(&source, CaptureOptions::empty(), qh, udata) {
            Ok(session) => Some(Self { formats: None, buffers: None, session, release: None, last_submit: Instant::now() }),
            Err(err) => {
                tracing::error!("cannot create capture session: {err:?}");
                None
            }
        }
    }

    fn submit(&mut self, capture: &Arc<Capture>, conn: &Connection, qh: &QueueHandle<AppData>) {
        let Some(back) = self.buffers.as_ref().map(|b| &b[1]) else { return };
        self.session.capture(
            &back.buffer,
            &back.damage,
            qh,
            FrameData { frame_data: Default::default(), capture: Arc::downgrade(capture) },
        );
        self.last_submit = Instant::now();
        let _ = conn.flush();
    }
}

pub struct SessionData {
    session_data: ScreencopySessionData,
    capture: Weak<Capture>,
}

impl ScreencopySessionDataExt for SessionData {
    fn screencopy_session_data(&self) -> &ScreencopySessionData {
        &self.session_data
    }
}

struct FrameData {
    frame_data: ScreencopyFrameData,
    capture: Weak<Capture>,
}

impl ScreencopyFrameDataExt for FrameData {
    fn screencopy_frame_data(&self) -> &ScreencopyFrameData {
        &self.frame_data
    }
}

impl AppData {
    pub fn start_capture(&mut self, handle: &Handle) {
        let capture = self.captures.entry(handle.clone()).or_insert_with(|| Capture::new(handle.clone())).clone();
        capture.start(&self.screencopy_state, &self.qh);
    }

    pub fn stop_capture(&mut self, handle: &Handle) {
        if let Some(capture) = self.captures.remove(handle) {
            capture.stop();
        }
    }

    fn allocate(&mut self, formats: &Formats) -> Option<[Buffer; BUFFER_COUNT]> {
        let a = self.create_buffer(formats);
        let b = self.create_buffer(formats);
        match (a, b) {
            (Ok(a), Ok(b)) => Some([a, b]),
            (Err(err), _) | (_, Err(err)) => {
                tracing::error!("cannot allocate capture buffers: {err}");
                None
            }
        }
    }
}

fn frame_interval(fps: &AtomicU32) -> Duration {
    Duration::from_secs_f64(1.0 / fps.load(Ordering::Relaxed).max(1) as f64)
}

impl ScreencopyHandler for AppData {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }

    fn init_done(&mut self, conn: &Connection, qh: &QueueHandle<Self>, session: &CaptureSession, formats: &Formats) {
        let Some(capture) = Capture::for_session(session) else { return };
        let buffers = self.allocate(formats);
        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        let resized = state.formats.as_ref().is_some_and(|f| f.buffer_size != formats.buffer_size);
        state.formats = Some(formats.clone());
        if state.buffers.is_none() || resized {
            state.buffers = buffers;
            state.release = None;
            state.submit(&capture, conn, qh);
        }
    }

    fn ready(&mut self, conn: &Connection, qh: &QueueHandle<Self>, capture_frame: &CaptureFrame, frame: Frame) {
        let Some(capture) = capture_frame.data::<FrameData>().and_then(|d| d.capture.upgrade()) else { return };
        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        let Some(buffers) = state.buffers.as_mut() else { return };

        // Back buffer now holds the newest frame: make it the front.
        buffers.rotate_left(1);
        buffers[0].damage.clear();
        for buffer in &mut buffers[1..] {
            buffer.damage.extend_from_slice(&frame.damage);
        }

        let front = &buffers[0];
        let (subsurface_buffer, release) = SubsurfaceBuffer::new(front.backing.clone());
        let image = CaptureImage {
            buffer: subsurface_buffer,
            width: front.size.0,
            height: front.size.1,
            transform: match frame.transform {
                WEnum::Value(t) => t,
                WEnum::Unknown(_) => cctk::wayland_client::protocol::wl_output::Transform::Normal,
            },
        };
        let previous_release = state.release.replace(release);

        // Next capture: after the previous front buffer is released by the
        // compositor and at least one frame interval since the last submit.
        let wait = frame_interval(&self.fps).saturating_sub(state.last_submit.elapsed());
        let capture_for_task = capture.clone();
        let conn = conn.clone();
        let qh = qh.clone();
        self.thread_pool.spawn_ok(async move {
            if let Some(release) = previous_release {
                release.await;
            }
            if !wait.is_zero() {
                futures_timer::Delay::new(wait).await;
            }
            let mut guard = capture_for_task.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                state.submit(&capture_for_task, &conn, &qh);
            }
        });

        drop(guard);
        self.send_event(Event::Frame(capture.handle.clone(), image));
    }

    fn failed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, capture_frame: &CaptureFrame, reason: WEnum<FailureReason>) {
        let Some(capture) = capture_frame.data::<FrameData>().and_then(|d| d.capture.upgrade()) else { return };
        match reason {
            WEnum::Value(FailureReason::BufferConstraints) => {
                tracing::info!("buffer constraints changed; reallocating");
                let formats = capture.session.lock().unwrap().as_ref().and_then(|s| s.formats.clone());
                let Some(formats) = formats else { return };
                let buffers = self.allocate(&formats);
                let mut guard = capture.session.lock().unwrap();
                if let Some(state) = guard.as_mut() {
                    state.buffers = buffers;
                    state.release = None;
                    state.submit(&capture, conn, qh);
                }
            }
            WEnum::Value(FailureReason::Stopped) => {
                tracing::info!("capture stopped by compositor");
                capture.stop();
            }
            other => {
                tracing::warn!("capture failed: {other:?}; retrying in 500ms");
                let capture_for_task = capture.clone();
                let conn = conn.clone();
                let qh = qh.clone();
                self.thread_pool.spawn_ok(async move {
                    futures_timer::Delay::new(Duration::from_millis(500)).await;
                    let mut guard = capture_for_task.session.lock().unwrap();
                    if let Some(state) = guard.as_mut() {
                        state.submit(&capture_for_task, &conn, &qh);
                    }
                });
            }
        }
    }

    fn stopped(&mut self, _: &Connection, _: &QueueHandle<Self>, session: &CaptureSession) {
        if let Some(capture) = Capture::for_session(session) {
            capture.stop();
        }
    }
}

cctk::delegate_screencopy!(AppData);
