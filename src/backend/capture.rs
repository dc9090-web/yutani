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
use cosmic::iced::platform_specific::shell::subsurface_widget::{BufferSource, SubsurfaceBuffer, SubsurfaceBufferRelease};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::buffer::Buffer;
use super::{AppData, CaptureImage, Event, Handle};

const BUFFER_COUNT: usize = 2;

/// Thumbnail-sized GL targets for one session: [front, back], rotated on
/// every processed frame. `release` is the compositor's release of the
/// front target (the next render into it must wait for that).
pub struct ThumbPool {
    pub size: (u32, u32),
    pub targets: [super::gl::Target; BUFFER_COUNT],
    pub release: Option<SubsurfaceBufferRelease>,
}

pub struct Capture {
    pub handle: Handle,
    pub session: Mutex<Option<ScreencopySession>>,
    /// Set by `AppData::set_paused` when the UI hides this client's
    /// thumbnail: capture stops submitting new frames until resumed.
    pub paused: AtomicBool,
}

impl Capture {
    pub fn new(handle: Handle) -> Arc<Self> {
        Arc::new(Capture { handle, session: Mutex::new(None), paused: AtomicBool::new(false) })
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
    /// GL thumbnail targets, allocated on the first processed frame.
    pub thumb: Option<ThumbPool>,
    last_submit: Instant,
    consecutive_failures: u32,
    /// At most one `ext_image_copy_capture_frame` may be outstanding per
    /// session; a second one before the first resolves is a fatal
    /// `duplicate_frame` protocol error.
    in_flight: bool,
}

impl ScreencopySession {
    fn new(capture: &Arc<Capture>, screencopy: &ScreencopyState, qh: &QueueHandle<AppData>) -> Option<Self> {
        let udata = SessionData { session_data: Default::default(), capture: Arc::downgrade(capture) };
        let source = CaptureSource::Toplevel(capture.handle.clone());
        match screencopy.capturer().create_session(&source, CaptureOptions::empty(), qh, udata) {
            Ok(session) => Some(Self {
                formats: None,
                buffers: None,
                session,
                release: None,
                thumb: None,
                last_submit: Instant::now(),
                consecutive_failures: 0,
                in_flight: false,
            }),
            Err(err) => {
                tracing::error!("cannot create capture session: {err:?}");
                None
            }
        }
    }

