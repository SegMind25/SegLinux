//! The lock screen (ext-session-lock: swaylock, Super+L). While locked,
//! only the lock screen is drawn and gets input; no key combination works
//! but switching terminals. If the lock screen crashes, the screen stays
//! black and locked.

use smithay::{
    delegate_session_lock,
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::SERIAL_COUNTER,
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
};

use crate::state::State;

#[derive(Default)]
pub struct Lock {
    pub locked: bool,
    pub surfaces: Vec<(Output, LockSurface)>,
}

impl State {
    /// The lock screen surface shown on `output`, if any.
    pub fn lock_surface(&self, output: &Output) -> Option<&LockSurface> {
        self.lock
            .surfaces
            .iter()
            .find(|(o, s)| o == output && s.alive())
            .map(|(_, s)| s)
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        self.lock.locked = true;
        self.lock.surfaces.clear();
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        confirmation.lock();
        self.backend.schedule_render();
    }

    fn unlock(&mut self) {
        self.lock = Lock::default();
        let top = self.space.elements().last().cloned();
        self.focus(top);
    }

    fn new_surface(&mut self, surface: LockSurface, wl_output: WlOutput) {
        let Some(output) = Output::from_resource(&wl_output) else {
            return;
        };
        if let Some(mode) = output.current_mode() {
            let size = mode
                .size
                .to_f64()
                .to_logical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>();
            surface.with_pending_state(|s| s.size = Some((size.w as u32, size.h as u32).into()));
        }
        surface.send_configure();
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(
                self,
                Some(surface.wl_surface().clone().into()),
                SERIAL_COUNTER.next_serial(),
            );
        }
        self.lock.surfaces.push((output, surface));
        self.backend.schedule_render();
    }
}

delegate_session_lock!(State);
