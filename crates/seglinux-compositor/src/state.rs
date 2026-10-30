//! The compositor's state and the Wayland protocols it speaks. Based on
//! Smithay's smallvil and anvil examples (MIT licence, Smithay authors).

use std::{ffi::OsString, sync::Arc, time::Instant};

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_layer_shell,
    delegate_output, delegate_primary_selection, delegate_seat, delegate_shm, delegate_viewporter,
    delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{
        LayerSurface, PopupKind, PopupManager, Space, Window, WindowSurfaceType,
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
    },
    input::{
        Seat, SeatHandler, SeatState,
        keyboard::XkbConfig,
        pointer::{CursorImageStatus, CursorImageSurfaceData, Focus},
    },
    output::Output,
    reexports::{
        calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel,
        },
        wayland_server::{
            Client, Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer, wl_output, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface, with_states,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::{OutputHandler, OutputManagerState},
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
            primary_selection::{
                PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
            },
        },
        shell::{
            wlr_layer::{
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
                XdgToplevelSurfaceData,
                decoration::{XdgDecorationHandler, XdgDecorationState},
            },
        },
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
    },
};

use crate::{
    config::Config,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab, resize_grab},
};

/// What the backend in use offers the rest of the compositor.
pub trait Backend {
    /// Switch to another virtual terminal (only on real hardware).
    fn change_vt(&mut self, _vt: i32) {}
    /// Something changed on screen: draw a new frame soon.
    fn schedule_render(&mut self) {}
    /// Hand a client's GPU buffer to the renderer.
    fn import_dmabuf(&mut self, _dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        false
    }
}

/// A window's place before it was maximized, snapped or made fullscreen.
struct Restore(std::sync::Mutex<Option<Rectangle<i32, Logical>>>);

/// Where a window was when it was minimized.
struct MinimizedAt(std::sync::Mutex<Point<i32, Logical>>);

#[allow(dead_code)] // protocol states are kept alive here
pub struct State {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, State>,
    pub loop_signal: LoopSignal,
    pub backend: Box<dyn Backend>,

    pub space: Space<Window>,
    pub popups: PopupManager,
    /// Minimized windows, out of the space until they are shown again.
    pub minimized: Vec<Window>,
    pub cursor_status: CursorImageStatus,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub viewporter_state: ViewporterState,
    pub seat: Seat<State>,
    pub foreign_toplevel: crate::foreign_toplevel::ForeignToplevelState,

    pub config: Config,
    pub config_path: Option<std::path::PathBuf>,
}

struct NoBackend;
impl Backend for NoBackend {}

