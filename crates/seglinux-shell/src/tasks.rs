//! Open windows, from wlroots foreign-toplevel (never process-name
//! guessing): the dock lists them and can activate, minimize, close them and
//! show the desktop.
//!
//! The compositor connection is part of GTK's main loop: the desktop wakes
//! up when the compositor has news (a window opened, closed, retitled,
//! focused), never to look for it.
use gtk::{glib, prelude::*};
use std::{cell::RefCell, collections::BTreeMap, os::fd::AsRawFd, rc::Rc};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as handle, zwlr_foreign_toplevel_manager_v1 as manager,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: u32,
    pub title: String,
    pub app_id: String,
    pub active: bool,
    pub minimized: bool,
    pub fullscreen: bool,
}

/// What the dock asks the compositor to do.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    /// Activate the window, or minimize it when it is already active.
    Toggle(u32),
    Close(u32),
    /// Minimize every window; when all are minimized, restore them.
    ShowDesktop,
}

struct Link {
    connection: Connection,
    queue: RefCell<EventQueue<State>>,
    state: RefCell<State>,
    source: RefCell<Option<glib::SourceId>>,
}

impl Drop for Link {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.remove();
        }
    }
}

/// Sends actions to the compositor (nothing without one, as in the preview
/// on X11).
#[derive(Clone)]
pub struct Tasks(Option<Rc<Link>>);

impl Tasks {
    pub fn send(&self, action: Action) {
        if let Some(link) = &self.0 {
            link.state.borrow().apply(action);
            let _ = link.connection.flush();
        }
    }
}

struct Entry {
    proxy: handle::ZwlrForeignToplevelHandleV1,
    task: Task,
}
#[derive(Default)]
struct State {
    windows: BTreeMap<u32, Entry>,
    seat: Option<wl_seat::WlSeat>,
    changed: bool,
}
impl State {
    fn tasks(&self) -> Vec<Task> {
        self.windows.values().map(|w| w.task.clone()).collect()
    }

    fn apply(&self, action: Action) {
        match action {
            Action::Toggle(id) => {
                if let (Some(entry), Some(seat)) = (self.windows.get(&id), &self.seat) {
                    if entry.task.active && !entry.task.minimized {
                        entry.proxy.set_minimized();
                    } else {
                        entry.proxy.unset_minimized();
                        entry.proxy.activate(seat);
                    }
                }
            }
            Action::Close(id) => {
                if let Some(entry) = self.windows.get(&id) {
                    entry.proxy.close();
                }
            }
            Action::ShowDesktop => {
                let all_minimized = self.windows.values().all(|w| w.task.minimized);
                for entry in self.windows.values() {
                    if all_minimized {
                        entry.proxy.unset_minimized();
                    } else {
                        entry.proxy.set_minimized();
                    }
                }
            }
        }
    }
}

/// Reads what the compositor sent and dispatches it. False when the
/// connection is gone.
fn pump(link: &Link) -> bool {
    let mut queue = link.queue.borrow_mut();
    let mut state = link.state.borrow_mut();
    if queue.dispatch_pending(&mut state).is_err() {
        return false;
    }
    if let Some(guard) = queue.prepare_read() {
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return false,
        }
    }
    queue.dispatch_pending(&mut state).is_ok() && link.connection.flush().is_ok()
}

/// Follows the open windows. `on_update` runs on the GTK main loop whenever
/// the list changes, for as long as `owner` exists: when the dock is
/// rebuilt (new monitor, changed settings), the old one stops listening and
/// its compositor connection closes.
pub fn watch(owner: &impl IsA<gtk::Widget>, on_update: impl Fn(&[Task]) + 'static) -> Tasks {
    let Ok(connection) = Connection::connect_to_env() else {
        return Tasks(None);
    };
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State::default();
    // The first round trip lists the globals (and binds the window list
    // while handling them); the second sends that and brings the windows
    // already open.
    if queue.roundtrip(&mut state).is_err() || queue.roundtrip(&mut state).is_err() {
        return Tasks(None);
    }
    let fd = connection.backend().poll_fd().as_raw_fd();
    let link = Rc::new(Link {
        connection,
        queue: RefCell::new(queue),
        state: RefCell::new(state),
        source: RefCell::default(),
    });
    let weak = Rc::downgrade(&link);
    let report = move |link: &Link| {
        let tasks = {
            let mut state = link.state.borrow_mut();
            if !std::mem::take(&mut state.changed) {
                return;
            }
            state.tasks()
        };
        on_update(&tasks);
    };
    report(&link);
    let source = glib::unix_fd_add_local(
        fd,
        glib::IOCondition::IN | glib::IOCondition::ERR | glib::IOCondition::HUP,
        move |_, condition| {
            let Some(link) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let alive = !condition.intersects(glib::IOCondition::ERR | glib::IOCondition::HUP)
                && pump(&link);
            report(&link);
            if alive {
                glib::ControlFlow::Continue
            } else {
                // The compositor is gone (the session is ending).
                link.source.take();
                glib::ControlFlow::Break
            }
        },
    );
    link.source.replace(Some(source));
    // The dock owns the connection: it closes with the dock. (The listener
    // holds the dock's buttons, which hold the connection: stopping the
    // listener breaks that circle.)
    let held = RefCell::new(Some(link.clone()));
    owner.as_ref().connect_destroy(move |_| {
        if let Some(link) = held.take()
            && let Some(source) = link.source.take()
        {
            source.remove();
        }
    });
    Tasks(Some(link))
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" => {
                    registry.bind::<manager::ZwlrForeignToplevelManagerV1, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    );
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<manager::ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &manager::ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = event {
            let id = toplevel.id().protocol_id();
            state.windows.insert(
                id,
                Entry {
                    proxy: toplevel,
                    task: Task {
                        id,
                        title: "Application".into(),
                        app_id: String::new(),
                        active: false,
                        minimized: false,
                        fullscreen: false,
                    },
                },
            );
        }
    }
    wayland_client::event_created_child!(State, manager::ZwlrForeignToplevelManagerV1, [0 => (handle::ZwlrForeignToplevelHandleV1, ())]);
}
impl Dispatch<handle::ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &handle::ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        match event {
            handle::Event::Title { title } => {
                if let Some(entry) = state.windows.get_mut(&id) {
                    entry.task.title = title;
                }
            }
            handle::Event::AppId { app_id } => {
                if let Some(entry) = state.windows.get_mut(&id) {
                    entry.task.app_id = app_id;
                }
            }
            handle::Event::State { state: flags } => {
                if let Some(entry) = state.windows.get_mut(&id) {
                    let states: Vec<WEnum<handle::State>> = flags
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|bytes| WEnum::from(u32::from_ne_bytes(*bytes)))
                        .collect();
                    let has = |s: handle::State| states.contains(&WEnum::Value(s));
                    entry.task.active = has(handle::State::Activated);
                    entry.task.minimized = has(handle::State::Minimized);
                    entry.task.fullscreen = has(handle::State::Fullscreen);
                }
            }
            handle::Event::Done => state.changed = true,
            handle::Event::Closed => {
                state.windows.remove(&id);
                proxy.destroy();
                state.changed = true;
            }
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
