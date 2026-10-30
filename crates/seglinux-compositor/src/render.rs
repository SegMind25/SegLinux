//! What is drawn on an output: panels, windows and the pointer.

use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        ImportAll, ImportMem, Renderer, Texture,
        element::{
            AsRenderElements, Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement,
        },
    },
    desktop::{Window, layer_map_for_output},
    input::pointer::CursorImageStatus,
    output::Output,
    render_elements,
    utils::{IsAlive, Logical, Point, Scale, Transform},
    wayland::{
        compositor::with_states,
        shell::{wlr_layer::Layer, xdg::XdgToplevelSurfaceData},
    },
};

use crate::{
    cursor::Cursor,
    deco,
    pointer::{PointerElement, PointerRenderElement},
    state::State,
};

render_elements! {
    pub OutputElement<R> where R: ImportAll + ImportMem;
    Pointer=PointerRenderElement<R>,
    Surface=WaylandSurfaceRenderElement<R>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<R>,
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
) -> Vec<OutputElement<R>>
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
                .render_elements::<OutputElement<R>>(renderer, pos, scale, 1.0),
        );
    }
    // Front to back: overlay and top panels, windows with their title
    // bars, then bottom panels and the wallpaper.
    let layers = layer_map_for_output(output);
    let layer = |l: Layer, elements: &mut Vec<OutputElement<R>>, renderer: &mut R| {
        for surface in layers.layers_on(l).rev() {
            let Some(geo) = layers.layer_geometry(surface) else {
                continue;
            };
            elements.extend(
                surface
                    .render_elements::<WaylandSurfaceRenderElement<R>>(
                        renderer,
                        geo.loc.to_physical_precise_round(scale),
                        scale,
                        1.0,
                    )
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
    };
    layer(Layer::Overlay, &mut elements, renderer);
    layer(Layer::Top, &mut elements, renderer);
    let focused = state.focused_window();
    for window in state.space.elements_for_output(output).rev() {
        let Some(loc) = state.space.element_location(window) else {
            continue;
        };
        let loc = loc - geo.loc;
        let render_loc = (loc - window.geometry().loc).to_physical_precise_round(scale);
        elements.extend(
            window
                .render_elements::<WaylandSurfaceRenderElement<R>>(renderer, render_loc, scale, 1.0)
                .into_iter()
                .map(OutputElement::Surface),
        );
        if deco::has_bar(window) {
            let width = window.geometry().size.w;
            let title = window_title(window);
            let bar: Point<i32, Logical> = (loc.x, loc.y - deco::BAR).into();
            deco::with_deco(window, |d| {
                d.update(width, Some(window) == focused.as_ref(), &title);
                if let Some((at, buffer)) = &d.title
                    && let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (bar + *at).to_f64().to_physical(scale),
                        buffer,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    )
                {
                    elements.push(OutputElement::Memory(e));
                }
                // Glyphs before the bar they sit on.
                for (at, buffer) in d.rects.iter().rev() {
                    elements.push(OutputElement::Solid(SolidColorRenderElement::from_buffer(
                        buffer,
                        (bar + *at).to_physical_precise_round(scale),
                        scale,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            });
        }
    }
    layer(Layer::Bottom, &mut elements, renderer);
    layer(Layer::Background, &mut elements, renderer);
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

/// The window's title, for its title bar.
pub fn window_title(window: &Window) -> String {
    window
        .toplevel()
        .map(|t| {
            with_states(t.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().unwrap().title.clone())
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
}
