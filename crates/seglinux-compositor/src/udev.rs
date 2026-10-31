//! Running on the computer's own screens, keyboard and mouse: the session
//! (libseat) opens the GPU and input devices, DRM/KMS shows the frames.
//! One GPU, the one the firmware boots with; every screen connected to it.

use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc, time::Duration};

use smithay::{
    backend::{
        allocator::{
            Fourcc,
            dmabuf::Dmabuf,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{ImportDma, ImportMemWl, gles::GlesRenderer},
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, primary_gpu},
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, LoopHandle,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Device as ControlDevice, ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::DeviceFd,
    wayland::dmabuf::DmabufFeedbackBuilder,
};

use crate::{
    render::{OutputElement, PointerImage, output_elements, send_frames},
    state::{Backend, State},
};

type Manager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;
type Element = OutputElement<GlesRenderer>;

struct Screen {
    output: Output,
    global: GlobalId,
    drm_output:
        DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>,
    /// A frame is on its way to the screen: wait for its vblank.
    queued: bool,
    /// Something changed: draw again at the next chance.
    dirty: bool,
    /// Every mode the screen offers.
    modes: Vec<smithay::reexports::drm::control::Mode>,
}

struct Udev {
    session: LibSeatSession,
    renderer: GlesRenderer,
    manager: Manager,
    screens: HashMap<crtc::Handle, Screen>,
    pointer: PointerImage,
    idle_queued: bool,
    active: bool,
    /// Mice and touchpads, to apply the Settings app's choices to.
    devices: Vec<smithay::reexports::input::Device>,
}

/// The Settings app's mouse and touchpad choices on one device.
fn configure(device: &mut smithay::reexports::input::Device, input: &crate::desktop::Input) {
    use smithay::reexports::input::AccelProfile;
    let touchpad = device.config_tap_finger_count() > 0;
    if touchpad {
        let _ = device.config_tap_set_enabled(input.tap_to_click);
        let _ = device.config_dwt_set_enabled(input.disable_touchpad_while_typing);
    }
    if device.config_scroll_has_natural_scroll() {
        let natural = if touchpad {
            input.natural_scroll_touchpad
        } else {
            input.natural_scroll_mouse
        };
        let _ = device.config_scroll_set_natural_scroll_enabled(natural);
    }
    if device.config_left_handed_is_available() {
        let _ = device.config_left_handed_set(input.left_handed);
    }
    if device.config_accel_is_available() {
        let _ =
            device.config_accel_set_speed(f64::from(input.pointer_speed.clamp(-100, 100)) / 100.0);
        let profile = if input.mouse_acceleration {
            AccelProfile::Adaptive
        } else {
            AccelProfile::Flat
        };
        let _ = device.config_accel_set_profile(profile);
    }
}

#[derive(Clone)]
struct Handle(Rc<RefCell<Udev>>, LoopHandle<'static, State>);

impl Backend for Handle {
    fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.0.borrow_mut().session.change_vt(vt) {
            tracing::warn!("cannot switch to terminal {vt}: {e}");
        }
    }

    fn schedule_render(&mut self) {
        let mut udev = self.0.borrow_mut();
        for screen in udev.screens.values_mut() {
            screen.dirty = true;
        }
        if !udev.idle_queued {
            udev.idle_queued = true;
            let handle = self.clone();
            self.1.insert_idle(move |state| {
                handle.0.borrow_mut().idle_queued = false;
                handle.render_all(state);
            });
        }
    }

    fn output_modes(&self, output: &Output) -> Vec<WlMode> {
        let udev = self.0.borrow();
        let mut modes: Vec<WlMode> = udev
            .screens
            .values()
            .find(|s| &s.output == output)
            .map(|s| s.modes.iter().map(|m| WlMode::from(*m)).collect())
            .unwrap_or_default();
        modes.dedup();
        modes
    }

    fn set_mode(&mut self, output: &Output, mode: WlMode) -> bool {
        let mut guard = self.0.borrow_mut();
        let udev = &mut *guard;
        let Some((crtc, drm_mode)) = udev.screens.iter().find_map(|(c, s)| {
            (&s.output == output)
                .then(|| {
                    s.modes
                        .iter()
                        .find(|m| WlMode::from(**m) == mode)
                        .map(|m| (*c, *m))
                })
                .flatten()
        }) else {
            return false;
        };
        match udev.manager.use_mode::<_, Element>(
            &crtc,
            drm_mode,
            &mut udev.renderer,
            &DrmOutputRenderElements::default(),
        ) {
            Ok(()) => {
                output.change_current_state(Some(mode), None, None, None);
                if let Some(s) = udev.screens.get_mut(&crtc) {
                    s.dirty = true;
                }
                tracing::info!(
                    "{}: {}x{} at {} Hz",
                    output.name(),
                    mode.size.w,
                    mode.size.h,
                    mode.refresh / 1000
                );
                true
            }
            Err(e) => {
                tracing::warn!("{}: cannot use that mode: {e}", output.name());
                false
            }
        }
    }

    fn configure_input(&mut self, input: &crate::desktop::Input) {
        for device in self.0.borrow_mut().devices.iter_mut() {
            configure(device, input);
        }
    }

    fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.0
            .borrow_mut()
            .renderer
            .import_dmabuf(dmabuf, None)
            .is_ok()
    }
}

