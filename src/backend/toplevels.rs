//! Toplevel list → `Event::Client*`; toplevel manager capabilities.

use cosmic::cctk::{
    self,
    cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    wayland_client::{Connection, QueueHandle, WEnum},
};
use std::collections::HashSet;

use super::{AppData, Event, Handle};

impl AppData {
    /// Re-run classification for every known toplevel (after `SetAppIds`).
    pub fn reclassify_all(&mut self) {
        let infos: Vec<_> = self
            .toplevel_info_state
            .toplevels()
            .map(|info| (info.foreign_toplevel.clone(), info.clone()))
            .collect();
        for (handle, info) in infos {
            match self.client_info(&info) {
                Some(client) => {
                    self.start_capture(&handle);
                    self.send_event(Event::ClientAdded(handle.clone(), client));
                }
                None => {
                    // Only a previously-classified client transitions to
                    // removed; a toplevel that was never a client (e.g. it
                    // never matched `app_ids`) must not spuriously emit
                    // `ClientRemoved`.
                    if self.captures.contains_key(&handle) {
                        self.stop_capture(&handle);
                        self.send_event(Event::ClientRemoved(handle));
                    }
                }
            }
        }
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, handle: &Handle) {
        let Some(info) = self.toplevel_info_state.info(handle).cloned() else { return };
        if let Some(client) = self.client_info(&info) {
            tracing::info!(title = %info.title, "client added");
            self.send_event(Event::ClientAdded(handle.clone(), client));
            self.start_capture(handle);
        }
    }

    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, handle: &Handle) {
        let Some(info) = self.toplevel_info_state.info(handle).cloned() else { return };
        match self.client_info(&info) {
            Some(client) => {
                tracing::debug!(title = %info.title, activated = client.activated, "client updated");
                self.send_event(Event::ClientUpdated(handle.clone(), client));
                self.start_capture(handle);
            }
            None => {
                // See `reclassify_all`: only gate `ClientRemoved` behind a
                // toplevel that was previously a known client.
                if self.captures.contains_key(handle) {
                    self.stop_capture(handle);
                    self.send_event(Event::ClientRemoved(handle.clone()));
                }
            }
        }
    }

    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, handle: &Handle) {
        tracing::info!("toplevel closed");
        self.stop_capture(handle);
        self.send_event(Event::ClientRemoved(handle.clone()));
    }
}

impl ToplevelManagerHandler for AppData {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        // `None` is unreachable here: the delegate only dispatches manager
        // events for a manager it was bound with in the first place.
        self.toplevel_manager_state.as_mut().expect("toplevel manager")
    }

    fn capabilities(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        capabilities: Vec<
            WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>,
        >,
    ) {
        let caps: HashSet<_> = capabilities
            .into_iter()
            .filter_map(|c| match c {
                WEnum::Value(v) => Some(v),
                WEnum::Unknown(_) => None,
            })
            .collect();
        tracing::info!(?caps, "toplevel manager capabilities");
    }
}

cctk::delegate_toplevel_info!(AppData);
cctk::delegate_toplevel_manager!(AppData);
