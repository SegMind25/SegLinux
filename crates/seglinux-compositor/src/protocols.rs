//! The smaller protocols applications and games expect:
//! - locked and confined pointers with raw motion (relative-pointer,
//!   pointer-constraints): the mouse look of games, Wayland and X11 alike
//! - focus requests (xdg-activation): a link opened from a chat brings the
//!   browser forward
//! - fractional scales: sharp text at 125 % or 150 %
//! - exported windows (xdg-foreign): a portal's file dialog over its
//!   application

use std::time::Duration;

use smithay::{
    delegate_fractional_scale, delegate_pointer_constraints, delegate_relative_pointer,
    delegate_xdg_activation, delegate_xdg_foreign,
    desktop::Window,
    input::pointer::PointerHandle,
    output::Output,
    reexports::wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Transform},
    wayland::{
        compositor::{SurfaceData, send_surface_state},
        fractional_scale::{
            FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
        },
        pointer_constraints::{
            PointerConstraint, PointerConstraintsHandler, PointerConstraintsState,
            with_pointer_constraint,
        },
        relative_pointer::RelativePointerManagerState,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

use crate::state::State;

/// A focus request honoured this long after its token was made: a click
/// in one application, then the other window comes forward.
const ACTIVATION_WINDOW: Duration = Duration::from_secs(10);

pub struct Protocols {
    pub activation: XdgActivationState,
    pub foreign: XdgForeignState,
    _relative_pointer: RelativePointerManagerState,
    _pointer_constraints: PointerConstraintsState,
    _fractional_scale: FractionalScaleManagerState,
}

impl Protocols {
    pub fn new(dh: &DisplayHandle) -> Self {
        Protocols {
            activation: XdgActivationState::new::<State>(dh),
            foreign: XdgForeignState::new::<State>(dh),
            _relative_pointer: RelativePointerManagerState::new::<State>(dh),
            _pointer_constraints: PointerConstraintsState::new::<State>(dh),
            _fractional_scale: FractionalScaleManagerState::new::<State>(dh),
        }
    }
}

/// What a pointer constraint on the surface under the pointer does to motion.
pub enum Constraint {
    None,
    /// The pointer stays where it is; only raw motion goes out.
    Locked,
    /// The pointer stays inside the surface (the region, if any).
    Confined,
}

impl State {
    /// The active constraint of `surface` (at `origin`) for the pointer at `pos`.
    pub fn constraint(
        &self,
        pointer: &PointerHandle<State>,
        surface: &WlSurface,
        origin: Point<f64, Logical>,
        pos: Point<f64, Logical>,
    ) -> Constraint {
        with_pointer_constraint(surface, pointer, |c| match c {
            Some(c) if c.is_active() => {
                let inside = c
                    .region()
                    .is_none_or(|r| r.contains((pos - origin).to_i32_round()));
                match &*c {
                    _ if !inside => Constraint::None,
                    PointerConstraint::Locked(_) => Constraint::Locked,
                    PointerConstraint::Confined(_) => Constraint::Confined,
                }
            }
            _ => Constraint::None,
        })
    }

    /// Turn on the constraint the surface under the pointer asked for, once
    /// the pointer is inside it.
    pub fn activate_constraint(
        &self,
        pointer: &PointerHandle<State>,
        surface: &WlSurface,
        origin: Point<f64, Logical>,
    ) {
        let pos = pointer.current_location();
        with_pointer_constraint(surface, pointer, |c| {
            if let Some(c) = c
                && !c.is_active()
                && c.region()
                    .is_none_or(|r| r.contains((pos - origin).to_i32_round()))
            {
                c.activate();
            }
        });
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // A game locks the pointer while it has it: at once.
        if pointer.current_focus().as_ref() == Some(surface)
            && let Some((_, origin)) = self.surface_under(pointer.current_location())
        {
            self.activate_constraint(pointer, surface, origin);
        }
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        // Where the game drew its own cursor: the pointer reappears there.
        let locked = with_pointer_constraint(surface, pointer, |c| {
            c.is_some_and(|c| c.is_active() && matches!(&*c, PointerConstraint::Locked(_)))
        });
        if !locked {
            return;
        }
        if let Some((_, origin)) = self.surface_under(pointer.current_location()) {
            pointer.set_location(origin + location);
        }
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.protocols.activation
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed() < ACTIVATION_WINDOW
            && let Some(window) = self.window_for_surface(&surface)
        {
            self.show_hidden(&window);
            self.unminimize(&window);
            self.focus(Some(window));
            self.backend.schedule_render();
        }
        self.protocols.activation.remove_token(&token);
    }
}

impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.protocols.foreign
    }
}

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // The scale of the screen it is on (the first one until it is placed).
        let output = self
            .window_for_surface(&surface)
            .and_then(|w| self.space.outputs_for_element(&w).into_iter().next())
            .or_else(|| self.space.outputs().next().cloned());
        if let Some(output) = output {
            smithay::wayland::compositor::with_states(&surface, |data| {
                prefer_scale(&surface, data, &output)
            });
        }
    }
}

/// Tell a surface the scale of its screen (whole and fractional).
pub fn prefer_scale(surface: &WlSurface, data: &SurfaceData, output: &Output) {
    let scale = output.current_scale();
    // Screens are not rotated: buffers come upright.
    send_surface_state(surface, data, scale.integer_scale(), Transform::Normal);
    with_fractional_scale(data, |f| f.set_preferred_scale(scale.fractional_scale()));
}

/// Every surface of `window` gets the scale of `output`.
pub fn prefer_window_scale(window: &Window, output: &Output) {
    window.with_surfaces(|surface, data| prefer_scale(surface, data, output));
}

delegate_relative_pointer!(State);
delegate_pointer_constraints!(State);
delegate_xdg_activation!(State);
delegate_xdg_foreign!(State);
delegate_fractional_scale!(State);
