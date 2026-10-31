//! Display settings (wlr-output-management v4: wdisplays, kanshi,
//! wlr-randr): the screens, their modes, place and scale go out; a new
//! resolution, refresh rate, scale or arrangement comes back and is applied.
//! A screen can be turned off (it leaves the arrangement; its windows move
//! to the others) and adaptive sync set per screen.

use std::sync::Mutex;

use smithay::{
    output::{Mode, Output, Scale},
    reexports::{
        wayland_protocols_wlr::output_management::v1::server::{
            zwlr_output_configuration_head_v1::{
                self as config_head, ZwlrOutputConfigurationHeadV1,
            },
            zwlr_output_configuration_v1::{self as configuration, ZwlrOutputConfigurationV1},
            zwlr_output_head_v1::{self as head, ZwlrOutputHeadV1},
            zwlr_output_manager_v1::{self as manager, ZwlrOutputManagerV1},
            zwlr_output_mode_v1::{self as mode, ZwlrOutputModeV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            protocol::wl_output::Transform as WlTransform,
        },
    },
    utils::{Logical, Point},
};

use crate::state::State;

/// What a client was told about one screen.
#[derive(Clone, PartialEq)]
struct Snapshot {
    output: Output,
    enabled: bool,
    /// None: the screen cannot; else whether it is on now.
    adaptive_sync: Option<bool>,
    modes: Vec<Mode>,
    current: Option<Mode>,
    position: Point<i32, Logical>,
    scale: f64,
}

struct Head {
    output: Output,
    head: ZwlrOutputHeadV1,
    modes: Vec<(ZwlrOutputModeV1, Mode)>,
}

struct Manager {
    manager: ZwlrOutputManagerV1,
    heads: Vec<Head>,
}

#[derive(Default)]
pub struct OutputManagementState {
    managers: Vec<Manager>,
    serial: u32,
    last: Vec<Snapshot>,
}

impl OutputManagementState {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<State, ZwlrOutputManagerV1, ()>(4, ());
        Self::default()
    }
}

/// A configuration's wish for one screen.
#[derive(Default)]
pub struct HeadWish {
    output: Option<Output>,
    mode: Option<Mode>,
    custom: Option<(i32, i32, i32)>,
    position: Option<Point<i32, Logical>>,
    scale: Option<f64>,
    adaptive_sync: Option<bool>,
}

/// Each screen a configuration names: on or off, and what it asks.
type Wishes = Vec<(Output, bool, std::sync::Arc<Mutex<HeadWish>>)>;

pub struct ConfigData {
    serial: u32,
    heads: Mutex<Wishes>,
}

fn snapshot(state: &State) -> Vec<Snapshot> {
    let on = state.space.outputs().map(|o| {
        let position = state
            .space
            .output_geometry(o)
            .map(|g| g.loc)
            .unwrap_or_default();
        (o, true, position)
    });
    let off = state.disabled_outputs.iter().map(|(o, p)| (o, false, *p));
    on.chain(off)
        .map(|(o, enabled, position)| Snapshot {
            output: o.clone(),
            enabled,
            adaptive_sync: state.backend.adaptive_sync(o),
            modes: state.backend.output_modes(o),
            current: o.current_mode(),
            position,
            scale: o.current_scale().fractional_scale(),
        })
        .collect()
}

fn announce(dh: &DisplayHandle, m: &mut Manager, s: &Snapshot) {
    let Some(client) = m.manager.client() else {
        return;
    };
    let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, Output, State>(
        dh,
        m.manager.version(),
        s.output.clone(),
    ) else {
        return;
    };
    m.manager.head(&head);
    head.name(s.output.name());
    head.description(s.output.description());
    let phys = s.output.physical_properties();
    if phys.size.w > 0 {
        head.physical_size(phys.size.w, phys.size.h);
    }
    let mut modes = Vec::new();
    for md in &s.modes {
        let Ok(res) =
            client.create_resource::<ZwlrOutputModeV1, Mode, State>(dh, head.version(), *md)
        else {
            continue;
        };
        head.mode(&res);
        res.size(md.size.w, md.size.h);
        res.refresh(md.refresh);
        if s.output.preferred_mode() == Some(*md) {
            res.preferred();
        }
        modes.push((res, *md));
    }
    send_state(&head, &modes, s);
    if head.version() >= 2 {
        head.make(phys.make.clone());
        head.model(phys.model.clone());
    }
    m.heads.push(Head {
        output: s.output.clone(),
        head,
        modes,
    });
}

