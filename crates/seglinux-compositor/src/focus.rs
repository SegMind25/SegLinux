//! What has the keyboard: a Wayland surface (a window, the Start menu) or
//! an X11 window, which also needs X's own input focus.

use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
    xwayland::X11Surface,
};

use crate::state::State;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocus {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl From<WlSurface> for KeyboardFocus {
    fn from(s: WlSurface) -> Self {
        KeyboardFocus::Wayland(s)
    }
}

impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        match self {
            KeyboardFocus::Wayland(s) => s.alive(),
            KeyboardFocus::X11(x) => x.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            KeyboardFocus::Wayland(s) => Some(Cow::Borrowed(s)),
            KeyboardFocus::X11(x) => x.wl_surface().map(Cow::Owned),
        }
    }
}

macro_rules! each {
    ($self:ident, $t:ident => $e:expr) => {
        match $self {
            KeyboardFocus::Wayland($t) => $e,
            KeyboardFocus::X11($t) => $e,
        }
    };
}

impl KeyboardTarget<State> for KeyboardFocus {
    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        each!(self, t => KeyboardTarget::enter(t, seat, data, keys, serial))
    }
    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        each!(self, t => KeyboardTarget::leave(t, seat, data, serial))
    }
    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        each!(self, t => KeyboardTarget::key(t, seat, data, key, state, serial, time))
    }
    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        each!(self, t => KeyboardTarget::modifiers(t, seat, data, modifiers, serial))
    }
}