impl Handle {
    fn render_all(&self, state: &mut State) {
        let crtcs: Vec<crtc::Handle> = self.0.borrow().screens.keys().copied().collect();
        for crtc in crtcs {
            self.render(state, crtc);
        }
    }

    fn render(&self, state: &mut State, crtc: crtc::Handle) {
        let mut guard = self.0.borrow_mut();
        let udev = &mut *guard;
        if !udev.active {
            return;
        }
        let Some(screen) = udev.screens.get_mut(&crtc) else {
            return;
        };
        if screen.queued || !screen.dirty {
            return;
        }
        screen.dirty = false;
        let output = screen.output.clone();
        crate::capture::fulfill(state, &mut udev.renderer, &output, &mut udev.pointer);
        let elements: Vec<Element> =
            output_elements(state, &mut udev.renderer, &output, &mut udev.pointer);
        let screen = udev.screens.get_mut(&crtc).expect("screen");
        match screen.drm_output.render_frame(
            &mut udev.renderer,
            &elements,
            if state.lock.locked {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                state.config.background
            },
            FrameFlags::DEFAULT,
        ) {
            Ok(result) if !result.is_empty => match screen.drm_output.queue_frame(()) {
                Ok(()) => screen.queued = true,
                Err(e) => tracing::warn!("cannot show a frame: {e}"),
            },
            Ok(_) => {
                // Nothing changed on this screen: let the applications
                // draw again after about one refresh.
                let refresh = output
                    .current_mode()
                    .map(|m| m.refresh)
                    .unwrap_or(60_000)
                    .max(1);
                let delay = Duration::from_micros(1_000_000_000 / refresh as u64);
                let _ = self
                    .1
                    .insert_source(Timer::from_duration(delay), move |_, _, state| {
                        send_frames(state, &output);
                        TimeoutAction::Drop
                    });
                return;
            }
            Err(e) => tracing::warn!("cannot draw a frame: {e:?}"),
        }
        drop(guard);
        send_frames(state, &output);
    }

    fn vblank(&self, state: &mut State, crtc: crtc::Handle) {
        {
            let mut udev = self.0.borrow_mut();
            let Some(screen) = udev.screens.get_mut(&crtc) else {
                return;
            };
            if let Err(e) = screen.drm_output.frame_submitted() {
                tracing::warn!("frame not shown: {e}");
            }
            screen.queued = false;
        }
        self.render(state, crtc);
    }

    /// The screens connected now: new ones are set up, unplugged ones go.
    fn scan(&self, state: &mut State) {
        let mut guard = self.0.borrow_mut();
        let udev = &mut *guard;
        let device = udev.manager.device();
        let Ok(resources) = device.resource_handles() else {
            return;
        };
        let mut connected: Vec<(connector::Info, crtc::Handle)> = Vec::new();
        let mut taken: Vec<crtc::Handle> = udev.screens.keys().copied().collect();
        for handle in resources.connectors() {
            let Ok(info) = device.get_connector(*handle, true) else {
                continue;
            };
            if info.state() != connector::State::Connected || info.modes().is_empty() {
                continue;
            }
            // The CRTC it already drives, else a free one its encoders allow.
            let current = udev
                .screens
                .iter()
                .find(|(_, s)| s.output.name() == connector_name(&info))
                .map(|(c, _)| *c);
            let crtc = current.or_else(|| {
                info.encoders().iter().find_map(|enc| {
                    let enc = device.get_encoder(*enc).ok()?;
                    resources
                        .filter_crtcs(enc.possible_crtcs())
                        .into_iter()
                        .find(|c| !taken.contains(c))
                })
            });
            if let Some(crtc) = crtc {
                taken.push(crtc);
                connected.push((info, crtc));
            }
        }

        // Unplugged.
        let gone: Vec<crtc::Handle> = udev
            .screens
            .keys()
            .filter(|c| !connected.iter().any(|(_, k)| k == *c))
            .copied()
            .collect();
        for crtc in gone {
            if let Some(screen) = udev.screens.remove(&crtc) {
                state.space.unmap_output(&screen.output);
                state.display_handle.remove_global::<State>(screen.global);
            }
        }

        // Plugged in: side by side, left to right.
        for (info, crtc) in connected {
            if udev.screens.contains_key(&crtc) {
                continue;
            }
            let mode = info
                .modes()
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .copied()
                .unwrap_or(info.modes()[0]);
            let (w, h) = info.size().unwrap_or((0, 0));
            let output = Output::new(
                connector_name(&info),
                PhysicalProperties {
                    size: (w as i32, h as i32).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Unknown".into(),
                    model: "Unknown".into(),
                },
            );
            let global = output.create_global::<State>(&state.display_handle);
            let x = state
                .space
                .outputs()
                .filter_map(|o| state.space.output_geometry(o))
                .map(|g| g.loc.x + g.size.w)
                .max()
                .unwrap_or(0);
            let wl_mode = WlMode::from(mode);
            output.set_preferred(wl_mode);
            output.change_current_state(Some(wl_mode), None, None, Some((x, 0).into()));
            state.space.map_output(&output, (x, 0));
            let drm_output = match udev.manager.initialize_output::<_, Element>(
                crtc,
                mode,
                &[info.handle()],
                &output,
                None,
                &mut udev.renderer,
                &DrmOutputRenderElements::default(),
            ) {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!("cannot use screen {}: {e}", output.name());
                    state.space.unmap_output(&output);
                    state.display_handle.remove_global::<State>(global);
                    continue;
                }
            };
            tracing::info!(
                "screen {} at {}x{}",
                output.name(),
                wl_mode.size.w,
                wl_mode.size.h
            );
            udev.screens.insert(
                crtc,
                Screen {
                    output,
                    global,
                    drm_output,
                    queued: false,
                    dirty: true,
                    modes: info.modes().to_vec(),
                },
            );
        }
        drop(guard);
        state.arrange_layers();
        let mut handle = self.clone();
        handle.schedule_render();
    }
}

