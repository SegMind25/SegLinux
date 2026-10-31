use crate::state::State;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<State>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
    /// Maximized or snapped: it gets its own size back once it moves.
    pub unsnap: bool,
}

/// How far the pointer moves before a maximized window comes loose.
const LOOSEN: f64 = 8.0;

impl PointerGrab<State> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);
        if self.unsnap {
            let moved = event.location - self.start_data.location;
            if moved.x.hypot(moved.y) < LOOSEN {
                return;
            }
            self.unsnap = false;
            if let Some(loc) = data.unsnap_under(&self.window, event.location) {
                self.initial_window_location = loc;
                self.start_data.location = event.location;
            }
        }
        // Where it would snap, shown while dragging.
        let preview = data.snap_at(event.location).map(|(_, r)| r);
        if preview != data.snap_preview {
            data.snap_preview = preview;
            data.backend.schedule_render();
        }

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        let mut new_location = new_location.to_i32_round::<i32>();
        // The title bar stays on the screen, where it can be caught again.
        if let Some(screen) = data
            .space
            .output_under(event.location)
            .next()
            .and_then(|o| data.space.output_geometry(o))
        {
            let bar = if crate::deco::has_bar(&self.window) {
                crate::deco::BAR
            } else {
                0
            };
            new_location.y = new_location.y.max(screen.loc.y + bar);
        }
        if let Some(x) = self.window.x11_surface() {
            // X11 applications place their menus from their own position.
            let _ = x.configure(smithay::utils::Rectangle::new(
                new_location,
                x.geometry().size,
            ));
        }
        data.space
            .map_element(self.window.clone(), new_location, true);
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed, release the grab.
            let at = handle.current_location();
            handle.unset_grab(self, data, event.serial, event.time, true);
            data.snap_preview = None;
            data.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
            // Let go at an edge or a corner: snapped there, once the pointer
            // is free again (maximizing asks where the pointer is).
            if let Some((snap, rect)) = data.snap_at(at) {
                // Its size back puts it where the drag began.
                State::remember(
                    &self.window,
                    smithay::utils::Rectangle::new(
                        self.initial_window_location,
                        self.window.geometry().size,
                    ),
                );
                let window = self.window.clone();
                data.loop_handle
                    .insert_idle(move |state| state.apply_snap(&window, snap, rect));
            } else if !self.unsnap {
                crate::snap::set_snapped(&self.window, false);
            }
            data.backend.schedule_render();
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<State> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut State) {}
}
