//! Running in a window of another desktop: for trying the compositor out
//! and for development (`seglinux-compositor --winit`).

use std::{cell::RefCell, rc::Rc, time::Duration};

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer, ImportDma},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
    wayland::dmabuf::DmabufFeedbackBuilder,
};

use crate::{
    render::{output_elements, send_frames, PointerImage},
    state::{Backend, State},
};

struct WinitBackend(Rc<RefCell<WinitGraphicsBackend<GlesRenderer>>>);

impl Backend for WinitBackend {
    fn schedule_render(&mut self) {
        self.0.borrow().window().request_redraw();
    }
    fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.0.borrow_mut().renderer().import_dmabuf(dmabuf, None).is_ok()
    }
}

pub fn init(event_loop: &mut EventLoop<'static, State>, state: &mut State) -> Result<(), Box<dyn std::error::Error>> {
    let (backend, winit) = winit::init::<GlesRenderer>()?;
    let backend = Rc::new(RefCell::new(backend));
    let size = backend.borrow().window_size();
    let mode = Mode { size, refresh: 60_000 };
    let output = Output::new(
        "winit".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "SegLinux".into(),
            model: "Window".into(),
        },
    );
    output.create_global::<State>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    // Buffers in GPU memory, for applications that draw with OpenGL or Vulkan.
    let formats = backend.borrow_mut().renderer().dmabuf_formats();
    if let Ok(feedback) = DmabufFeedbackBuilder::new(backend.borrow_mut().renderer().egl_context().display().get_device_id().unwrap_or(0), formats).build() {
        state.dmabuf_global = Some(
            state
                .dmabuf_state
                .create_global_with_default_feedback::<State>(&state.display_handle, &feedback),
        );
    }
    state.backend = Box::new(WinitBackend(backend.clone()));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut pointer = PointerImage::new();
    event_loop.handle().insert_source(winit, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => {
            output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
            state.arrange_layers();
        }
        WinitEvent::Input(event) => state.process_input_event(event),
        WinitEvent::Redraw => {
            let mut backend = backend.borrow_mut();
            let size = backend.window_size();
            let background = state.config.background;
            let rendered = {
                let Ok((renderer, mut framebuffer)) = backend.bind() else { return };
                let elements = output_elements(state, renderer, &output, &mut pointer);
                damage_tracker
                    .render_output(renderer, &mut framebuffer, 0, &elements, background)
                    .is_ok()
            };
            if rendered {
                let _ = backend.submit(Some(&[Rectangle::from_size(size)]));
            }
            send_frames(state, &output);
            let _ = Duration::ZERO;
        }
        WinitEvent::CloseRequested => state.loop_signal.stop(),
        _ => {}
    })?;
    Ok(())
}
