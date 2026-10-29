//! Keyboard and mouse: the configured key combinations first, everything
//! else to the focused application.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::{keysyms, FilterResult, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, Focus, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::{
    config::{normalize_key, Action, Combo, Mods},
    grabs::MoveSurfaceGrab,
    state::State,
};

const BTN_LEFT: u32 = 0x110;

fn combo(mods: &ModifiersState, sym: Keysym) -> Combo {
    Combo {
        mods: Mods {
            ctrl: mods.ctrl,
            alt: mods.alt,
            shift: mods.shift,
            logo: mods.logo,
        },
        key: normalize_key(&keysyms::keysym_get_name(sym)),
    }
}

impl State {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let pressed = event.state() == KeyState::Pressed;
                let Some(keyboard) = self.seat.get_keyboard() else { return };
                let binds = &self.config.binds;
                let action = keyboard.input::<Action, _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, mods, handle| {
                        if !pressed {
                            return FilterResult::Forward;
                        }
                        // The key as printed without Shift (Super+Shift+S is `s`),
                        // and the key as typed (XF86Switch_VT_1 from Ctrl+Alt+F1).
                        for sym in [handle.raw_latin_sym_or_raw_current_sym(), Some(handle.modified_sym())]
                            .into_iter()
                            .flatten()
                        {
                            if let Some(action) = binds.get(&combo(mods, sym)) {
                                return FilterResult::Intercept(action.clone());
                            }
                        }
                        FilterResult::Forward
                    },
                );
                if let Some(action) = action {
                    self.run(action);
                }
            }
            InputEvent::PointerMotion { event } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let pos = self.clamp(pointer.current_location() + event.delta());
                let under = self.surface_under(pos);
                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                pointer.relative_motion(
                    self,
                    under,
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );
                pointer.frame(self);
                self.after_motion(pos);
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(output) = self.space.outputs().next() else { return };
                let Some(geo) = self.space.output_geometry(output) else { return };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                let Some(pointer) = self.seat.get_pointer() else { return };
                let under = self.surface_under(pos);
                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
                self.after_motion(pos);
            }
            InputEvent::PointerButton { event } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let state = event.state();
                if state == ButtonState::Pressed && !pointer.is_grabbed() {
                    let pos = pointer.current_location();
                    let window = self.space.element_under(pos).map(|(w, _)| w.clone());
                    let logo = self
                        .seat
                        .get_keyboard()
                        .is_some_and(|k| k.modifier_state().logo);
                    match window {
                        // Super + drag moves a window from anywhere in it.
                        Some(window) if logo && button == BTN_LEFT => {
                            self.focus(Some(window.clone()));
                            if let Some(loc) = self.space.element_location(&window) {
                                let start_data = smithay::input::pointer::GrabStartData {
                                    focus: None,
                                    button,
                                    location: pos,
                                };
                                let grab = MoveSurfaceGrab {
                                    start_data,
                                    window,
                                    initial_window_location: loc,
                                };
                                pointer.set_grab(self, grab, serial, Focus::Clear);
                                return;
                            }
                        }
                        Some(window) => self.focus(Some(window)),
                        // A click on a panel (the dock) leaves the focus alone,
                        // unless the panel wants the keyboard itself.
                        None => {}
                    }
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
                self.backend.schedule_render();
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| {
                    event
                        .amount(axis)
                        .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
                };
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let value = amount(axis);
                    if value != 0.0 {
                        frame = frame.value(axis, value);
                        if let Some(discrete) = event.amount_v120(axis) {
                            frame = frame.v120(axis, discrete as i32);
                        }
                    }
                    if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    }
                }
                let Some(pointer) = self.seat.get_pointer() else { return };
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// Keep the pointer on the screens.
    fn clamp(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.space.output_under(pos).next().is_some() {
            return pos;
        }
        let Some(geo) = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .reduce(|a, b| a.merge(b))
        else {
            return pos;
        };
        let x = pos.x.clamp(geo.loc.x as f64, (geo.loc.x + geo.size.w - 1) as f64);
        let y = pos.y.clamp(geo.loc.y as f64, (geo.loc.y + geo.size.h - 1) as f64);
        (x, y).into()
    }

    fn after_motion(&mut self, pos: Point<f64, Logical>) {
        if self.config.focus_follows_mouse {
            let window = self.space.element_under(pos).map(|(w, _)| w.clone());
            if window.is_some() && window != self.focused_window() {
                self.focus(window);
            }
        }
        self.backend.schedule_render();
    }

    pub fn run(&mut self, action: Action) {
        let focused = self.focused_window();
        match action {
            Action::Spawn(command) => self.spawn(&command),
            Action::Close => {
                if let Some(t) = focused.as_ref().and_then(|w| w.toplevel()) {
                    t.send_close();
                }
            }
            Action::Maximize => {
                if let Some(w) = focused {
                    self.toggle_maximize(&w);
                }
            }
            Action::Fullscreen => {
                if let Some(w) = focused {
                    self.toggle_fullscreen(&w);
                }
            }
            Action::Minimize => {
                if let Some(w) = focused {
                    self.minimize(&w);
                }
            }
            Action::SnapLeft | Action::SnapRight => {
                if let Some(w) = focused {
                    self.snap(&w, action == Action::SnapLeft);
                }
            }
            Action::CycleWindows => self.cycle_windows(),
            Action::ShowDesktop => self.show_desktop(),
            Action::ReloadConfig => self.reload_config(),
            Action::Quit => self.loop_signal.stop(),
            Action::SwitchVt(vt) => self.backend.change_vt(vt),
        }
        self.backend.schedule_render();
    }
}
