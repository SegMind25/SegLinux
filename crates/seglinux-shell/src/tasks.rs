//! Open windows, from wlroots foreign-toplevel (never process-name
//! guessing): the dock lists them and can activate, minimize, close them and
//! show the desktop.
use gtk::{glib, prelude::*};
use std::{collections::BTreeMap, sync::mpsc, time::Duration};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
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

/// Sends actions to the compositor connection.
#[derive(Clone)]
pub struct Tasks(mpsc::Sender<Action>);

impl Tasks {
    pub fn send(&self, action: Action) {
        let _ = self.0.send(action);
    }
}

struct Entry {
    proxy: handle::ZwlrForeignToplevelHandleV1,
    task: Task,
}
struct State {
    windows: BTreeMap<u32, Entry>,
    seat: Option<wl_seat::WlSeat>,
    updates: mpsc::Sender<Vec<Task>>,
}
impl State {
    fn publish(&self) {
        let _ = self
            .updates
            .send(self.windows.values().map(|w| w.task.clone()).collect());
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

/// Follows the open windows. `on_update` runs on the GTK main loop whenever
/// the list changes, for as long as `owner` exists: when the dock is
/// rebuilt (new monitor, changed settings), the old one's timer stops, and
/// its compositor connection closes once its `Tasks` are dropped.
pub fn watch(owner: &impl IsA<gtk::Widget>, on_update: impl Fn(&[Task]) + 'static) -> Tasks {
    let (updates_tx, updates_rx) = mpsc::channel();
    let (actions_tx, actions_rx) = mpsc::channel::<Action>();
    std::thread::spawn(move || {
        let Ok(connection) = Connection::connect_to_env() else {
            return;
        };
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = State {
            windows: BTreeMap::new(),
            seat: None,
            updates: updates_tx,
        };
        if queue.roundtrip(&mut state).is_err() {
            return;
        }
        loop {
            if queue.dispatch_pending(&mut state).is_err() {
                break;
            }
            loop {
                match actions_rx.try_recv() {
                    Ok(action) => state.apply(action),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if connection.flush().is_err() {
                break;
            }
            // Non-blocking socket read; a short poll keeps UI commands responsive.
            if let Some(guard) = connection.prepare_read() {
                match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(e))
                        if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
            std::thread::sleep(Duration::from_millis(35));
        }
    });
    let owner = owner.downgrade();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if owner.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        if let Some(tasks) = updates_rx.try_iter().last() {
            on_update(&tasks);
        }
        glib::ControlFlow::Continue
    });
    Tasks(actions_tx)
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
                }
            }
            handle::Event::Done => state.publish(),
            handle::Event::Closed => {
                state.windows.remove(&id);
                proxy.destroy();
                state.publish();
            }
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
