//! linux-dmabuf handler: we only need the default feedback (main device).

use cosmic::cctk::{
    self,
    sctk::dmabuf::{DmabufFeedback, DmabufHandler, DmabufState},
    wayland_client::{Connection, QueueHandle, protocol::wl_buffer},
    wayland_protocols::wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
    },
};

use super::AppData;

impl DmabufHandler for AppData {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_feedback(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpLinuxDmabufFeedbackV1,
        feedback: DmabufFeedback,
    ) {
        self.dmabuf_feedback = Some(feedback);
    }

    fn created(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpLinuxBufferParamsV1, _: wl_buffer::WlBuffer) {}

    fn failed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpLinuxBufferParamsV1) {}

    fn released(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_buffer::WlBuffer) {}
}

cctk::sctk::delegate_dmabuf!(AppData);
