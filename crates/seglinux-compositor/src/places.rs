//! Windows open where they were last closed: each application's last place
//! and size (by its app id; an X11 one by its class), kept across restarts
//! in ~/.local/state/seglinux/windows.txt. One closed maximized opens
//! maximized; a place on a screen that is gone is not used (the window
//! opens centered, as the first time).

use std::{collections::HashMap, path::PathBuf};

use smithay::{
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Rectangle, Size},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::state::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    /// The window itself, below its title bar.
    pub rect: Rectangle<i32, Logical>,
    pub maximized: bool,
}

#[derive(Default)]
pub struct Places(HashMap<String, Place>);

fn file() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("seglinux/windows.txt"))
}

/// One line per application: `x y width height maximized(0/1) app-id`.
fn parse(text: &str) -> HashMap<String, Place> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.splitn(6, ' ');
            let mut n = || f.next()?.parse::<i32>().ok();
            let (x, y, w, h, m) = (n()?, n()?, n()?, n()?, n()?);
            let app = f.next()?.to_string();
            (w > 0 && h > 0 && !app.is_empty()).then(|| {
                let rect = Rectangle::new((x, y).into(), (w, h).into());
                (
                    app,
                    Place {
                        rect,
                        maximized: m == 1,
                    },
                )
            })
        })
        .collect()
}

impl Places {
    pub fn load() -> Self {
        let text = file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        Places(parse(&text))
    }

    fn set(&mut self, app: String, place: Place) {
        if self.0.get(&app) == Some(&place) {
            return;
        }
        self.0.insert(app, place);
        let Some(path) = file() else {
            return;
        };
        let mut text = String::new();
        for (app, p) in &self.0 {
            let r = p.rect;
            text += &format!(
                "{} {} {} {} {} {app}\n",
                r.loc.x,
                r.loc.y,
                r.size.w,
                r.size.h,
                u8::from(p.maximized)
            );
        }
        let tmp = path.with_extension("tmp");
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

struct LastSize(std::sync::Mutex<Size<i32, Logical>>);

/// A window drew a frame: its size, for when it closes.
pub fn note_size(window: &Window) {
    let size = window.geometry().size;
    if size.w > 0 && size.h > 0 {
        *window
            .user_data()
            .get_or_insert_threadsafe(|| LastSize(std::sync::Mutex::new(size)))
            .0
            .lock()
            .unwrap() = size;
    }
}

/// The application a window belongs to.
pub fn app_id(window: &Window) -> String {
    if let Some(x) = window.x11_surface() {
        return x.class();
    }
    window
        .toplevel()
        .and_then(|t| {
            with_states(t.wl_surface(), |s| {
                s.data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().unwrap().app_id.clone())
            })
        })
        .unwrap_or_default()
}

impl State {
    /// A window closes: its place is kept for the next time (not a
    /// fullscreen one: games set their own).
    pub fn remember_place(&mut self, window: &Window) {
        let app = app_id(window);
        if app.is_empty() || Self::is_state(window, xdg_toplevel::State::Fullscreen) {
            return;
        }
        let maximized = Self::is_state(window, xdg_toplevel::State::Maximized);
        let rect = if maximized {
            Self::restore_rect(window)
        } else {
            // Its size from its last frame: an application that quit
            // (or crashed) has no surface left to ask.
            let size = window
                .user_data()
                .get::<LastSize>()
                .map_or_else(|| window.geometry().size, |s| *s.0.lock().unwrap());
            self.space
                .element_location(window)
                .map(|loc| Rectangle::new(loc, size))
        };
        if let Some(rect) = rect.filter(|r| r.size.w > 0 && r.size.h > 0) {
            self.places.set(app, Place { rect, maximized });
        }
    }

    /// Where a new window goes: its application's last place, while that
    /// is still on a screen.
    pub fn saved_place(&self, window: &Window) -> Option<Place> {
        let place = *self.places.0.get(&app_id(window))?;
        self.space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .any(|g| {
                g.intersection(place.rect)
                    .is_some_and(|i| i.size.w >= 100 && i.size.h >= 50)
            })
            .then_some(place)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        let places =
            parse("10 40 800 600 0 firefox\n0 30 1366 700 1 Steam Client\nbad\n1 2 0 4 0 x\n");
        assert_eq!(places.len(), 2);
        assert_eq!(places["firefox"].rect.size.w, 800);
        assert!(places["Steam Client"].maximized);
    }
}