impl State {
    pub fn new(
        display: Display<State>,
        loop_handle: LoopHandle<'static, State>,
        loop_signal: LoopSignal,
        seat_name: &str,
        config_path: Option<std::path::PathBuf>,
    ) -> Self {
        let dh = display.handle();
        let (config, problems) = Config::load(config_path.as_deref());
        for p in problems {
            tracing::warn!("{p}");
        }

        let mut seat_state = SeatState::new();
        let mut seat: Seat<State> = seat_state.new_wl_seat(&dh, seat_name);
        let keyboard = &config.keyboard;
        let xkb = XkbConfig {
            layout: &keyboard.layout,
            variant: &keyboard.variant,
            options: keyboard.options.clone(),
            ..Default::default()
        };
        let (delay, rate) = (
            keyboard.repeat_delay.unwrap_or(300),
            keyboard.repeat_rate.unwrap_or(30),
        );
        if seat.add_keyboard(xkb, delay, rate).is_err() {
            tracing::warn!("keyboard layout `{}` not found; using us", keyboard.layout);
            seat.add_keyboard(XkbConfig::default(), delay, rate)
                .expect("the default keyboard layout");
        }
        seat.add_pointer();

        // The socket clients connect to, and the clients' messages.
        let socket = ListeningSocketSource::new_auto().expect("a Wayland socket");
        let socket_name = socket.socket_name().to_os_string();
        loop_handle
            .insert_source(socket, |stream, _, state| {
                if let Err(e) = state
                    .display_handle
                    .insert_client(stream, Arc::new(ClientState::default()))
                {
                    tracing::warn!("a client could not connect: {e}");
                }
            })
            .expect("the socket source");
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: the display is not dropped while the loop runs.
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("the display source");

        State {
            start_time: Instant::now(),
            socket_name,
            compositor_state: CompositorState::new::<Self>(&dh),
            xdg_shell_state: XdgShellState::new::<Self>(&dh),
            xdg_decoration_state: XdgDecorationState::new::<Self>(&dh),
            layer_shell_state: WlrLayerShellState::new::<Self>(&dh),
            shm_state: ShmState::new::<Self>(&dh, vec![]),
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&dh),
            data_device_state: DataDeviceState::new::<Self>(&dh),
            primary_selection_state: PrimarySelectionState::new::<Self>(&dh),
            viewporter_state: ViewporterState::new::<Self>(&dh),
            foreign_toplevel: crate::foreign_toplevel::ForeignToplevelState::new(&dh),
            seat_state,
            seat,
            display_handle: dh,
            loop_handle,
            loop_signal,
            backend: Box::new(NoBackend),
            space: Space::default(),
            popups: PopupManager::default(),
            minimized: Vec::new(),
            cursor_status: CursorImageStatus::default_named(),
            config,
            config_path,
        }
    }

    /// Starts the autostart commands, with WAYLAND_DISPLAY pointing here.
    pub fn autostart(&self) {
        for command in &self.config.autostart {
            self.spawn(command);
        }
    }

    pub fn spawn(&self, command: &str) {
        let result = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .spawn();
        match result {
            // Reap it in a thread, so no zombie stays behind.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => tracing::warn!("cannot start `{command}`: {e}"),
        }
    }

    pub fn reload_config(&mut self) {
        let (config, problems) = Config::load(self.config_path.as_deref());
        for p in problems {
            tracing::warn!("{p}");
        }
        self.config = config;
        tracing::info!("settings reloaded");
    }

    /// The window and the position of the surface under a point, layers
    /// (the dock, menus) first as they are drawn above windows.
    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.space.output_under(pos).next()?;
        let output_geo = self.space.output_geometry(output)?;
        let layers = layer_map_for_output(output);
        let layer_hit = |layer: Layer| {
            layers
                .layer_under(layer, pos - output_geo.loc.to_f64())
                .and_then(|l| {
                    let loc = layers.layer_geometry(l)?.loc + output_geo.loc;
                    l.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + loc).to_f64()))
                })
        };
        if let Some(hit) = layer_hit(Layer::Overlay).or_else(|| layer_hit(Layer::Top)) {
            return Some(hit);
        }
        if self.deco_under(pos).is_some() {
            return None;
        }
        None.or_else(|| {
            self.space.element_under(pos).and_then(|(window, loc)| {
                window
                    .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + loc).to_f64()))
            })
        })
        .or_else(|| layer_hit(Layer::Bottom))
        .or_else(|| layer_hit(Layer::Background))
    }

    /// The title bar at `pos`, if one is in front there: its window and
    /// which part (title, minimize, maximize, close).
    pub fn deco_under(&self, pos: Point<f64, Logical>) -> Option<(Window, crate::deco::Hit)> {
        let p = pos.to_i32_floor();
        for window in self.space.elements().rev() {
            let Some(loc) = self.space.element_location(window) else {
                continue;
            };
            let size = window.geometry().size;
            if crate::deco::has_bar(window) {
                let bar = Rectangle::new(
                    (loc.x, loc.y - crate::deco::BAR).into(),
                    (size.w, crate::deco::BAR).into(),
                );
                if bar.contains(p) {
                    return Some((window.clone(), crate::deco::hit(size.w, p.x - loc.x)));
                }
            }
            if Rectangle::new(loc, size).contains(p) {
                return None;
            }
        }
        None
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }

    pub fn focused_window(&self) -> Option<Window> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        self.window_for_surface(&focus)
    }

    /// Raise and focus a window (None: focus nothing).
    pub fn focus(&mut self, window: Option<Window>) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(w) = &window {
            self.space.raise_element(w, true);
        }
        for w in self.space.elements() {
            if Some(w) != window.as_ref() {
                w.set_activated(false);
            }
            if let Some(t) = w.toplevel() {
                t.send_pending_configure();
            }
        }
        let surface = window.and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, serial);
        }
        self.backend.schedule_render();
    }

    /// Where windows may go on the output under the pointer: the screen
    /// minus the dock and other exclusive panels.
    pub fn work_area(&self) -> Option<Rectangle<i32, Logical>> {
        let pointer = self.seat.get_pointer()?.current_location();
        let output = self
            .space
            .output_under(pointer)
            .next()
            .or_else(|| self.space.outputs().next())?;
        let geo = self.space.output_geometry(output)?;
        let zone = layer_map_for_output(output).non_exclusive_zone();
        Some(Rectangle::new(geo.loc + zone.loc, zone.size))
    }

    fn remember(window: &Window, rect: Rectangle<i32, Logical>) {
        let slot = window
            .user_data()
            .get_or_insert(|| Restore(std::sync::Mutex::new(None)));
        let mut slot = slot.0.lock().unwrap();
        if slot.is_none() {
            *slot = Some(rect);
        }
    }

    fn take_restore(window: &Window) -> Option<Rectangle<i32, Logical>> {
        window
            .user_data()
            .get::<Restore>()
            .and_then(|r| r.0.lock().unwrap().take())
    }

    /// Put a window into `rect`, remembering where it was.
    pub fn place(
        &mut self,
        window: &Window,
        rect: Rectangle<i32, Logical>,
        state: Option<xdg_toplevel::State>,
    ) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        if let Some(loc) = self.space.element_location(window) {
            Self::remember(window, Rectangle::new(loc, window.geometry().size));
        }
        // Our title bar goes above the window, inside the rectangle.
        let mut rect = rect;
        let bar = toplevel
            .with_pending_state(|s| s.decoration_mode == Some(DecorationMode::ServerSide))
            && state != Some(xdg_toplevel::State::Fullscreen);
        if bar {
            rect.loc.y += crate::deco::BAR;
            rect.size.h -= crate::deco::BAR;
        }
        toplevel.with_pending_state(|s| {
            s.size = Some(rect.size);
            s.states.unset(xdg_toplevel::State::Maximized);
            s.states.unset(xdg_toplevel::State::Fullscreen);
            if let Some(st) = state {
                s.states.set(st);
            }
        });
        toplevel.send_pending_configure();
        self.space.map_element(window.clone(), rect.loc, true);
    }

    /// Back to the size and place before maximize, snap or fullscreen.
    pub fn restore(&mut self, window: &Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let rect = Self::take_restore(window);
        toplevel.with_pending_state(|s| {
            s.states.unset(xdg_toplevel::State::Maximized);
            s.states.unset(xdg_toplevel::State::Fullscreen);
            s.size = rect.map(|r| r.size);
        });
        toplevel.send_pending_configure();
        if let Some(rect) = rect {
            self.space.map_element(window.clone(), rect.loc, true);
        }
    }

    pub fn is_state(window: &Window, state: xdg_toplevel::State) -> bool {
        window
            .toplevel()
            .is_some_and(|t| t.with_pending_state(|s| s.states.contains(state)))
    }

    pub fn toggle_maximize(&mut self, window: &Window) {
        if Self::is_state(window, xdg_toplevel::State::Maximized) {
            self.restore(window);
        } else if let Some(area) = self.work_area() {
            self.place(window, area, Some(xdg_toplevel::State::Maximized));
        }
    }

    pub fn toggle_fullscreen(&mut self, window: &Window) {
        if Self::is_state(window, xdg_toplevel::State::Fullscreen) {
            self.restore(window);
            return;
        }
        let pointer = self
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let output = self
            .space
            .output_under(pointer)
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned();
        if let Some(geo) = output.and_then(|o| self.space.output_geometry(&o)) {
            self.place(window, geo, Some(xdg_toplevel::State::Fullscreen));
        }
    }

    pub fn snap(&mut self, window: &Window, left: bool) {
        if let Some(area) = self.work_area() {
            let half = area.size.w / 2;
            let loc = if left {
                area.loc
            } else {
                (area.loc.x + half, area.loc.y).into()
            };
            self.place(
                window,
                Rectangle::new(loc, (half, area.size.h).into()),
                None,
            );
        }
    }

    pub fn minimize(&mut self, window: &Window) {
        if let Some(loc) = self.space.element_location(window) {
            Self::minimized_at(window, Some(loc));
        }
        self.space.unmap_elem(window);
        self.minimized.push(window.clone());
        let next = self.space.elements().last().cloned();
        self.focus(next);
    }

    /// Back from the dock: where it was before it was minimized.
    pub fn unminimize(&mut self, window: &Window) {
        if let Some(i) = self.minimized.iter().position(|w| w == window) {
            let window = self.minimized.remove(i);
            let loc = Self::minimized_at(&window, None);
            self.space.map_element(window, loc, false);
        }
    }

    /// Store (Some) or read (None) where a window was minimized.
    fn minimized_at(window: &Window, set: Option<Point<i32, Logical>>) -> Point<i32, Logical> {
        let slot = window
            .user_data()
            .get_or_insert(|| MinimizedAt(std::sync::Mutex::new(Point::default())));
        let mut slot = slot.0.lock().unwrap();
        if let Some(loc) = set {
            *slot = loc;
        }
        *slot
    }

    pub fn unminimize_all(&mut self) {
        for window in std::mem::take(&mut self.minimized) {
            let loc = Self::minimized_at(&window, None);
            self.space.map_element(window, loc, false);
        }
    }

    /// Super+D: minimize everything, or bring it all back.
    pub fn show_desktop(&mut self) {
        if self.space.elements().next().is_none() {
            self.unminimize_all();
            return;
        }
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for w in windows {
            if let Some(loc) = self.space.element_location(&w) {
                Self::minimized_at(&w, Some(loc));
            }
            self.space.unmap_elem(&w);
            self.minimized.push(w);
        }
        self.focus(None);
    }

    /// Alt+Tab: the window at the bottom of the stack comes to the top.
    pub fn cycle_windows(&mut self) {
        let first = self.space.elements().next().cloned();
        if first.is_some() {
            self.focus(first);
        }
    }

    /// A new window: centred in the work area, a little lower and to the
    /// right of the previous one so they do not cover each other exactly.
    fn place_new(&mut self, window: Window) {
        let area = self
            .work_area()
            .unwrap_or_else(|| Rectangle::from_size((1280, 720).into()));
        let count = self.space.elements().count() as i32;
        let offset = (count % 8) * 32;
        let size = window.geometry().size;
        let w = if size.w > 0 {
            size.w
        } else {
            area.size.w * 2 / 3
        };
        let h = if size.h > 0 {
            size.h
        } else {
            area.size.h * 2 / 3
        };
        let bar = if crate::deco::has_bar(&window) {
            crate::deco::BAR
        } else {
            0
        };
        let x = area.loc.x + ((area.size.w - w) / 2).max(0) + offset;
        let y = area.loc.y + ((area.size.h - h - bar) / 2).max(0) + offset + bar;
        tracing::info!(x, y, w, h, "window shown");
        self.space.map_element(window.clone(), (x, y), true);
        self.focus(Some(window));
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self.window_for_surface(&root) else {
            // A layer surface's popup (the dock's menus): keep it on the output.
            let Some(output) = self.space.outputs().find(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            }) else {
                return;
            };
            let map = layer_map_for_output(output);
            let Some(layer_geo) = map
                .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                .and_then(|l| map.layer_geometry(l))
            else {
                return;
            };
            let mut target = Rectangle::from_size(
                output
                    .current_mode()
                    .map(|m| m.size.to_logical(1))
                    .unwrap_or_default(),
            );
            target.loc -= layer_geo.loc;
            target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
            popup.with_pending_state(|s| {
                s.geometry = s.positioner.get_unconstrained_geometry(target)
            });
            return;
        };
        let Some(window_geo) = self.space.element_geometry(&window) else {
            return;
        };
        let Some(output) = self.space.outputs_for_element(&window).into_iter().next() else {
            return;
        };
        let Some(mut target) = self.space.output_geometry(&output) else {
            return;
        };
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;
        popup.with_pending_state(|s| s.geometry = s.positioner.get_unconstrained_geometry(target));
    }

    /// Re-arrange the panels of every output (after a mode change).
    pub fn arrange_layers(&mut self) {
        for output in self.space.outputs() {
            layer_map_for_output(output).arrange();
        }
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
            }
        }

        // A toplevel's first commit gets its first configure; its first
        // buffer places it.
        let mapped = self
            .space
            .elements()
            .any(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface));
        if !mapped
            && let Some(index) = PENDING.with(|p| {
                p.borrow()
                    .iter()
                    .position(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            })
        {
            let window = PENDING.with(|p| p.borrow()[index].clone());
            let initial_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .map(|d| d.lock().unwrap().initial_configure_sent)
                    .unwrap_or(true)
            });
            if !initial_sent {
                if let Some(t) = window.toplevel() {
                    t.send_configure();
                }
            } else {
                window.on_commit();
                if window.bbox().size.w > 0 {
                    PENDING.with(|p| p.borrow_mut().remove(index));
                    self.place_new(window);
                }
            }
        }

        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(xdg)) = self.popups.find_popup(surface)
            && !xdg.is_initial_configure_sent()
        {
            let _ = xdg.send_configure();
        }

        if let Some(output) = self
            .space
            .outputs()
            .find(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
        {
            let initial_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<LayerSurfaceData>()
                    .map(|d| d.lock().unwrap().initial_configure_sent)
                    .unwrap_or(true)
            });
            let mut map = layer_map_for_output(&output);
            map.arrange();
            if !initial_sent
                && let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            {
                layer.layer_surface().send_configure();
            }
        }

        resize_grab::handle_commit(&mut self.space, surface);
        self.backend.schedule_render();
    }
}