    fn submit(&mut self, capture: &Arc<Capture>, conn: &Connection, qh: &QueueHandle<AppData>) {
        if capture.paused.load(Ordering::Relaxed) {
            return;
        }
        if self.in_flight {
            return;
        }
        let Some(back) = self.buffers.as_ref().map(|b| &b[1]) else { return };
        self.session.capture(
            &back.buffer,
            &back.damage,
            qh,
            FrameData { frame_data: Default::default(), capture: Arc::downgrade(capture) },
        );
        self.in_flight = true;
        self.last_submit = Instant::now();
        let _ = conn.flush();
    }
}

/// Exponential backoff for `failed` retries: 250 ms, 500 ms, 1 s, then
/// capped at 2 s ("250 ms → 2 s").
pub fn retry_delay_ms(consecutive_failures: u32) -> u64 {
    250u64.saturating_mul(1u64 << consecutive_failures.saturating_sub(1).min(3)).min(2000)
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

    /// Pause (or resume) capture for a client whose thumbnail the UI has
    /// hidden (or shown again). Resuming re-submits immediately if nothing
    /// is already in flight; `submit`'s own `in_flight` guard makes this
    /// safe to call unconditionally.
    pub fn set_paused(&mut self, handle: &Handle, paused: bool, conn: &Connection) {
        let Some(capture) = self.captures.get(handle).cloned() else { return };
        capture.paused.store(paused, Ordering::Relaxed);
        if !paused {
            let mut guard = capture.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                state.submit(&capture, conn, &self.qh);
            }
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

        // At most one frame may be in flight per session; a second `init_done`
        // (e.g. after a resize) can race with a frame that's already in
        // flight, so only the first `init_done` (no buffers yet) may submit.
        // Later resizes are handled by `failed(BufferConstraints)`, which is
        // only reached once nothing is in flight.
        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        state.formats = Some(formats.clone());
        if state.buffers.is_some() {
            return;
        }
        drop(guard);

        let buffers = self.allocate(formats);
        let unavailable = buffers.is_none();

        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        state.buffers = buffers;
        state.release = None;
        state.submit(&capture, conn, qh);
        drop(guard);

        if unavailable {
            // No buffers: this session can't produce frames. Stop it so the
            // next state change (`start_capture`) creates a fresh one instead
            // of no-op-ing on a session stuck with `buffers = None`.
            capture.stop();
            self.send_event(Event::CaptureUnavailable(capture.handle.clone()));
        }
    }

    fn ready(&mut self, conn: &Connection, qh: &QueueHandle<Self>, capture_frame: &CaptureFrame, frame: Frame) {
        let Some(capture) = capture_frame.data::<FrameData>().and_then(|d| d.capture.upgrade()) else { return };
        let mut guard = capture.session.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        state.in_flight = false;
        state.consecutive_failures = 0;
        let Some(buffers) = state.buffers.as_mut() else { return };

        // Back buffer now holds the newest frame: make it the front.
        buffers.rotate_left(1);
        buffers[0].damage.clear();
        for buffer in &mut buffers[1..] {
            buffer.damage.extend_from_slice(&frame.damage);
        }
        let transform = match frame.transform {
            WEnum::Value(t) => t,
            WEnum::Unknown(_) => cctk::wayland_client::protocol::wl_output::Transform::Normal,
        };

        // GL pass: the front capture buffer → a thumbnail-sized, corner-masked
        // target. Falls back to the raw frame if the pass is unavailable, the
        // UI hasn't told us a size yet, or this frame's render failed.
        let thumb_size = self.thumb_sizes.get(&capture.handle).copied();
        let front_size = buffers[0].size;
        let mut processed: Option<(Arc<BufferSource>, (u32, u32))> = None;
        let mut gl_error = None;
        if let Some(size) = thumb_size {
            let ScreencopySession { buffers: bufs, thumb, .. } = state;
            let front = &mut bufs.as_mut().unwrap()[0];
            match self.gl_process(front, thumb, size, transform) {
                Ok(backing) => processed = Some((backing, size)),
                Err(err) => gl_error = Some(err),
            }
        }
        let Some(buffers) = state.buffers.as_mut() else { return };

        let was_processed = processed.is_some();
        let (release, image) = match processed {
            Some((backing, (w, h))) => {
                let (sb, release) = SubsurfaceBuffer::new(backing);
                let image = CaptureImage {
                    buffer: sb,
                    width: w,
                    height: h,
                    transform: cctk::wayland_client::protocol::wl_output::Transform::Normal,
                };
                (release, image)
            }
            None => {
                let (sb, release) = SubsurfaceBuffer::new(buffers[0].backing.clone());
                let image = CaptureImage { buffer: sb, width: front_size.0, height: front_size.1, transform };
                (release, image)
            }
        };
        // What the next submit must wait for: the buffer the compositor is
        // now holding — the GL target if we rendered, else the raw front.
        // A processed frame also takes any pending raw release: the raw
        // buffer shipped last time (before the pass kicked in) is the one
        // the next capture writes into, so it must be released first.
        let (previous_release, stale_raw_release) = if was_processed {
            let pool = state.thumb.as_mut().unwrap();
            (pool.release.replace(release), state.release.take())
        } else {
            (state.release.replace(release), None)
        };
        let last_submit = state.last_submit;
        let session_id = state.session.clone();

        // Next capture: after the previous front buffer is released by the
        // compositor and at least one frame interval since the last submit.
        // The wait is computed *after* the release resolves, since waiting
        // for release can itself take longer than the frame interval.
        let capture_for_task = capture.clone();
        let conn = conn.clone();
        let qh = qh.clone();
        let fps = self.fps.clone();
        self.thread_pool.spawn_ok(async move {
            if let Some(release) = previous_release {
                release.await;
            }
            if let Some(release) = stale_raw_release {
                release.await;
            }
            let wait = frame_interval(&fps).saturating_sub(last_submit.elapsed());
            if !wait.is_zero() {
                futures_timer::Delay::new(wait).await;
            }
            let mut guard = capture_for_task.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                // Belt and braces: `in_flight` is the primary guard against a
                // stale task racing a restarted session on the same `Arc`,
                // but also bail if this task no longer targets the session
                // it was spawned for.
                if state.session != session_id {
                    return;
                }
                state.submit(&capture_for_task, &conn, &qh);
            }
        });

        drop(guard);
        if let Some(err) = gl_error {
            self.gl_failed(err);
        }
        self.send_event(Event::Frame(capture.handle.clone(), image));
    }

