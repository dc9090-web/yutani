//! One capture buffer: a gbm dmabuf (zero-copy) or, if that fails, wl_shm.
//! `backing` is shared with the UI so the Subsurface widget reuses its
//! wl_buffer instead of re-importing the fds every frame.

use cosmic::cctk::{
    screencopy::{Formats, Rect},
    wayland_client::{
        Connection, Dispatch, QueueHandle,
        protocol::{wl_buffer, wl_shm, wl_shm_pool},
    },
    wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1,
};
use cosmic::iced::platform_specific::shell::subsurface_widget::{BufferSource, Dmabuf, Plane, Shmbuf};
use std::os::fd::AsFd;
use std::sync::Arc;

use super::AppData;

pub struct Buffer {
    pub backing: Arc<BufferSource>,
    pub buffer: wl_buffer::WlBuffer,
    pub damage: Vec<Rect>,
    pub size: (u32, u32),
    /// EGL import of `backing`, created on first use by the GL pass and
    /// kept for the buffer's lifetime (imports are per buffer, not per frame).
    pub source: Option<super::gl::SourceTexture>,
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

fn full_damage((width, height): (u32, u32)) -> Vec<Rect> {
    vec![Rect { x: 0, y: 0, width: width as i32, height: height as i32 }]
}

impl AppData {
    fn create_gbm_buffer(
        &mut self,
        format: u32,
        modifiers: &[u64],
        (width, height): (u32, u32),
        drm_dev: Option<u64>,
    ) -> anyhow::Result<Option<Buffer>> {
        let Some(feedback) = self.dmabuf_feedback.as_ref() else {
            return Ok(None);
        };
        let drm_dev = drm_dev.unwrap_or(feedback.main_device());
        let Some((_path, gbm)) = self.gbm_devices.gbm_device(drm_dev)? else {
            return Ok(None);
        };

        let modifiers: Vec<gbm::Modifier> = modifiers.iter().map(|m| gbm::Modifier::from(*m)).collect();
        if modifiers.is_empty() {
            return Ok(None);
        }
        let gbm_format = gbm::Format::try_from(format)?;
        let bo = if modifiers.iter().all(|m| *m == gbm::Modifier::Invalid) {
            gbm.create_buffer_object::<()>(width, height, gbm_format, gbm::BufferObjectFlags::empty())?
        } else {
            gbm.create_buffer_object_with_modifiers::<()>(width, height, gbm_format, modifiers.iter().copied())?
        };

        let params = self.dmabuf_state.create_params(&self.qh)?;
        let modifier = bo.modifier();
        let mut planes = Vec::new();
        for i in 0..bo.plane_count() as i32 {
            let fd = bo.fd_for_plane(i)?;
            let offset = bo.offset(i);
            let stride = bo.stride_for_plane(i);
            params.add(fd.as_fd(), i as u32, offset, stride, modifier.into());
            planes.push(Plane { fd, plane_idx: i as u32, offset, stride });
        }
        // `create_immed` makes a rejected dmabuf a fatal protocol error
        // rather than a `failed` event, and that is deliberate: the format
        // and modifier come from the compositor's own per-session formats
        // and gbm chose the modifier from that list, so a rejection is a
        // compositor bug. Switching to `create` would also need the
        // `DmabufHandler::created/failed` stubs to become real.
        let (buffer, _) = params.create_immed(
            width as i32,
            height as i32,
            format,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &self.qh,
        );

        Ok(Some(Buffer {
            backing: Arc::new(
                Dmabuf { width: width as i32, height: height as i32, planes, format, modifier: modifier.into() }.into(),
            ),
            buffer,
            damage: full_damage((width, height)),
            size: (width, height),
            source: None,
        }))
    }

    fn create_shm_buffer(&self, format: wl_shm::Format, (width, height): (u32, u32)) -> anyhow::Result<Buffer> {
        let stride = width as i32 * 4;
        let len = stride as usize * height as usize;
        let fd = memfd(len)?;
        let pool = self.shm_state.wl_shm().create_pool(fd.as_fd(), len as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, width as i32, height as i32, stride, format, &self.qh, ());
        pool.destroy();
        Ok(Buffer {
            backing: Arc::new(
                Shmbuf { fd, offset: 0, width: width as i32, height: height as i32, stride, format }.into(),
            ),
            buffer,
            damage: full_damage((width, height)),
            size: (width, height),
            source: None,
        })
    }

    /// gbm dmabuf if the compositor advertises one for ABGR8888, else shm.
    pub fn create_buffer(&mut self, formats: &Formats) -> anyhow::Result<Buffer> {
        ensure_nonzero_size(formats.buffer_size)?;
        let format = wl_shm::Format::Abgr8888;
        if let Some((_, modifiers)) = formats.dmabuf_formats.iter().find(|(f, _)| *f == u32::from(format)) {
            match self.create_gbm_buffer(u32::from(format), modifiers, formats.buffer_size, formats.dmabuf_device) {
                Ok(Some(buffer)) => return Ok(buffer),
                Ok(None) => tracing::warn!("no usable gbm device; falling back to shm"),
                Err(err) => tracing::warn!("gbm buffer failed: {err}; falling back to shm"),
            }
        }
        anyhow::ensure!(formats.shm_formats.contains(&format), "compositor offers neither dmabuf nor shm ABGR8888");
        self.create_shm_buffer(format, formats.buffer_size)
    }
}

/// A 0×0 size would reach `wl_shm.create_pool(size = 0)` (the gbm path
/// already rejects it and falls through to shm), a fatal protocol error
/// that takes the whole backend down; the caller turns this `Err` into
/// `CaptureUnavailable` instead.
fn ensure_nonzero_size((width, height): (u32, u32)) -> anyhow::Result<()> {
    anyhow::ensure!(width > 0 && height > 0, "zero-sized capture ({width}x{height})");
    Ok(())
}

fn memfd(len: usize) -> anyhow::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let name = c"yutani-shm";
    // SAFETY: memfd_create with a valid C string and flags; result checked below.
    let fd = unsafe { libc_memfd_create(name.as_ptr(), 1 /* MFD_CLOEXEC */) };
    anyhow::ensure!(fd >= 0, "memfd_create failed: {}", std::io::Error::last_os_error());
    // SAFETY: fd is a fresh, owned descriptor.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let file = std::fs::File::from(fd.try_clone()?);
    file.set_len(len as u64)?;
    Ok(fd)
}

unsafe extern "C" {
    #[link_name = "memfd_create"]
    fn libc_memfd_create(name: *const std::ffi::c_char, flags: std::ffi::c_uint) -> std::ffi::c_int;
}

impl Dispatch<wl_buffer::WlBuffer, ()> for AppData {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for AppData {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_sized_capture_is_refused_before_any_pool_is_created() {
        assert!(ensure_nonzero_size((0, 0)).is_err());
        assert!(ensure_nonzero_size((640, 0)).is_err());
        assert!(ensure_nonzero_size((0, 480)).is_err());
        assert!(ensure_nonzero_size((640, 480)).is_ok());
    }
}
