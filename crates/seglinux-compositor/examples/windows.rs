//! Lists the compositor's windows as the dock sees them, and with
//! `minimize` / `restore` asks for that, as the dock does. For testing:
//!   WAYLAND_DISPLAY=wayland-1 cargo run -p seglinux-compositor --example windows \
//!     [minimize|restore|maximize|unmaximize|activate|close [APP_ID]]

use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as handle, zwlr_foreign_toplevel_manager_v1 as manager,
};

#[derive(Default)]
struct Win {
    title: String,
    app_id: String,
    states: Vec<u32>,
}

#[derive(Default)]
struct App {
    wins: Vec<(handle::ZwlrForeignToplevelHandleV1, Win)>,
    seat: Option<wl_seat::WlSeat>,
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = e
        {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" => {
                    r.bind::<manager::ZwlrForeignToplevelManagerV1, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    );
                }
                "wl_seat" => s.seat = Some(r.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<manager::ZwlrForeignToplevelManagerV1, ()> for App {
    fn event(
        s: &mut Self,
        _: &manager::ZwlrForeignToplevelManagerV1,
        e: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = e {
            s.wins.push((toplevel, Win::default()));
        }
    }
    wayland_client::event_created_child!(App, manager::ZwlrForeignToplevelManagerV1, [0 => (handle::ZwlrForeignToplevelHandleV1, ())]);
}

impl Dispatch<handle::ZwlrForeignToplevelHandleV1, ()> for App {
    fn event(
        s: &mut Self,
        h: &handle::ZwlrForeignToplevelHandleV1,
        e: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, w)) = s.wins.iter_mut().find(|(x, _)| x == h) else {
            return;
        };
        match e {
            handle::Event::Title { title } => w.title = title,
            handle::Event::AppId { app_id } => w.app_id = app_id,
            handle::Event::State { state } => {
                w.states = state
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_ne_bytes(*c))
                    .collect()
            }
            handle::Event::Closed => s.wins.retain(|(x, _)| x != h),
            _ => {}
        }
        let _ = WEnum::<handle::State>::Value(handle::State::Activated);
    }
}

fn main() {
    let conn = Connection::connect_to_env().expect("no compositor");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut app = App::default();
    for _ in 0..3 {
        queue.roundtrip(&mut app).unwrap();
    }
    if let Some(cmd) = std::env::args().nth(1) {
        // An optional second argument limits it to one app id.
        let only = std::env::args().nth(2);
        for (h, w) in &app.wins {
            if only.as_ref().is_some_and(|o| *o != w.app_id) {
                continue;
            }
            match cmd.as_str() {
                "minimize" => h.set_minimized(),
                "restore" => h.unset_minimized(),
                "maximize" => h.set_maximized(),
                "unmaximize" => h.unset_maximized(),
                "activate" => {
                    if let Some(seat) = &app.seat {
                        h.activate(seat)
                    }
                }
                "close" => h.close(),
                _ => {}
            }
        }
        for _ in 0..3 {
            queue.roundtrip(&mut app).unwrap();
        }
    }
    for (_, w) in &app.wins {
        // 0 maximized, 1 minimized, 2 activated, 3 fullscreen
        println!("{:?} {:?} states={:?}", w.app_id, w.title, w.states);
    }
    println!("{} window(s)", app.wins.len());
}
