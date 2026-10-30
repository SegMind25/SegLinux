//! The list of open windows for the dock (wlr-foreign-toplevel-management):
//! titles, app ids and states go out; activate, minimize, maximize,
//! fullscreen and close come back.

use std::sync::atomic::{AtomicU64, Ordering};

use smithay::{
    desktop::Window,
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_protocols_wlr::foreign_toplevel::v1::server::{
            zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
            zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::state::State;

/// A window's number for the protocol, kept in its user data.
struct WindowId(u64);

pub fn window_id(window: &Window) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    window
        .user_data()
        .get_or_insert(|| WindowId(NEXT.fetch_add(1, Ordering::Relaxed)))
        .0
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Info {
    title: String,
    app_id: String,
    activated: bool,
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
}

impl Info {
    fn states(&self) -> Vec<u8> {
        let mut list = Vec::new();
        for (on, s) in [
            (self.maximized, handle::State::Maximized),
            (self.minimized, handle::State::Minimized),
            (self.activated, handle::State::Activated),
            (self.fullscreen, handle::State::Fullscreen),
        ] {
            if on {
                list.extend_from_slice(&(s as u32).to_ne_bytes());
            }
        }
        list
    }
}

struct Entry {
    id: u64,
    window: Window,
    info: Info,
    /// The screens it is on (kept while it is minimized).
    outputs: Vec<Output>,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
}

/// output_enter / output_leave for the screens a window came to or left.
fn send_outputs(h: &ZwlrForeignToplevelHandleV1, old: &[Output], new: &[Output]) {
    let Some(client) = h.client() else { return };
    for o in old.iter().filter(|o| !new.contains(o)) {
        for wl in o.client_outputs(&client) {
            h.output_leave(&wl);
        }
    }
    for o in new.iter().filter(|o| !old.contains(o)) {
        for wl in o.client_outputs(&client) {
            h.output_enter(&wl);
        }
    }
}

#[derive(Default)]
pub struct ForeignToplevelState {
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    entries: Vec<Entry>,
}

impl ForeignToplevelState {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<State, ZwlrForeignToplevelManagerV1, ()>(3, ());
        Self::default()
    }

    pub fn window(&self, id: u64) -> Option<Window> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.window.clone())
    }
}

fn info(window: &Window, activated: bool, minimized: bool) -> Info {
    if let Some(x) = window.x11_surface() {
        return Info {
            title: x.title(),
            app_id: x.class(),
            activated: activated && !minimized,
            minimized,
            maximized: x.is_maximized(),
            fullscreen: x.is_fullscreen(),
        };
    }
    let Some(toplevel) = window.toplevel() else {
        return Info::default();
    };
    let (title, app_id) = with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|d| {
                let d = d.lock().unwrap();
                (
                    d.title.clone().unwrap_or_default(),
                    d.app_id.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    });
    let has = |s| toplevel.with_pending_state(|p| p.states.contains(s));
    Info {
        title,
        app_id,
        activated: activated && !minimized,
        minimized,
        maximized: has(xdg_toplevel::State::Maximized),
        fullscreen: has(xdg_toplevel::State::Fullscreen),
    }
}

fn send(h: &ZwlrForeignToplevelHandleV1, old: Option<&Info>, new: &Info) {
    if old.map(|o| &o.title) != Some(&new.title) {
        h.title(new.title.clone());
    }
    if old.map(|o| &o.app_id) != Some(&new.app_id) {
        h.app_id(new.app_id.clone());
    }
    h.state(new.states());
    h.done();
}

fn new_handle(
    dh: &DisplayHandle,
    manager: &ZwlrForeignToplevelManagerV1,
    entry: &Entry,
) -> Option<ZwlrForeignToplevelHandleV1> {
    let client = manager.client()?;
    let handle = client
        .create_resource::<ZwlrForeignToplevelHandleV1, u64, State>(dh, manager.version(), entry.id)
        .ok()?;
    manager.toplevel(&handle);
    send_outputs(&handle, &[], &entry.outputs);
    send(&handle, None, &entry.info);
    Some(handle)
}

