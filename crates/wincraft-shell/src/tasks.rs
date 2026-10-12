//! Window buttons use wlroots foreign-toplevel, never process-name guessing.
use gtk::{glib, prelude::*};
use std::{collections::BTreeMap, sync::mpsc, time::Duration};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as handle, zwlr_foreign_toplevel_manager_v1 as manager,
};

#[derive(Clone, Debug)]
struct Task {
    id: u32,
    title: String,
    active: bool,
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
}

pub fn attach(container: &gtk::Box) {
    let (updates_tx, updates_rx) = mpsc::channel();
    let (actions_tx, actions_rx) = mpsc::channel::<u32>();
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
                    Ok(id) => {
                        if let (Some(entry), Some(seat)) = (state.windows.get(&id), &state.seat) {
                            if entry.task.active {
                                entry.proxy.set_minimized();
                            } else {
                                entry.proxy.unset_minimized();
                                entry.proxy.activate(seat);
                            }
                        }
                    }
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
    let weak = container.downgrade();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(container) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if let Some(tasks) = updates_rx.try_iter().last() {
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }
            for task in tasks {
                let button = gtk::Button::new();
                button.add_css_class("task");
                if task.active {
                    button.add_css_class("active");
                }
                let label = gtk::Label::new(Some(&task.title));
                label.set_max_width_chars(22);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                button.set_child(Some(&label));
                button.set_tooltip_text(Some(&task.title));
                let actions = actions_tx.clone();
                button.connect_clicked(move |_| {
                    let _ = actions.send(task.id);
                });
                container.append(&button);
            }
        }
        glib::ControlFlow::Continue
    });
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
                        active: false,
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
            handle::Event::State { state: flags } => {
                if let Some(entry) = state.windows.get_mut(&id) {
                    entry.task.active = flags.as_chunks::<4>().0.iter().any(|bytes| {
                        let value = u32::from_ne_bytes(*bytes);
                        matches!(
                            WEnum::<handle::State>::from(value),
                            WEnum::Value(handle::State::Activated)
                        )
                    });
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