fn send_state(head: &ZwlrOutputHeadV1, modes: &[(ZwlrOutputModeV1, Mode)], s: &Snapshot) {
    head.enabled(i32::from(s.enabled));
    if head.version() >= 4 {
        head.adaptive_sync(if s.adaptive_sync == Some(true) {
            head::AdaptiveSyncState::Enabled
        } else {
            head::AdaptiveSyncState::Disabled
        });
    }
    // An off screen has no mode, place or scale.
    if !s.enabled {
        return;
    }
    if let Some((res, _)) = modes.iter().find(|(_, m)| Some(*m) == s.current) {
        head.current_mode(res);
    }
    head.position(s.position.x, s.position.y);
    head.transform(WlTransform::Normal);
    head.scale(s.scale);
}

impl State {
    /// Tell display settings apps what changed (once per loop turn).
    pub fn refresh_output_management(&mut self) {
        let now = snapshot(self);
        if now == self.output_management.last {
            return;
        }
        let dh = self.display_handle.clone();
        let om = &mut self.output_management;
        om.serial = om.serial.wrapping_add(1);
        for m in &mut om.managers {
            m.heads.retain(|h| {
                let alive = now.iter().any(|s| s.output == h.output);
                if !alive {
                    for (res, _) in &h.modes {
                        res.finished();
                    }
                    h.head.finished();
                }
                alive
            });
            for s in &now {
                match m.heads.iter().find(|h| h.output == s.output) {
                    Some(h) => send_state(&h.head, &h.modes, s),
                    None => announce(&dh, m, s),
                }
            }
            m.manager.done(om.serial);
        }
        om.last = now;
    }