thread_local! {
    /// Toplevels that have no buffer yet: placed once they show something,
    /// so their size is known.
    static PENDING: std::cell::RefCell<Vec<Window>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self.backend.import_dmabuf(&dmabuf) {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.backend.schedule_render();
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
        if let Some(window) = focused.and_then(|s| self.window_for_surface(s)) {
            window.set_activated(true);
            if let Some(t) = window.toplevel() {
                t.send_pending_configure();
            }
        }
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}
impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl OutputHandler for State {}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Client-side decorations by default (GTK and Qt draw their own).
        let window = Window::new_wayland_window(surface);
        PENDING.with(|p| p.borrow_mut().push(window));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        PENDING.with(|p| {
            p.borrow_mut()
                .retain(|w| w.toplevel().is_some_and(|t| t != &surface))
        });
        self.minimized
            .retain(|w| w.toplevel().is_some_and(|t| t != &surface));
        let gone = self.window_for_surface(surface.wl_surface());
        if let Some(window) = gone {
            self.space.unmap_elem(&window);
            let next = self.space.elements().last().cloned();
            self.focus(next);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else {
            return;
        };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };
        let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) else {
            return;
        };
        if Self::is_state(&window, xdg_toplevel::State::Maximized) {
            self.restore(&window);
        }
        let Some(initial_window_location) = self.space.element_location(&window) else {
            return;
        };
        let grab = MoveSurfaceGrab {
            start_data,
            window,
            initial_window_location,
        };
        if let Some(pointer) = seat.get_pointer() {
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else {
            return;
        };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };
        let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        let size = window.geometry().size;
        surface.with_pending_state(|state| state.states.set(xdg_toplevel::State::Resizing));
        surface.send_pending_configure();
        let grab =
            ResizeSurfaceGrab::start(start_data, window, edges.into(), Rectangle::new(loc, size));
        if let Some(pointer) = seat.get_pointer() {
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if !Self::is_state(&window, xdg_toplevel::State::Maximized) {
                self.toggle_maximize(&window);
            }
        } else {
            // Not shown yet: it starts maximized.
            if let Some(area) = self.work_area() {
                surface.with_pending_state(|s| {
                    s.states.set(xdg_toplevel::State::Maximized);
                    s.size = Some(area.size);
                });
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface())
            && Self::is_state(&window, xdg_toplevel::State::Maximized)
        {
            self.restore(&window);
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if !Self::is_state(&window, xdg_toplevel::State::Fullscreen) {
                self.toggle_fullscreen(&window);
            }
        } else if let Some(output) = self.space.outputs().next().cloned()
            && let Some(geo) = self.space.output_geometry(&output)
        {
            surface.with_pending_state(|s| {
                s.states.set(xdg_toplevel::State::Fullscreen);
                s.size = Some(geo.size);
            });
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface())
            && Self::is_state(&window, xdg_toplevel::State::Fullscreen)
        {
            self.restore(&window);
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.minimize(&window);
        }
    }
}

fn check_grab(
    seat: &Seat<State>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<smithay::input::pointer::GrabStartData<State>> {
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }
    Some(start_data)
}

impl XdgDecorationHandler for State {
    /// Applications that can draw their own title bar or use ours get ours,
    /// unless they ask otherwise.
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    }
    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.request_mode(toplevel, DecorationMode::ServerSide);
    }
}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            tracing::warn!("cannot show a panel: {e}");
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&popup);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let found = self.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|l| l.layer_surface() == &surface)
                .cloned();
            layer.map(|l| (o.clone(), l))
        });
        if let Some((output, layer)) = found {
            layer_map_for_output(&output).unmap_layer(&layer);
        }
        self.backend.schedule_render();
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_dmabuf!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_output!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_layer_shell!(State);
delegate_viewporter!(State);

/// The cursor image a client set: its hotspot.
pub fn cursor_hotspot(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .map(|d| d.lock().unwrap().hotspot)
            .unwrap_or_default()
    })
}
