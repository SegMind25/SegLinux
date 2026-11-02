//! Laptops: the touchpad's gestures and the lid.
//! - three or four fingers swiped left or right: the next or previous
//!   workspace (with one workspace: the next window, as Alt+Tab); down:
//!   the desktop (every window minimized); up: the windows back
//! - pinch (zoom) and hold go to the application under the pointer
//!   (pointer-gestures: Firefox, image viewers, GTK)
//! - the lid closed with another screen connected: the laptop's own screen
//!   turns off and its windows move over; opened, it is back. Without
//!   another screen elogind suspends, as before.
//!
//! Windows left on a screen that is gone (unplugged, turned off, the lid)
//! move to one that is there, and so does the pointer.

use smithay::{
    desktop::Window,
    input::pointer as p,
    output::Output,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use crate::{config::Action, state::State};

/// How far the fingers travel (about pixels) before a swipe counts.
const SWIPE: f64 = 100.0;

/// The laptop's own screen (eDP, LVDS, DSI connectors).
fn built_in(output: &Output) -> bool {
    ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|p| output.name().starts_with(p))
}

impl State {
    pub fn swipe_begin(&mut self) {
        self.swipe = Some(Point::default());
    }

    pub fn swipe_update(&mut self, delta: Point<f64, Logical>) {
        if let Some(d) = &mut self.swipe {
            *d += delta;
        }
    }

    pub fn swipe_end(&mut self, cancelled: bool) {
        let Some(d) = self.swipe.take() else {
            return;
        };
        if cancelled || self.lock.locked {
            return;
        }
        if d.x.abs() > d.y.abs() && d.x.abs() > SWIPE {
            // Fingers to the left: the next one, as pages turn.
            if self.workspace_count() > 1 {
                self.run(if d.x < 0.0 {
                    Action::WorkspaceNext
                } else {
                    Action::WorkspacePrev
                });
            } else {
                // The previous window, as a quick Alt+Tab.
                self.switch_windows(false);
                self.finish_switcher();
            }
        } else if d.y.abs() > SWIPE {
            // Down hides the windows; up, with none shown, brings them back.
            let shown = self.space.elements().next().is_some();
            if (d.y > 0.0) == shown {
                self.show_desktop();
            }
        }
        self.backend.schedule_render();
    }

    /// Pinch and hold go to the application under the pointer.
    pub fn pinch(&mut self, event: Pinch) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        match event {
            Pinch::Begin { time, fingers } => {
                let serial = SERIAL_COUNTER.next_serial();
                let e = p::GesturePinchBeginEvent {
                    serial,
                    time,
                    fingers,
                };
                pointer.gesture_pinch_begin(self, &e);
            }
            Pinch::Update {
                time,
                delta,
                scale,
                rotation,
            } => {
                let e = p::GesturePinchUpdateEvent {
                    time,
                    delta,
                    scale,
                    rotation,
                };
                pointer.gesture_pinch_update(self, &e);
            }
            Pinch::End { time, cancelled } => {
                let serial = SERIAL_COUNTER.next_serial();
                let e = p::GesturePinchEndEvent {
                    serial,
                    time,
                    cancelled,
                };
                pointer.gesture_pinch_end(self, &e);
            }
            Pinch::HoldBegin { time, fingers } => {
                let serial = SERIAL_COUNTER.next_serial();
                let e = p::GestureHoldBeginEvent {
                    serial,
                    time,
                    fingers,
                };
                pointer.gesture_hold_begin(self, &e);
            }
            Pinch::HoldEnd { time, cancelled } => {
                let serial = SERIAL_COUNTER.next_serial();
                let e = p::GestureHoldEndEvent {
                    serial,
                    time,
                    cancelled,
                };
                pointer.gesture_hold_end(self, &e);
            }
        }
        pointer.frame(self);
    }

    /// The lid closed (`true`) or opened.
    pub fn lid(&mut self, closed: bool) {
        if closed {
            let others = self.space.outputs().any(|o| !built_in(o));
            if !others {
                return;
            }
            let panels: Vec<Output> = self
                .space
                .outputs()
                .filter(|o| built_in(o))
                .cloned()
                .collect();
            for o in panels {
                tracing::info!("lid closed: {} off", o.name());
                self.disable_output(&o);
                self.lid_off.push(o);
            }
        } else {
            for o in std::mem::take(&mut self.lid_off) {
                tracing::info!("lid opened: {} on", o.name());
                self.enable_output(&o);
            }
        }
        self.rescue_windows();
        self.backend.schedule_render();
    }

    /// Windows and the pointer left where no screen is move onto one.
    pub fn rescue_windows(&mut self) {
        let screens: Vec<Rectangle<i32, Logical>> = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .collect();
        let Some(&first) = screens.first() else {
            return;
        };
        if let Some(pointer) = self.seat.get_pointer() {
            let pos = pointer.current_location().to_i32_round();
            if !screens.iter().any(|s| s.contains(pos)) {
                let center = first.loc + first.size.downscale(2).to_point();
                pointer.set_location(center.to_f64());
            }
        }
        let lost: Vec<(Window, Rectangle<i32, Logical>)> = self
            .space
            .elements()
            .filter(|w| !w.x11_surface().is_some_and(|x| x.is_override_redirect()))
            .filter_map(|w| {
                use smithay::utils::IsAlive;
                let g = self.space.element_geometry(w)?;
                // Not yet drawn, or closing: nothing to move.
                let shown = w.alive() && g.size.w > 0 && g.size.h > 0;
                (shown && !screens.iter().any(|s| s.overlaps(g))).then(|| (w.clone(), g))
            })
            .collect();
        let area = self.work_area().unwrap_or(first);
        for (w, g) in lost {
            let full = xdg_toplevel::State::Fullscreen;
            let max = xdg_toplevel::State::Maximized;
            if Self::is_state(&w, full) {
                self.place(&w, first, Some(full));
                continue;
            }
            if Self::is_state(&w, max) {
                self.place(&w, area, Some(max));
                continue;
            }
            let size = g.size;
            let loc = Point::from((
                area.loc.x + ((area.size.w - size.w) / 2).max(0),
                area.loc.y + ((area.size.h - size.h) / 2).max(crate::deco::BAR),
            ));
            tracing::info!(x = loc.x, y = loc.y, "window moved onto a screen");
            if let Some(x) = w.x11_surface() {
                let _ = x.configure(Rectangle::new(loc, size));
            }
            self.space.map_element(w, loc, false);
        }
    }
}

/// A pinch or hold, from the touchpad.
pub enum Pinch {
    Begin {
        time: u32,
        fingers: u32,
    },
    Update {
        time: u32,
        delta: Point<f64, Logical>,
        scale: f64,
        rotation: f64,
    },
    End {
        time: u32,
        cancelled: bool,
    },
    HoldBegin {
        time: u32,
        fingers: u32,
    },
    HoldEnd {
        time: u32,
        cancelled: bool,
    },
}