fn connector_name(info: &connector::Info) -> String {
    format!("{}-{}", info.interface().as_str(), info.interface_id())
}

pub fn init(
    event_loop: &mut EventLoop<'static, State>,
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|e| format!("no seat session (is seatd or elogind running?): {e}"))?;
    let seat_name = session.seat();

    let gpu_path = primary_gpu(&seat_name)?.ok_or("no graphics card found")?;
    tracing::info!("graphics card: {}", gpu_path.display());
    let fd = session.open(
        &gpu_path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let node = DrmNode::from_file(fd.clone())?;
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let renderer = unsafe { GlesRenderer::new(context)? };
    let render_formats: FormatSet = renderer.egl_context().dmabuf_render_formats().clone();
    let manager = DrmOutputManager::new(
        drm,
        GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        ),
        GbmFramebufferExporter::new(gbm.clone(), Some(node)),
        Some(gbm),
        [Fourcc::Abgr8888, Fourcc::Argb8888],
        render_formats,
    );

    // Buffers in GPU memory: games and every OpenGL or Vulkan application.
    let feedback = DmabufFeedbackBuilder::new(node.dev_id(), renderer.dmabuf_formats()).build()?;
    state.dmabuf_global = Some(
        state
            .dmabuf_state
            .create_global_with_default_feedback::<State>(&state.display_handle, &feedback),
    );
    state.shm_state.update_formats(renderer.shm_formats());

    let handle = Handle(
        Rc::new(RefCell::new(Udev {
            session: session.clone(),
            renderer,
            manager,
            screens: HashMap::new(),
            pointer: PointerImage::new(),
            idle_queued: false,
            active: true,
            devices: Vec::new(),
        })),
        event_loop.handle(),
    );
    state.backend = Box::new(handle.clone());

    let h = handle.clone();
    event_loop
        .handle()
        .insert_source(drm_notifier, move |event, _, state| match event {
            DrmEvent::VBlank(crtc) => h.vblank(state, crtc),
            DrmEvent::Error(e) => tracing::warn!("graphics card: {e}"),
        })?;

    // Keyboards, mice and touchpads.
    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "cannot read the input devices")?;
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput.clone()), {
            let h = handle.clone();
            move |mut event, _, state| {
                use smithay::backend::input::InputEvent;
                match &mut event {
                    InputEvent::DeviceAdded { device } => {
                        configure(device, &state.desktop.input);
                        h.0.borrow_mut().devices.push(device.clone());
                    }
                    InputEvent::DeviceRemoved { device } => {
                        h.0.borrow_mut().devices.retain(|d| d != device);
                    }
                    _ => {}
                }
                state.process_input_event(event)
            }
        })?;

    // Switching to another terminal and back.
    let h = handle.clone();
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                libinput.suspend();
                let mut udev = h.0.borrow_mut();
                udev.active = false;
                udev.manager.pause();
            }
            SessionEvent::ActivateSession => {
                if libinput.resume().is_err() {
                    tracing::warn!("input devices did not come back");
                }
                {
                    let mut udev = h.0.borrow_mut();
                    udev.active = true;
                    if let Err(e) = udev.manager.activate(false) {
                        tracing::warn!("screens did not come back: {e}");
                    }
                    for screen in udev.screens.values_mut() {
                        screen.queued = false;
                    }
                }
                h.scan(state);
            }
        })?;

    // Screens plugged in or out.
    let udev_backend = UdevBackend::new(&seat_name)?;
    let h = handle.clone();
    event_loop
        .handle()
        .insert_source(udev_backend, move |event, _, state| {
            if let UdevEvent::Changed { device_id } = event
                && device_id == node.dev_id()
            {
                h.scan(state);
            }
        })?;

    handle.scan(state);
    let _ = Path::new("");
    Ok(())
}