impl State {
    /// Tell the dock what changed since the last time: called once per
    /// turn of the event loop.
    pub fn refresh_foreign_toplevels(&mut self) {
        let focused = self.focused_window();
        let mut current: Vec<(Window, Info)> = self
            .space
            .elements()
            .map(|w| (w.clone(), info(w, Some(w) == focused.as_ref(), false)))
            .collect();
        current.extend(
            self.minimized
                .iter()
                .map(|w| (w.clone(), info(w, false, true))),
        );
        let dh = self.display_handle.clone();
        let ft = &mut self.foreign_toplevel;

        // Closed windows.
        ft.entries.retain(|e| {
            let alive = current.iter().any(|(w, _)| w == &e.window);
            if !alive {
                for h in &e.handles {
                    h.closed();
                }
            }
            alive
        });
        for (window, new) in current {
            let outputs = if new.minimized {
                None
            } else {
                Some(self.space.outputs_for_element(&window))
            };
            match ft.entries.iter_mut().find(|e| e.window == window) {
                Some(entry) => {
                    let moved = outputs.filter(|o| *o != entry.outputs);
                    if let Some(o) = &moved {
                        for h in &entry.handles {
                            send_outputs(h, &entry.outputs, o);
                        }
                        entry.outputs = o.clone();
                    }
                    if entry.info != new || moved.is_some() {
                        for h in &entry.handles {
                            send(h, Some(&entry.info), &new);
                        }
                        entry.info = new;
                    }
                }
                None => {
                    let mut entry = Entry {
                        id: window_id(&window),
                        window,
                        info: new,
                        outputs: outputs.unwrap_or_default(),
                        handles: Vec::new(),
                    };
                    for m in &ft.managers {
                        if let Some(h) = new_handle(&dh, m, &entry) {
                            entry.handles.push(h);
                        }
                    }
                    ft.entries.push(entry);
                }
            }
        }
    }
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn bind(
        state: &mut State,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ());
        let ft = &mut state.foreign_toplevel;
        for entry in &mut ft.entries {
            if let Some(h) = new_handle(dh, &manager, entry) {
                entry.handles.push(h);
            }
        }
        ft.managers.push(manager);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let manager::Request::Stop = request {
            resource.finished();
            state.foreign_toplevel.managers.retain(|m| m != resource);
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, u64> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        resource: &ZwlrForeignToplevelHandleV1,
        request: handle::Request,
        id: &u64,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let handle::Request::Destroy = request {
            for e in &mut state.foreign_toplevel.entries {
                e.handles.retain(|h| h != resource);
            }
            return;
        }
        let Some(window) = state.foreign_toplevel.window(*id) else {
            return;
        };
        let maximized = State::is_state(&window, xdg_toplevel::State::Maximized);
        let fullscreen = State::is_state(&window, xdg_toplevel::State::Fullscreen);
        match request {
            handle::Request::Activate { .. } => {
                state.unminimize(&window);
                state.focus(Some(window));
            }
            handle::Request::Close => {
                State::close(&window);
            }
            handle::Request::SetMinimized => {
                if !state.minimized.contains(&window) {
                    state.minimize(&window);
                }
            }
            handle::Request::UnsetMinimized => state.unminimize(&window),
            handle::Request::SetMaximized if !maximized => state.toggle_maximize(&window),
            handle::Request::UnsetMaximized if maximized => state.toggle_maximize(&window),
            handle::Request::SetFullscreen { .. } if !fullscreen => {
                state.toggle_fullscreen(&window)
            }
            handle::Request::UnsetFullscreen if fullscreen => state.toggle_fullscreen(&window),
            _ => {}
        }
        state.backend.schedule_render();
    }
}
