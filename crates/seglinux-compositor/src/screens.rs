//! Screens off and adaptive sync:
//! - a screen's picture on or off (wlr-output-power-management: wlopm,
//!   swayidle); any key or mouse movement turns them back on
//! - adaptive sync (VRR: FreeSync, G-Sync Compatible): off, on, or only
//!   while a window is fullscreen there (games), the default
//! - idle (ext-idle-notify: swayidle) and what keeps a screen awake
//!   (idle-inhibit: video players, browsers playing a video)

use std::sync::{Mutex, MutexGuard};

use smithay::{
    delegate_idle_inhibit, delegate_idle_notify,
    output::Output,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    reexports::{
        wayland_protocols_wlr::output_power_management::v1::server::{
            zwlr_output_power_manager_v1::{self as manager, ZwlrOutputPowerManagerV1},
            zwlr_output_power_v1::{self as power, ZwlrOutputPowerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            protocol::wl_surface::WlSurface,
        },
    },
    wayland::{
        idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState},
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
    },
};

use crate::{config::AdaptiveSync, state::State};

/// One screen's settings, kept on its `Output`.
#[derive(Debug)]
pub struct Prefs {
    /// The picture is on (off: power management).
    pub power: bool,
    /// Adaptive sync chosen in the display settings for this screen;
    /// none: compositor.toml's `adaptive_sync`.
    pub adaptive_sync: Option<AdaptiveSync>,
}

pub fn prefs(output: &Output) -> MutexGuard<'_, Prefs> {
    output
        .user_data()
        .get_or_insert_threadsafe(|| {
            Mutex::new(Prefs {
                power: true,
                adaptive_sync: None,
            })
        })
        .lock()
        .unwrap()
}

pub struct ScreensState {
    powers: Vec<(ZwlrOutputPowerV1, Output)>,
    pub idle_notifier: IdleNotifierState<State>,
    _idle_inhibit: IdleInhibitManagerState,
    inhibitors: Vec<WlSurface>,
    /// A screen was turned off by power management.
    pub some_off: bool,
}

impl ScreensState {
    pub fn new(
        dh: &DisplayHandle,
        loop_handle: smithay::reexports::calloop::LoopHandle<'static, State>,
    ) -> Self {
        dh.create_global::<State, ZwlrOutputPowerManagerV1, ()>(1, ());
        ScreensState {
            powers: Vec::new(),
            idle_notifier: IdleNotifierState::new(dh, loop_handle),
            _idle_inhibit: IdleInhibitManagerState::new::<State>(dh),
            inhibitors: Vec::new(),
            some_off: false,
        }
    }
}

fn mode(on: bool) -> power::Mode {
    if on {
        power::Mode::On
    } else {
        power::Mode::Off
    }
}

impl State {
    /// Turn a screen's picture on or off.
    pub fn set_power(&mut self, output: &Output, on: bool) {
        if prefs(output).power == on {
            return;
        }
        // A screen turned off in the display settings stays off.
        let in_use = self.space.outputs().any(|o| o == output);
        if !in_use || !self.backend.set_power(output, on) {
            for (p, o) in &self.screens.powers {
                if o == output {
                    p.failed();
                }
            }
            return;
        }
        prefs(output).power = on;
        for (p, o) in &self.screens.powers {
            if o == output {
                p.mode(mode(on));
            }
        }
        self.screens.some_off = self.space.outputs().any(|o| !prefs(o).power);
        if on {
            self.backend.schedule_render();
        }
    }

    /// Input happened: screens wake up, idle clients hear of it.
    pub fn activity(&mut self) {
        if self.screens.some_off {
            let outputs: Vec<Output> = self.space.outputs().cloned().collect();
            for o in &outputs {
                self.set_power(o, true);
            }
        }
        let seat = self.seat.clone();
        self.screens.idle_notifier.notify_activity(&seat);
    }

    /// Whether adaptive sync should be on for this screen now.
    pub fn wants_adaptive_sync(&self, output: &Output) -> bool {
        match self.adaptive_sync_mode(output) {
            AdaptiveSync::Off => false,
            AdaptiveSync::On => true,
            AdaptiveSync::Fullscreen => self
                .space
                .elements_for_output(output)
                .any(|w| State::is_state(w, xdg_toplevel::State::Fullscreen)),
        }
    }

    /// Adaptive sync was asked for explicitly (may flicker once to turn on).
    pub fn adaptive_sync_forced(&self, output: &Output) -> bool {
        self.adaptive_sync_mode(output) == AdaptiveSync::On
    }

    /// The display settings' choice for this screen, else compositor.toml's,
    /// else the Settings app's.
    fn adaptive_sync_mode(&self, output: &Output) -> AdaptiveSync {
        prefs(output)
            .adaptive_sync
            .or(self.config.adaptive_sync)
            .unwrap_or_else(|| self.desktop.windows.adaptive_sync())
    }

    fn update_inhibit(&mut self) {
        self.screens.inhibitors.retain(|s| s.is_alive());
        let inhibited = !self.screens.inhibitors.is_empty();
        self.screens.idle_notifier.set_is_inhibited(inhibited);
    }
}

impl GlobalDispatch<ZwlrOutputPowerManagerV1, ()> for State {
    fn bind(
        _: &mut State,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, ()> for State {
    fn request(
        state: &mut State,
        _: &Client,
        _: &ZwlrOutputPowerManagerV1,
        request: manager::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let manager::Request::GetOutputPower { id, output } = request {
            let output = Output::from_resource(&output);
            let power = data_init.init(id, output.clone());
            match output {
                Some(o) => {
                    power.mode(mode(prefs(&o).power));
                    state.screens.powers.push((power, o));
                }
                None => power.failed(),
            }
        }
    }
}

impl Dispatch<ZwlrOutputPowerV1, Option<Output>> for State {
    fn request(
        state: &mut State,
        _: &Client,
        resource: &ZwlrOutputPowerV1,
        request: power::Request,
        output: &Option<Output>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
        match request {
            power::Request::SetMode { mode } => {
                let Some(output) = output else {
                    return;
                };
                if let smithay::reexports::wayland_server::WEnum::Value(m) = mode {
                    state.set_power(output, m == power::Mode::On);
                }
            }
            power::Request::Destroy => state.screens.powers.retain(|(p, _)| p != resource),
            _ => {}
        }
    }

    fn destroyed(
        state: &mut State,
        _: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ZwlrOutputPowerV1,
        _: &Option<Output>,
    ) {
        state.screens.powers.retain(|(p, _)| p != resource);
    }
}

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.screens.idle_notifier
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.screens.inhibitors.push(surface);
        self.update_inhibit();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.screens.inhibitors.retain(|s| s != &surface);
        self.update_inhibit();
    }
}

delegate_idle_notify!(State);
delegate_idle_inhibit!(State);