    /// Apply a configuration: true when every screen took its new settings.
    fn apply_outputs(
        &mut self,
        wishes: &[(Output, bool, std::sync::Arc<Mutex<HeadWish>>)],
        test: bool,
    ) -> bool {
        // At least one screen stays on.
        let all: Vec<Output> = self
            .space
            .outputs()
            .chain(self.disabled_outputs.iter().map(|(o, _)| o))
            .cloned()
            .collect();
        let stays_on = all
            .iter()
            .any(|o| match wishes.iter().find(|(w, _, _)| w == o) {
                Some((_, enabled, _)) => *enabled,
                None => self.space.outputs().any(|s| s == o),
            });
        if !stays_on {
            return false;
        }
        for (output, enabled, wish) in wishes {
            let is_on = self.space.outputs().any(|o| o == output);
            if !enabled {
                if !test && is_on {
                    let pos = self
                        .space
                        .output_geometry(output)
                        .map(|g| g.loc)
                        .unwrap_or_default();
                    self.space.unmap_output(output);
                    self.disabled_outputs.push((output.clone(), pos));
                    crate::screens::prefs(output).power = false;
                    self.backend.set_power(output, false);
                }
                continue;
            }
            if !test && !is_on {
                let pos = self
                    .disabled_outputs
                    .iter()
                    .find(|(o, _)| o == output)
                    .map(|(_, p)| *p)
                    .unwrap_or_default();
                self.disabled_outputs.retain(|(o, _)| o != output);
                self.space.map_output(output, pos);
                crate::screens::prefs(output).power = false;
                self.set_power(output, true);
            }
            let wish = wish.lock().unwrap();
            let target = wish.mode.or_else(|| {
                wish.custom.and_then(|(w, h, r)| {
                    self.backend.output_modes(output).into_iter().find(|m| {
                        m.size.w == w && m.size.h == h && (r == 0 || (m.refresh - r).abs() < 500)
                    })
                })
            });
            if let Some(mode) = target {
                if !self.backend.output_modes(output).contains(&mode) {
                    return false;
                }
                if !test
                    && output.current_mode() != Some(mode)
                    && !self.backend.set_mode(output, mode)
                {
                    return false;
                }
            }
            if let Some(scale) = wish.scale
                && !(0.5..=4.0).contains(&scale)
            {
                return false;
            }
            // Adaptive sync only where the screen has it.
            if wish.adaptive_sync == Some(true) && self.backend.adaptive_sync(output).is_none() {
                return false;
            }
            if test {
                continue;
            }
            if let Some(scale) = wish.scale {
                output.change_current_state(None, None, Some(Scale::Fractional(scale)), None);
            }
            if let Some(pos) = wish.position {
                output.change_current_state(None, None, None, Some(pos));
                self.space.map_output(output, pos);
            }
            if let Some(on) = wish.adaptive_sync {
                use crate::config::AdaptiveSync;
                crate::screens::prefs(output).adaptive_sync = Some(if on {
                    AdaptiveSync::On
                } else {
                    AdaptiveSync::Off
                });
            }
        }
        if !test {
            self.arrange_layers();
            self.backend.schedule_render();
        }
        true
    }
}

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for State {
    fn bind(
        state: &mut State,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ());
        let mut m = Manager {
            manager,
            heads: Vec::new(),
        };
        let now = snapshot(state);
        for s in &now {
            announce(dh, &mut m, s);
        }
        m.manager.done(state.output_management.serial);
        state.output_management.managers.push(m);
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        resource: &ZwlrOutputManagerV1,
        request: manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::CreateConfiguration { id, serial } => {
                data_init.init(
                    id,
                    ConfigData {
                        serial,
                        heads: Mutex::new(Vec::new()),
                    },
                );
            }
            manager::Request::Stop => {
                resource.finished();
                state
                    .output_management
                    .managers
                    .retain(|m| &m.manager != resource);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputHeadV1, Output> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ZwlrOutputHeadV1,
        _: head::Request,
        _: &Output,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, Mode> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ZwlrOutputModeV1,
        _: mode::Request,
        _: &Mode,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ConfigData> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: configuration::Request,
        data: &ConfigData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            configuration::Request::EnableHead { id, head } => {
                let wish = std::sync::Arc::new(Mutex::new(HeadWish::default()));
                if let Some(output) = head.data::<Output>() {
                    wish.lock().unwrap().output = Some(output.clone());
                    data.heads
                        .lock()
                        .unwrap()
                        .push((output.clone(), true, wish.clone()));
                }
                data_init.init(id, wish);
            }
            configuration::Request::DisableHead { head } => {
                if let Some(output) = head.data::<Output>() {
                    data.heads
                        .lock()
                        .unwrap()
                        .push((output.clone(), false, Default::default()));
                }
            }
            configuration::Request::Apply | configuration::Request::Test => {
                let test = matches!(request, configuration::Request::Test);
                if data.serial != state.output_management.serial {
                    resource.cancelled();
                    return;
                }
                let wishes = std::mem::take(&mut *data.heads.lock().unwrap());
                if state.apply_outputs(&wishes, test) {
                    resource.succeeded();
                } else {
                    resource.failed();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, std::sync::Arc<Mutex<HeadWish>>> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &ZwlrOutputConfigurationHeadV1,
        request: config_head::Request,
        wish: &std::sync::Arc<Mutex<HeadWish>>,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let mut w = wish.lock().unwrap();
        match request {
            config_head::Request::SetMode { mode } => w.mode = mode.data::<Mode>().copied(),
            config_head::Request::SetCustomMode {
                width,
                height,
                refresh,
            } => w.custom = Some((width, height, refresh)),
            config_head::Request::SetPosition { x, y } => w.position = Some((x, y).into()),
            config_head::Request::SetScale { scale } => w.scale = Some(scale),
            config_head::Request::SetAdaptiveSync { state } => {
                w.adaptive_sync = match state {
                    smithay::reexports::wayland_server::WEnum::Value(
                        head::AdaptiveSyncState::Enabled,
                    ) => Some(true),
                    _ => Some(false),
                };
            }
            _ => {}
        }
        let _ = &w.output;
    }
}
