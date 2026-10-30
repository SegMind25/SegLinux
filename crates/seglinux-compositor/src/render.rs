//! What is drawn on an output: panels, windows and the pointer.

use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        ImportAll, ImportMem, Renderer, Texture,
        element::{AsRenderElements, Kind, memory::MemoryRenderBuffer},
    },
    desktop::space::SpaceRenderElements,
    input::pointer::CursorImageStatus,
    output::Output,
    render_elements,
    utils::{IsAlive, Scale, Transform},
};

use crate::{
    cursor::Cursor,
    pointer::{PointerElement, PointerRenderElement},
    state::State,
};

render_elements! {
    pub OutputElement<R, E> where R: ImportAll + ImportMem;
    Pointer=PointerRenderElement<R>,
    Space=SpaceRenderElements<R, E>,
}

/// The XCursor theme's default image, uploaded once.
pub struct PointerImage {
    cursor: Cursor,
    buffer: Option<MemoryRenderBuffer>,
    element: PointerElement,
}

impl PointerImage {
    pub fn new() -> Self {
        PointerImage {
            cursor: Cursor::load(),
            buffer: None,
            element: PointerElement::default(),
        }
    }
}

/// Everything to draw on `output`, front first.
pub fn output_elements<R>(
    state: &mut State,
    renderer: &mut R,
    output: &Output,
    pointer: &mut PointerImage,
) -> Vec<
    OutputElement<R, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<R>>,
>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let mut elements = Vec::new();
    let Some(geo) = state.space.output_geometry(output) else {
        return elements;
    };
    let scale = Scale::from(output.current_scale().fractional_scale());
    let location = state
        .seat
        .get_pointer()
        .map(|p| p.current_location())
        .unwrap_or_default();
    if geo.to_f64().contains(location) {
        if let CursorImageStatus::Surface(surface) = &state.cursor_status
            && !surface.alive()
        {
            state.cursor_status = CursorImageStatus::default_named();
        }
        let hotspot = match &state.cursor_status {
            CursorImageStatus::Surface(surface) => crate::state::cursor_hotspot(surface),
            _ => {
                let image = pointer.cursor.get_image(1, state.start_time.elapsed());
                if pointer.buffer.is_none() {
                    pointer.buffer = Some(MemoryRenderBuffer::from_slice(
                        &image.pixels_rgba,
                        Fourcc::Argb8888,
                        (image.width as i32, image.height as i32),
                        1,
                        Transform::Normal,
                        None,
                    ));
                }
                (image.xhot as i32, image.yhot as i32).into()
            }
        };
        if let Some(buffer) = &pointer.buffer {
            pointer.element.set_buffer(buffer.clone());
        }
        pointer.element.set_status(state.cursor_status.clone());
        let pos = (location - geo.loc.to_f64() - hotspot.to_f64())
            .to_physical(scale)
            .to_i32_round();
        elements.extend(
            pointer
                .element
                .render_elements::<OutputElement<R, _>>(renderer, pos, scale, 1.0),
        );
    }
    let _ = Kind::Cursor;
    match smithay::desktop::space::space_render_elements(renderer, [&state.space], output, 1.0) {
        Ok(space) => elements.extend(space.into_iter().map(OutputElement::Space)),
        Err(e) => tracing::warn!("nothing to draw on {}: {e:?}", output.name()),
    }
    elements
}

/// Frame callbacks: the windows and panels on `output` may draw again.
pub fn send_frames(state: &State, output: &Output) {
    let time = state.start_time.elapsed();
    for window in state.space.elements_for_output(output) {
        window.send_frame(output, time, Some(std::time::Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
    let map = smithay::desktop::layer_map_for_output(output);
    for layer in map.layers() {
        layer.send_frame(output, time, Some(std::time::Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
}
