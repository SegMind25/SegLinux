//! X11 applications (Steam, Wine and Proton games, older programs) through
//! XWayland: their windows are SegLinux windows like any other, their menus
//! and tooltips appear where they ask.

use smithay::{
    delegate_xwayland_shell,
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Rectangle},
    wayland::{
        compositor::CompositorHandler,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use std::os::fd::OwnedFd;

use smithay::wayland::selection::{
    SelectionTarget,
    data_device::{
        clear_data_device_selection, current_data_device_selection_userdata,
        request_data_device_client_selection, set_data_device_selection,
    },
    primary_selection::{
        clear_primary_selection, current_primary_selection_userdata,
        request_primary_client_selection, set_primary_selection,
    },
};

use crate::state::State;

impl State {
    /// Start XWayland; the autostart runs once it is ready (or failed), so
    /// that everything started from the desktop finds it ($DISPLAY).
    pub fn start_xwayland(&mut self) {
        // X11's socket folder (normally made at boot).
        let dir = std::path::Path::new("/tmp/.X11-unix");
        if !dir.exists() && std::fs::create_dir(dir).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o1777));
        }
        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            |_| (),
        );
        let (xwayland, client) = match spawned {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("no XWayland ({e}): X11 applications will not start");
                self.autostart();
                return;
            }
        };
        let result = self
            .loop_handle
            .insert_source(xwayland, move |event, _, state| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    state.client_compositor_state(&client).set_client_scale(1.0);
                    match X11Wm::start_wm(state.loop_handle.clone(), x11_socket, client.clone()) {
                        Ok(wm) => state.xwm = Some(wm),
                        Err(e) => tracing::warn!("X11 window manager: {e}"),
                    }
                    // Safety: set before any thread reads the environment.
                    unsafe {
                        std::env::set_var("DISPLAY", format!(":{display_number}"));
                    }
                    tracing::info!("XWayland on :{display_number}");
                    state.autostart();
                }
                XWaylandEvent::Error => {
                    tracing::warn!("XWayland did not start: X11 applications will not start");
                    state.autostart();
                }
            });
        if let Err(e) = result {
            tracing::warn!("XWayland: {e}");
            self.autostart();
        }
    }

    fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .chain(self.minimized.iter())
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

delegate_xwayland_shell!(State);

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("the X11 window manager")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        let _ = surface.set_mapped(true);
        let window = Window::new_x11_window(surface.clone());
        self.place_new(window.clone());
        if let Some(loc) = self.space.element_location(&window) {
            let _ = surface.configure(Rectangle::new(loc, surface.geometry().size));
        }
    }

    /// Menus, tooltips, drop-downs: where the application put them, on top.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let loc = surface.geometry().loc;
        self.space
            .map_element(Window::new_x11_window(surface), loc, true);
        self.backend.schedule_render();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let had_focus = self
            .focused_window()
            .is_some_and(|w| w.x11_surface() == Some(&surface));
        if let Some(window) = self.x11_window(&surface) {
            self.space.unmap_elem(&window);
            self.minimized.retain(|w| w != &window);
        }
        // Hidden on another workspace (Steam to its tray): it stays hidden.
        self.hidden
            .retain(|(w, _, _)| w.x11_surface() != Some(&surface));
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
        }
        if had_focus {
            let top = self.space.elements().last().cloned();
            self.focus(top);
        }
        self.backend.schedule_render();
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geo = surface.geometry();
        // A window not shown yet may choose its place; a shown one only
        // its size (games resize themselves when they change mode).
        if self.x11_window(&surface).is_none() {
            if let Some(x) = x {
                geo.loc.x = x;
            }
            if let Some(y) = y {
                geo.loc.y = y;
            }
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = surface.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        if surface.is_override_redirect()
            && let Some(window) = self.x11_window(&surface)
        {
            self.space.map_element(window, geometry.loc, false);
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.x11_window(&surface)
            && !surface.is_maximized()
        {
            self.toggle_maximize(&w);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.x11_window(&surface)
            && surface.is_maximized()
        {
            self.restore(&w);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.x11_window(&surface)
            && !surface.is_fullscreen()
        {
            self.toggle_fullscreen(&w);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.x11_window(&surface)
            && State::is_state(&w, xdg_toplevel::State::Fullscreen)
        {
            self.restore(&w);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.x11_window(&surface) {
            self.minimize(&w);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.x11_window(&surface) {
            self.unminimize(&w);
            self.focus(Some(w));
        }
    }

    /// Resizing from a border the application draws itself.
    fn resize_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32, edge: ResizeEdge) {
        let Some(window) = self.x11_window(&surface) else {
            return;
        };
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button,
            location: pointer.current_location(),
        };
        let rect = Rectangle::new(loc, surface.geometry().size);
        let grab = crate::grabs::ResizeSurfaceGrab::start(start_data, window, edge.into(), rect);
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Dragging a window by a title bar the application draws itself
    /// (Steam's).
    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32) {
        let Some(window) = self.x11_window(&surface) else {
            return;
        };
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        let grab = crate::grabs::MoveSurfaceGrab {
            start_data: smithay::input::pointer::GrabStartData {
                focus: None,
                button,
                location: pointer.current_location(),
            },
            unsnap: State::is_state(&window, xdg_toplevel::State::Maximized)
                || crate::snap::is_snapped(&window),
            window,
            initial_window_location: loc,
        };
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// X11 applications may read the clipboard while one of them is in front.
    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(
                |f| matches!(f, crate::focus::KeyboardFocus::X11(x) if x.xwm_id() == Some(xwm)),
            )
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let failed = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd).is_err()
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime_type, fd).is_err()
            }
        };
        if failed {
            tracing::warn!("the clipboard could not be handed to an X11 application");
        }
    }

    /// An X11 application copied something: Wayland applications can paste it.
    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat)
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat)
                }
            }
        }
    }
}