    fn failed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, capture_frame: &CaptureFrame, reason: WEnum<FailureReason>) {
        let Some(capture) = capture_frame.data::<FrameData>().and_then(|d| d.capture.upgrade()) else { return };
        {
            let mut guard = capture.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                state.in_flight = false;
            }
        }
        match reason {
            WEnum::Value(FailureReason::BufferConstraints) => {
                tracing::info!("buffer constraints changed; reallocating");
                let formats = capture.session.lock().unwrap().as_ref().and_then(|s| s.formats.clone());
                let Some(formats) = formats else { return };
                let buffers = self.allocate(&formats);
                let unavailable = buffers.is_none();
                let mut guard = capture.session.lock().unwrap();
                if let Some(state) = guard.as_mut() {
                    state.buffers = buffers;
                    state.release = None;
                    state.submit(&capture, conn, qh);
                }
                drop(guard);

                if unavailable {
                    // Same as `init_done`: no buffers means this session is
                    // dead; stop it so a later state change starts fresh.
                    capture.stop();
                    self.send_event(Event::CaptureUnavailable(capture.handle.clone()));
                }
            }
            WEnum::Value(FailureReason::Stopped) => {
                tracing::info!("capture stopped by compositor");
                capture.stop();
            }
            other => {
                let mut guard = capture.session.lock().unwrap();
                let Some(state) = guard.as_mut() else { return };
                state.consecutive_failures += 1;
                let n = state.consecutive_failures;
                let session_id = state.session.clone();
                drop(guard);

                if n >= 5 {
                    tracing::warn!(
                        "capture failed {n} times in a row ({other:?}); giving up until the client changes state"
                    );
                    capture.stop();
                    self.send_event(Event::CaptureUnavailable(capture.handle.clone()));
                    return;
                }

                let delay_ms = retry_delay_ms(n);
                tracing::debug!("capture failed: {other:?}; retrying in {delay_ms}ms (attempt {n})");
                let delay = Duration::from_millis(delay_ms);
                let capture_for_task = capture.clone();
                let conn = conn.clone();
                let qh = qh.clone();
                self.thread_pool.spawn_ok(async move {
                    futures_timer::Delay::new(delay).await;
                    let mut guard = capture_for_task.session.lock().unwrap();
                    if let Some(state) = guard.as_mut() {
                        if state.session != session_id {
                            return;
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_ms_schedule() {
        assert_eq!(retry_delay_ms(1), 250);
        assert_eq!(retry_delay_ms(2), 500);
        assert_eq!(retry_delay_ms(3), 1000);
        assert_eq!(retry_delay_ms(4), 2000);
        assert_eq!(retry_delay_ms(5), 2000);
        assert_eq!(retry_delay_ms(6), 2000);
    }
}
