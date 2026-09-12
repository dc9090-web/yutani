//! libcosmic application: no main window. One overlay layer surface per
//! shown EVE client, each showing that client's live captured frame.
//! Floating mode: the user places them (drag/pin/persist). Dock mode: the
//! same surfaces, auto-arranged and centred along `dock_edge` (see `dock`).

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
use crate::model::config::{Config, Mode};
use crate::model::layout::{self, Layout, Rect, ThumbPos};

pub mod config_watch;
pub mod dock;
pub mod ipc;
pub mod pointer;
pub mod rules;
pub mod thumbnail;
pub mod tray;

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
    pub scale: i32,
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
    /// Last size sent via `set_size` (or the surface's creation size), so a
    /// resize to the same size can be skipped.
    pub last_size: Option<(u32, u32)>,
    /// Backend capture is paused for this client (last command we sent).
    /// Pause/resume is an edge on this, not on whether a surface exists, so
    /// a client born hidden is paused too.
    pub paused: bool,
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
    /// Tray/IPC-toggled visibility: when true, no thumbnail is shown
    /// regardless of `Visibility`/`hide_active`. Only `set_hidden` writes it.
    pub hidden: bool,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Wayland(WaylandEvent),
    Backend(Event),
    Pointer(SurfaceId, mouse::Event),
    ConfigChanged(Config),
    Tray(tray::TrayEvent),
    Ipc(ipc::IpcEvent),
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

    /// Integer scale of the output `client` is (or would be) shown on.
    fn scale_for(&self, client: &Client) -> i32 {
        self.output_for(&client.info)
            .and_then(|handle| self.outputs.iter().find(|o| o.handle == handle))
            .map_or(1, |o| o.scale)
    }

    /// Tell the backend the physical size to render this client's frames at
    /// and the mask radius to use, both scaled by the client's own output
    /// (outputs may differ in scale, so the radius is per client too).
    /// Nothing is sent before the backend has handed over its channel; the
    /// `CmdSender` handler replays every sized client then.
    fn send_thumb_size(&self, handle: &Handle, logical: (u32, u32)) {
        if self.cmd.is_none() {
            return;
        }
        let Some(client) = self.clients.get(handle) else { return };
        let s = self.scale_for(client) as u32;
        let physical = (logical.0 * s, logical.1 * s);
        let radius = self.config.corner_radius * s;
        tracing::debug!(?handle, ?logical, scale = s, ?physical, radius, "thumb size");
        self.send(Cmd::SetThumbSize(handle.clone(), physical, radius));
    }

    /// Re-send `send_thumb_size` for every client that has a surface (its
    /// `last_size` is the logical size it was created or resized at). Used
    /// when something that feeds the command changes globally: the channel
    /// arriving, or the configured corner radius.
    fn resend_thumb_sizes(&self) {
        for (h, c) in &self.clients {
            if let Some(size) = c.last_size.filter(|_| c.surface.is_some()) {
                self.send_thumb_size(h, size);
            }
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
                let scale = info.scale_factor.max(1);
                if let Some(existing) = self.outputs.iter_mut().find(|o| o.handle == output) {
                    existing.name = name;
                    existing.logical_size = logical_size;
                    existing.scale = scale;
                } else {
                    tracing::info!(%name, ?logical_size, scale, "output");
                    self.outputs.push(Output { handle: output, name, logical_size, scale });
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
        rules::should_show(
            self.config.visibility,
            self.config.hide_active,
            self.hidden,
            self.any_client_activated(),
            client.info.activated,
        )
    }

    /// Bring the backend's capture state for `h` in line with `show`: pause
    /// when it should not be shown but is running, resume when it should be
    /// but is paused. An edge on `Client::paused` (what we last told the
    /// backend), so a client that is born hidden gets paused, and a client
    /// that merely moves between floating and dock gets nothing.
    fn sync_capture(&mut self, h: &Handle, show: bool) {
        let Some(c) = self.clients.get(h) else { return };
        if let Some(pause) = rules::capture_transition(show, c.paused) {
            self.send(if pause { Cmd::PauseCapture(h.clone()) } else { Cmd::ResumeCapture(h.clone()) });
            self.clients.get_mut(h).unwrap().paused = pause;
        }
    }

    /// Make what is on screen match `should_show`: every shown client has a
    /// surface, no other client does. Capture is paused for a client when it
    /// should not be shown and resumed when it should (see `sync_capture`).
    /// In dock mode the surviving surfaces are then re-laid out along the
    /// edge, since the set on each output may have changed.
    fn reconcile_surfaces(&mut self) -> Task<cosmic::Action<Msg>> {
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        let mut tasks = Vec::new();
        for h in handles {
            let show = self.should_show(&self.clients[&h]);
            self.sync_capture(&h, show);
            if show && self.clients[&h].surface.is_none() {
                tasks.push(self.create_surface(&h));
            } else if !show && self.clients[&h].surface.is_some() {
                tasks.push(self.destroy_surface(&h));
            }
        }
        tasks.push(self.relayout_dock());
        Task::batch(tasks)
    }

    /// Layout order of every known client (spec §7).
    fn focus_order(&self) -> Vec<Handle> {
        let items = self
            .clients
            .iter()
            .map(|(h, c)| rules::FocusItem {
                handle: h.clone(),
                label: c.info.login.label().to_string(),
                output: self
                    .output_for(&c.info)
                    .and_then(|o| self.outputs.iter().find(|k| k.handle == o).map(|k| k.name.clone()))
                    .unwrap_or_default(),
                position: c.position,
            })
            .collect();
        rules::focus_order(self.config.mode, items)
    }

    fn active_client(&self) -> Option<Handle> {
        self.clients.iter().find(|(_, c)| c.info.activated).map(|(h, _)| h.clone())
    }

    /// Execute one IPC request. `Err` is the text sent back after `err `.
    fn handle_request(&mut self, request: &crate::ipc::Request) -> (Result<(), String>, Task<cosmic::Action<Msg>>) {
        use crate::ipc::Request;
        match request {
            Request::Focus(n) => {
                let order = self.focus_order();
                match n.checked_sub(1).and_then(|i| order.get(i)) {
                    Some(h) => {
                        self.send(Cmd::Activate(h.clone()));
                        (Ok(()), Task::none())
                    }
                    None => (Err(format!("no client {n} ({} known)", order.len())), Task::none()),
                }
            }
            Request::Next | Request::Prev => {
                let order = self.focus_order();
                let active = self.active_client();
                match rules::step(&order, active.as_ref(), matches!(request, Request::Next)) {
                    Some(h) => {
                        self.send(Cmd::Activate(h));
                        (Ok(()), Task::none())
                    }
                    None => (Err("no clients".into()), Task::none()),
                }
            }
            Request::Show => (Ok(()), self.set_hidden(false)),
            Request::Hide => (Ok(()), self.set_hidden(true)),
            Request::Toggle => {
                let h = !self.hidden;
                (Ok(()), self.set_hidden(h))
            }
            Request::Layout(_) | Request::Settings => {
                (Err("not supported yet (settings and layouts arrive in plan 5)".into()), Task::none())
            }
            Request::Quit => {
                ipc::remove_socket();
                (Ok(()), cosmic::iced::exit())
            }
        }
    }

    /// The one place `hidden` changes (tray and IPC both come through here).
    fn set_hidden(&mut self, hidden: bool) -> Task<cosmic::Action<Msg>> {
        if self.hidden == hidden {
            return Task::none();
        }
        self.hidden = hidden;
        self.reconcile_surfaces()
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

    /// Floating position for `handle`: this client's own previous placement
    /// (destroy+recreate cycles like hide_active, EveFocusedOnly, or output
    /// loss should put the thumbnail back where it was even if never
    /// persisted); otherwise a saved position for this character; otherwise
    /// the next free slot.
    fn floating_position(&self, handle: &Handle) -> ((i32, i32), bool) {
        let client = &self.clients[handle];
        let saved = match &client.info.login {
            Login::LoggedIn(name) => self.layout.thumbs.get(name).cloned(),
            Login::LoggingIn => None,
        };
        rules::choose_position(
            client.placed.then_some((client.position, client.pinned)),
            saved.as_ref(),
            self.next_position(),
        )
    }

    fn create_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        if client.surface.is_some() || !self.should_show(client) {
            return Task::none();
        }
        let Some(output) = self.output_for(&client.info) else {
            tracing::debug!("no outputs yet; deferring surface");
            return Task::none();
        };
        let (width, height) = self.surface_size(client);
        let id = SurfaceId::unique();
        let position = match self.config.mode {
            Mode::Floating => {
                let (position, pinned) = self.floating_position(handle);
                let client = self.clients.get_mut(handle).unwrap();
                client.pinned = pinned;
                client.placed = true;
                position
            }
            Mode::Dock => {
                // Register the surface first so the layout counts this
                // client among its output's docked ones. `placed` stays
                // false: dock coordinates must never be reused as floating
                // ones.
                self.clients.get_mut(handle).unwrap().surface = Some(id);
                self.dock_position_of(handle)
            }
        };
        let client = self.clients.get_mut(handle).unwrap();
        client.surface = Some(id);
        client.position = position;
        client.last_size = Some((width, height));
        tracing::info!(?id, x = position.0, y = position.1, width, height, "create_surface");
        self.send_thumb_size(handle, (width, height));
        let create = get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: IcedOutput::Output(output),
            namespace: "yutani".into(),
            margin: IcedMargin { top: position.1, left: position.0, ..Default::default() },
            size: Some((Some(width), Some(height))),
            exclusive_zone: 0,
            // This pinned iced ignores size_limits for layer surfaces; NONE is harmless.
            size_limits: Limits::NONE,
            ..Default::default()
        });
        create
    }

    fn destroy_surface(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        match self.forget_surface(handle) {
            Some(id) => {
                tracing::info!(?id, "destroy_surface");
                destroy_layer_surface(id)
            }
            None => Task::none(),
        }
    }

    /// Clear `client.surface` (if set) along with any hover/drag state tied
    /// to it, returning the surface id that was there. A surface that goes
    /// away — destroyed by us or closed by the compositor — must not leave
    /// `hovered` set, or the next `create_surface` for this client comes back
    /// zoomed; and a drag on it must not linger and block future drags.
    fn forget_surface(&mut self, handle: &Handle) -> Option<SurfaceId> {
        let id = self.clients.get_mut(handle).and_then(|c| c.surface.take())?;
        if self.drag.as_ref().is_some_and(|d| d.surface == id) {
            self.drag = None;
        }
        if let Some(c) = self.clients.get_mut(handle) {
            c.hovered = false;
            c.last_cursor = Point::ORIGIN;
        }
        Some(id)
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

    /// Remember this client's position (and pin state) under its character
    /// name. Floating only: dock positions come from the layout.
    fn persist_position(&mut self, handle: &Handle) {
        if self.config.mode == Mode::Dock {
            return;
        }
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

    /// If we have a saved position for this character, move there. Floating
    /// only: in dock mode the saved spot applies when (if) the mode changes.
    fn apply_saved_position(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        if self.config.mode == Mode::Dock {
            return Task::none();
        }
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Login::LoggedIn(name) = &client.info.login else { return Task::none() };
        let Some(saved) = self.layout.thumbs.get(name).cloned() else { return Task::none() };
        let client = self.clients.get_mut(handle).unwrap();
        client.position = (saved.x, saved.y);
        client.pinned = saved.pinned;
        let surface = client.surface;
        match surface {
            // A drag canvas surface must keep its enlarged size until the
            // drag ends; a margin here would shrink it out from under the drag.
            Some(id) if !self.in_canvas(id) => set_margin(id, saved.y, 0, 0, saved.x),
            _ => Task::none(),
        }
    }

    /// Leaving dock mode: every surface goes back to a floating spot — the
    /// character's saved position, else the next free slot. `placed` was
    /// cleared on entering dock mode, so dock coordinates are never reused.
    /// Handles are visited in dock order so slot assignment is deterministic.
    fn refloat_surfaces(&mut self) -> Task<cosmic::Action<Msg>> {
        let with_surface = self.clients.iter().filter(|(_, c)| c.surface.is_some());
        let handles = rules::dock_order(with_surface.map(|(h, c)| (h, c.info.login.label())));
        let mut tasks = Vec::new();
        for h in handles {
            let (position, pinned) = self.floating_position(&h);
            let c = self.clients.get_mut(&h).unwrap();
            c.position = position;
            c.pinned = pinned;
            c.placed = true;
            let id = c.surface.unwrap();
            tracing::info!(?id, x = position.0, y = position.1, "leaving dock: floating position");
            if !self.in_canvas(id) {
                tasks.push(set_margin(id, position.1, 0, 0, position.0));
            }
        }
        Task::batch(tasks)
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
    fn leave_canvas(&mut self, id: SurfaceId, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        // `client.hovered` is set true before this is called (the cursor is
        // over the thumbnail at drag end), so this restores at zoomed size.
        let (w, h) = self.surface_size(client);
        let (x, y) = client.position;
        self.clients.get_mut(handle).unwrap().last_size = Some((w, h));
        self.send_thumb_size(handle, (w, h));
        Task::batch([
            set_anchor(id, Anchor::TOP | Anchor::LEFT),
            set_size(id, Some(w), Some(h)),
            set_margin(id, y, 0, 0, x),
        ])
    }

    /// True while `id` is enlarged to a drag canvas: a drag on this surface
    /// has crossed the arming threshold (`phase != Idle`) and is not pinned.
    fn in_canvas(&self, id: SurfaceId) -> bool {
        self.drag.as_ref().is_some_and(|d| d.surface == id && !d.pinned && d.phase != pointer::Phase::Idle)
    }

    /// Resize `handle`'s surface to its current target size, but only if it
    /// isn't already that size and it isn't a full-output drag canvas.
    fn resize_if_needed(&mut self, handle: &Handle) -> Task<cosmic::Action<Msg>> {
        let Some(client) = self.clients.get(handle) else { return Task::none() };
        let Some(id) = client.surface else { return Task::none() };
        if self.in_canvas(id) {
            return Task::none();
        }
        let size = self.surface_size(client);
        if client.last_size == Some(size) {
            return Task::none();
        }
        self.clients.get_mut(handle).unwrap().last_size = Some(size);
        self.send_thumb_size(handle, size);
        set_size(id, Some(size.0), Some(size.1))
    }

    fn on_pointer(&mut self, id: SurfaceId, event: mouse::Event) -> Task<cosmic::Action<Msg>> {
        let Some(handle) = self.client_for_surface(id) else { return Task::none() };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Middle) => {
                // No pins in dock mode: positions come from the layout.
                if self.config.mode == Mode::Dock {
                    return Task::none();
                }
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
                // Dock mode: no dragging, but a release must still be a
                // click — a pinned `DragState` never leaves `Idle` and yields
                // `Click` on release.
                let pinned = c.pinned || self.config.mode == Mode::Dock;
                self.drag = pointer::on_press(id, button, cursor, c.position, pinned);
                // Idle: the canvas isn't entered until motion crosses the
                // threshold (see CursorMoved), so a plain click never
                // touches the surface at all.
                Task::none()
            }
            mouse::Event::CursorMoved { position } => {
                if let Some(c) = self.clients.get_mut(&handle) {
                    c.last_cursor = position;
                }
                let Some(drag) = self.drag.as_mut().filter(|d| d.surface == id) else { return Task::none() };
                match drag.phase {
                    pointer::Phase::Idle => match pointer::on_move_local(drag, position) {
                        pointer::Outcome::StartDrag => {
                            tracing::debug!(?id, "drag: threshold crossed; arming canvas");
                            Self::enter_canvas(id)
                        }
                        _ => Task::none(),
                    },
                    pointer::Phase::Arming => Task::none(),
                    pointer::Phase::Dragging => match pointer::on_move(drag, position) {
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
                    },
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if !self.drag.as_ref().is_some_and(|d| d.surface == id && d.button == button) {
                    return Task::none();
                }
                let drag = self.drag.take().unwrap();
                // The canvas is only entered once motion crosses the threshold
                // (`drag.moved`); if it never did, the surface never changed
                // and there's nothing to restore.
                let entered_canvas = !drag.pinned && drag.moved;
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
                if entered_canvas {
                    // The cursor is over the thumbnail at drag end (no CursorEntered
                    // fires for a surface that was already under the pointer), so mark
                    // it hovered before computing the restore size — leave_canvas then
                    // restores at zoomed size instead of snapping small first.
                    self.clients.get_mut(&handle).unwrap().hovered = true;
                    tracing::debug!(?id, pos = ?self.clients[&handle].position, "drag: leaving canvas");
                    self.leave_canvas(id, &handle)
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
                self.clients.get_mut(&handle).unwrap().hovered = true;
                // Dock mode: a zoomed thumbnail shifts its neighbours.
                Task::batch([self.resize_if_needed(&handle), self.relayout_dock()])
            }
            mouse::Event::CursorLeft => {
                // Releasing outside is delivered to us anyway (implicit grab); nothing
                // to do while dragging — canvas mode owns the size until release.
                if self.drag.as_ref().is_some_and(|d| d.surface == id) {
                    return Task::none();
                }
                self.clients.get_mut(&handle).unwrap().hovered = false;
                Task::batch([self.resize_if_needed(&handle), self.relayout_dock()])
            }
            _ => Task::none(),
        }
    }

    fn on_backend(&mut self, event: Event) -> Task<cosmic::Action<Msg>> {
        match event {
            Event::CmdSender(sender) => {
                self.cmd = Some(sender);
                // Clients (and so surfaces) come from backend events that
                // follow this one, so nothing should be pending; replaying is
                // cheap and removes the ordering hazard should a surface ever
                // be sized before the channel exists (its send was dropped).
                self.resend_thumb_sizes();
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
                    last_size: None,
                    paused: false,
                });
                let was_named = matches!(entry.info.login, Login::LoggedIn(_));
                entry.info = info;
                let became_named = !was_named && matches!(entry.info.login, Login::LoggedIn(_));
                // An activation change on one client can hide/show others, so
                // reconcile every client's surface, not just this one's.
                let reconciled = self.reconcile_surfaces();
                if became_named {
                    // A saved position must apply even if the surface was just
                    // created (or doesn't exist yet): `apply_saved_position`
                    // updates position/pinned regardless, and is a no-op on
                    // margin if there's no surface.
                    Task::batch([reconciled, self.apply_saved_position(&handle)])
                } else {
                    reconciled
                }
            }
            Event::ClientRemoved(handle) => {
                let task = self.destroy_surface(&handle);
                self.clients.remove(&handle);
                // Dock mode: its neighbours close the gap.
                Task::batch([task, self.relayout_dock()])
            }
            Event::Frame(handle, image) => {
                let Some(client) = self.clients.get_mut(&handle) else { return Task::none() };
                client.image = Some(image);
                client.unavailable = false;
                let task = if client.surface.is_some() {
                    // `resize_if_needed` skips a drag canvas and dedupes
                    // against the last size actually sent.
                    self.resize_if_needed(&handle)
                } else {
                    self.create_surface(&handle)
                };
                // Dock mode: a first frame (or a new aspect) changes this
                // thumbnail's size, which moves its neighbours.
                Task::batch([task, self.relayout_dock()])
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

    /// Apply a freshly re-read (and validated) config. Border colours and
    /// names apply on the next redraw automatically because
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
        if new.corner_radius != self.config.corner_radius {
            self.config.corner_radius = new.corner_radius;
            self.resend_thumb_sizes();
        }
        let mode_changed = new.mode != self.config.mode;
        self.config = new;
        let mut tasks = Vec::new();
        if mode_changed {
            match self.config.mode {
                // Positions now come from the layout; forget the floating
                // ones so a later switch back doesn't reuse dock coordinates
                // (`refloat_surfaces` falls back to saved/next-slot instead).
                Mode::Dock => {
                    self.clients.values_mut().for_each(|c| c.placed = false);
                    // A button held down right now must not turn into a drag
                    // (a canvas already entered is left alone: its release
                    // path restores the surface at the dock position).
                    if let Some(d) = self.drag.as_mut().filter(|d| d.phase == pointer::Phase::Idle) {
                        d.pinned = true;
                    }
                }
                Mode::Floating => tasks.push(self.refloat_surfaces()),
            }
        }
        // Sizes and visibility may have changed.
        tasks.push(self.reconcile_surfaces());
        let handles: Vec<Handle> = self.clients.keys().cloned().collect();
        for h in handles {
            // `resize_if_needed` skips a drag canvas (must keep its size
            // until the drag ends — `leave_canvas` applies the current size
            // then) and dedupes against the last size actually sent.
            tasks.push(self.resize_if_needed(&h));
        }
        // Dock mode: `reconcile_surfaces` already re-laid out every surface
        // at its new size (the layout reads `surface_size`, not `last_size`)
        // and along the new edge.
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
            hidden: false,
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Msg) -> Task<cosmic::Action<Msg>> {
        match message {
            Msg::Wayland(WaylandEvent::Output(event, output)) => {
                // The first output lets deferred surfaces be created; a
                // removal does not itself recreate anything (the compositor
                // sends Layer(Done, ..) for each surface on that output, and
                // that handler recreates them via output_for); a size change
                // moves the dock layout. `reconcile_surfaces` covers all.
                self.on_output(event, output);
                // A scale change is picked up at the next size send.
                self.reconcile_surfaces()
            }
            Msg::Wayland(WaylandEvent::Layer(LayerEvent::Done, _, id)) => {
                // The compositor closed this surface (its output went away).
                if let Some(handle) = self.client_for_surface(id) {
                    tracing::info!("layer surface closed by compositor; recreating");
                    self.forget_surface(&handle);
                    return self.reconcile_surfaces();
                }
                Task::none()
            }
            Msg::Wayland(WaylandEvent::Layer(..)) => Task::none(),
            Msg::Wayland(_) => Task::none(),
            Msg::Backend(event) => self.on_backend(event),
            Msg::Pointer(id, event) => self.on_pointer(id, event),
            Msg::ConfigChanged(config) => self.apply_config(config),
            Msg::Tray(tray::TrayEvent::ToggleVisibility) => {
                let h = !self.hidden;
                self.set_hidden(h)
            }
            Msg::Tray(tray::TrayEvent::SetHidden(h)) => self.set_hidden(h),
            Msg::Tray(tray::TrayEvent::Quit) => {
                ipc::remove_socket();
                cosmic::iced::exit()
            }
            Msg::Ipc(ev) => {
                let (result, task) = self.handle_request(&ev.request);
                ev.reply.respond(match result {
                    Ok(()) => crate::ipc::Response::Ok,
                    Err(m) => crate::ipc::Response::Err(m),
                });
                task
            }
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
        let mut subs = vec![
            events,
            config_watch::subscription().map(Msg::ConfigChanged),
            tray::subscription().map(Msg::Tray),
            ipc::subscription().map(Msg::Ipc),
        ];
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
    /// process. In practice this never fires: the second-instance case is
    /// already handled before `run_single_instance` runs, by the socket
    /// check in `main.rs` (`cli::is_running`). This activation is a no-op.
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
