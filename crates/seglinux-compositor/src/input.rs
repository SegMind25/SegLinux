//! Keyboard and mouse: the configured key combinations first, everything
//! else to the focused application.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        TouchEvent,
    },
    input::{
        keyboard::{FilterResult, Keysym, ModifiersState, xkb},
        pointer::{AxisFrame, ButtonEvent, Focus, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

use crate::{
    config::{Action, Combo, Mods, normalize_key},
    deco::Hit,
    grabs::MoveSurfaceGrab,
    protocols::Constraint,
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
        key: normalize_key(&xkb::keysym_get_name(sym)),
    }
}

impl State {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        if !matches!(
            event,
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }
        ) {
            self.activity();
        }
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let pressed = event.state() == KeyState::Pressed;
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };
                let binds = self.config.binds.clone();
                let locked = self.lock.locked;
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
                        for sym in [
                            handle.raw_latin_sym_or_raw_current_sym(),
                            Some(handle.modified_sym()),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            if let Some(action) = binds.get(&combo(mods, sym)) {
                                // Locked: only switching terminals.
                                if locked && !matches!(action, Action::SwitchVt(_)) {
                                    continue;
                                }
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
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let old = pointer.current_location();
                let focus = pointer
                    .current_focus()
                    .and_then(|s| self.surface_under(old).filter(|(u, _)| *u == s));
                let constraint = focus.as_ref().map_or(Constraint::None, |(s, origin)| {
                    self.constraint(&pointer, s, *origin, old)
                });
                let relative = RelativeMotionEvent {
                    delta: event.delta(),
                    delta_unaccel: event.delta_unaccel(),
                    utime: event.time(),
                };
                // A game holding the pointer: raw motion only, the pointer stays.
                if matches!(constraint, Constraint::Locked) {
                    pointer.relative_motion(self, focus, &relative);
                    pointer.frame(self);
                    return;
                }
                let before = pointer.current_focus();
                let mut pos = self.clamp(old + event.delta());
                let mut under = self.surface_under(pos);
                // Confined: it does not leave the surface (or its region).
                if let (Constraint::Confined, Some((s, origin))) = (&constraint, &focus) {
                    let same = under.as_ref().is_some_and(|(u, _)| u == s);
                    if !same
                        || matches!(self.constraint(&pointer, s, *origin, pos), Constraint::None)
                    {
                        pos = old;
                        under = focus.clone();
                    }
                }
                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                pointer.relative_motion(self, under.clone(), &relative);
                pointer.frame(self);
                if let Some((s, origin)) = &under {
                    self.activate_constraint(&pointer, s, *origin);
                }
                if !pointer.is_grabbed() {
                    self.entered(&before, under.as_ref().map(|(s, _)| s));
                    self.update_cursor(pos, under.is_some());
                }
                self.after_motion(pos);
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(geo) = self.space.output_geometry(output) else {
                    return;
                };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let before = pointer.current_focus();
                let under = self.surface_under(pos);
                let on_surface = under.is_some();
                let now = under.as_ref().map(|(s, _)| s.clone());
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
                if !pointer.is_grabbed() {
                    self.entered(&before, now.as_ref());
                    self.update_cursor(pos, on_surface);
                }
                self.after_motion(pos);
            }
            InputEvent::PointerButton { event } => {
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let state = event.state();
                if state == ButtonState::Pressed && !pointer.is_grabbed() && !self.lock.locked {
                    let pos = pointer.current_location();
                    // The border around a window: resize it from there.
                    if button == BTN_LEFT
                        && let Some((window, edges)) = self.border_under(pos)
                    {
                        tracing::info!(?edges, "resizing from the border");
                        self.focus(Some(window.clone()));
                        if let Some(loc) = self.space.element_location(&window) {
                            let start_data = smithay::input::pointer::GrabStartData {
                                focus: None,
                                button,
                                location: pos,
                            };
                            let rect = smithay::utils::Rectangle::new(loc, window.geometry().size);
                            crate::snap::set_snapped(&window, false);
                            self.cursor_status = smithay::input::pointer::CursorImageStatus::Named(
                                crate::snap::edge_icon(edges),
                            );
                            let grab = crate::grabs::ResizeSurfaceGrab::start(
                                start_data, window, edges, rect,
                            );
                            pointer.set_grab(self, grab, serial, Focus::Clear);
                        }
                        self.backend.schedule_render();
                        return;
                    }
                    // A title bar: its buttons, or dragging it moves the window.
                    if button == BTN_LEFT
                        && let Some((window, hit)) = self.deco_under(pos)
                    {
                        self.focus(Some(window.clone()));
                        match hit {
                            Hit::Close => {
                                State::close(&window);
                            }
                            Hit::Maximize => self.toggle_maximize(&window),
                            Hit::Minimize => self.minimize(&window),
                            // A double click maximizes, or gives the size back.
                            Hit::Bar if self.is_double_click(&window) => {
                                self.toggle_maximize(&window);
                            }
                            Hit::Bar => {
                                let snapped = crate::snap::is_snapped(&window)
                                    || State::is_state(&window, xdg_toplevel::State::Maximized);
                                // Dragged off the edge: its own size again
                                // (only once it really moves, see the grab).
                                if let Some(loc) = self.space.element_location(&window) {
                                    self.cursor_status =
                                        smithay::input::pointer::CursorImageStatus::Named(
                                            smithay::input::pointer::CursorIcon::Grabbing,
                                        );
                                    let grab = MoveSurfaceGrab {
                                        start_data: smithay::input::pointer::GrabStartData {
                                            focus: None,
                                            button,
                                            location: pos,
                                        },
                                        window,
                                        initial_window_location: loc,
                                        unsnap: snapped,
                                    };
                                    pointer.set_grab(self, grab, serial, Focus::Clear);
                                }
                            }
                        }
                        self.backend.schedule_render();
                        return;
                    }
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
                                let snapped = crate::snap::is_snapped(&window)
                                    || State::is_state(&window, xdg_toplevel::State::Maximized);
                                let grab = MoveSurfaceGrab {
                                    start_data,
                                    window,
                                    initial_window_location: loc,
                                    unsnap: snapped,
                                };
                                pointer.set_grab(self, grab, serial, Focus::Clear);
                                return;
                            }
                        }
                        Some(window) => self.focus(Some(window)),
                        // A click on a panel (the dock) leaves the focus alone,
                        // unless the panel takes the keyboard (a search field).
                        None => self.focus_layer_at(pos),
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
            InputEvent::TouchDown { event } => {
                let Some(touch) = self.seat.get_touch() else {
                    return;
                };
                let Some(pos) = self.touch_position(&event) else {
                    return;
                };
                if !self.lock.locked {
                    // A title bar's buttons answer a tap; the window comes forward.
                    if let Some((window, hit)) = self.deco_under(pos) {
                        self.focus(Some(window.clone()));
                        match hit {
                            Hit::Close => State::close(&window),
                            Hit::Maximize => self.toggle_maximize(&window),
                            Hit::Minimize => self.minimize(&window),
                            Hit::Bar => {}
                        }
                        self.backend.schedule_render();
                        return;
                    }
                    match self.space.element_under(pos).map(|(w, _)| w.clone()) {
                        Some(window) => self.focus(Some(window)),
                        None => self.focus_layer_at(pos),
                    }
                }
                let under = self.surface_under(pos);
                touch.down(
                    self,
                    under,
                    &smithay::input::touch::DownEvent {
                        slot: event.slot(),
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                self.backend.schedule_render();
            }
            InputEvent::TouchMotion { event } => {
                let Some(touch) = self.seat.get_touch() else {
                    return;
                };
                let Some(pos) = self.touch_position(&event) else {
                    return;
                };
                let under = self.surface_under(pos);
                touch.motion(
                    self,
                    under,
                    &smithay::input::touch::MotionEvent {
                        slot: event.slot(),
                        location: pos,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchUp { event } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.up(
                        self,
                        &smithay::input::touch::UpEvent {
                            slot: event.slot(),
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                        },
                    );
                }
            }
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.frame(self);
                }
            }
            InputEvent::TouchCancel { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.cancel(self);
                }
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
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// Give the keyboard to the panel under `pos` if it accepts it.
    fn focus_layer_at(&mut self, pos: Point<f64, Logical>) {
        use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
        use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
        let Some(output) = self.space.output_under(pos).next().cloned() else {
            return;
        };
        let Some(geo) = self.space.output_geometry(&output) else {
            return;
        };
        let surface = {
            let map = layer_map_for_output(&output);
            let local = pos - geo.loc.to_f64();
            [
                smithay::wayland::shell::wlr_layer::Layer::Overlay,
                smithay::wayland::shell::wlr_layer::Layer::Top,
                smithay::wayland::shell::wlr_layer::Layer::Bottom,
                smithay::wayland::shell::wlr_layer::Layer::Background,
            ]
            .into_iter()
            .find_map(|l| map.layer_under(l, local))
            .filter(|l| l.cached_state().keyboard_interactivity != KeyboardInteractivity::None)
            .map(|l| l.wl_surface().clone())
        };
        let _ = WindowSurfaceType::TOPLEVEL;
        if let (Some(surface), Some(k)) = (surface, self.seat.get_keyboard()) {
            k.set_focus(self, Some(surface.into()), SERIAL_COUNTER.next_serial());
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
        let x = pos
            .x
            .clamp(geo.loc.x as f64, (geo.loc.x + geo.size.w - 1) as f64);
        let y = pos
            .y
            .clamp(geo.loc.y as f64, (geo.loc.y + geo.size.h - 1) as f64);
        (x, y).into()
    }

    /// Where a touch lands: on the built-in screen (a laptop's or tablet's),
    /// else the first one.
    fn touch_position<E: AbsolutePositionEvent<I>, I: InputBackend>(
        &self,
        event: &E,
    ) -> Option<Point<f64, Logical>> {
        let output = self
            .space
            .outputs()
            .find(|o| {
                let name = o.name();
                name.starts_with("eDP") || name.starts_with("LVDS") || name.starts_with("DSI")
            })
            .or_else(|| self.space.outputs().next())?;
        let geo = self.space.output_geometry(output)?;
        Some(event.position_transformed(geo.size) + geo.loc.to_f64())
    }

    fn after_motion(&mut self, pos: Point<f64, Logical>) {
        if self.focus_follows_mouse() {
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
                if let Some(w) = &focused {
                    State::close(w);
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
            Action::WorkspacePrev | Action::WorkspaceNext | Action::SendPrev | Action::SendNext => {
                let n = self.workspace_count();
                let target = match action {
                    Action::WorkspacePrev | Action::SendPrev => (self.workspace + n - 1) % n,
                    _ => (self.workspace + 1) % n,
                };
                if matches!(action, Action::SendPrev | Action::SendNext) {
                    self.send_to_workspace(target);
                } else {
                    self.switch_workspace(target);
                }
            }
        }
        self.backend.schedule_render();
    }
}
