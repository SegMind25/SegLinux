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
    /// Uploaded images: by shape, scale and animation frame.
    buffers: std::collections::HashMap<
        (smithay::input::pointer::CursorIcon, u32, usize),
        MemoryRenderBuffer,
    >,
    element: PointerElement,
    /// Where a dragged window would snap: a tinted area and its outline.
    snap_fill: smithay::backend::renderer::element::solid::SolidColorBuffer,
    snap_outline: [smithay::backend::renderer::element::solid::SolidColorBuffer; 4],
}

impl PointerImage {
    pub fn new() -> Self {
        PointerImage {
            cursor: Cursor::load(),
            buffers: std::collections::HashMap::new(),
            element: PointerElement::default(),
            snap_fill: Default::default(),
            snap_outline: Default::default(),
        }
    }
}

/// Breeze's highlight blue (#3daee9), premultiplied: faint inside, strong
/// at the edge.
const SNAP_FILL: [f32; 4] = [0.06, 0.17, 0.228, 0.25];
const SNAP_OUTLINE: [f32; 4] = [0.215, 0.614, 0.822, 0.9];
const SNAP_LINE: i32 = 3;

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
            CursorImageStatus::Hidden => (0, 0).into(),
            CursorImageStatus::Named(icon) => {
                // Drawn sharp at the screen's scale (2 at 150 %, 200 %).
                let image_scale = output.current_scale().fractional_scale().ceil().max(1.0) as u32;
                let (frame, image) =
                    pointer
                        .cursor
                        .image(*icon, image_scale, state.start_time.elapsed());
                let buffer = pointer
                    .buffers
                    .entry((*icon, image_scale, frame))
                    .or_insert_with(|| {
                        MemoryRenderBuffer::from_slice(
                            &image.pixels_rgba,
                            Fourcc::Argb8888,
                            (image.width as i32, image.height as i32),
                            image_scale as i32,
                            Transform::Normal,
                            None,
                        )
                    })
                    .clone();
                pointer.element.set_buffer(buffer);
                (
                    image.xhot as i32 / image_scale as i32,
                    image.yhot as i32 / image_scale as i32,
                )
                    .into()
            }
        };
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
    // Locked: the lock screen alone (black where it has not drawn yet).
    if state.lock.locked {
        if let Some(lock) = state.lock_surface(output) {
            elements.extend(
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    lock.wl_surface(),
                    (0, 0),
                    scale,
                    1.0,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(OutputElement::Surface),
            );
        }
        return elements;
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
    // A fullscreen window (a game, a video) covers the top panels (the dock).
    let covered = state.fullscreen_on(output);
    if !covered {
        layer(Layer::Top, &mut elements, renderer);
    }
    // Where the window being dragged would snap, over the windows.
    if let Some(rect) = state.snap_preview
        && let Some(on) = rect.intersection(geo)
    {
        let (x, y) = (on.loc.x - geo.loc.x, on.loc.y - geo.loc.y);
        let (w, h) = (on.size.w, on.size.h);
        let l = SNAP_LINE.min(w / 2).min(h / 2).max(1);
        let lines = [
            (x, y, w, l),
            (x, y + h - l, w, l),
            (x, y, l, h),
            (x + w - l, y, l, h),
        ];
        for (buffer, (lx, ly, lw, lh)) in pointer.snap_outline.iter_mut().zip(lines) {
            buffer.update((lw, lh), SNAP_OUTLINE);
            elements.push(OutputElement::Solid(SolidColorRenderElement::from_buffer(
                buffer,
                Point::<i32, smithay::utils::Logical>::from((lx, ly))
                    .to_physical_precise_round(scale),
                scale,
                1.0,
                Kind::Unspecified,
            )));
        }
        pointer.snap_fill.update((w, h), SNAP_FILL);
        elements.push(OutputElement::Solid(SolidColorRenderElement::from_buffer(
            &pointer.snap_fill,
            Point::<i32, smithay::utils::Logical>::from((x, y)).to_physical_precise_round(scale),
            scale,
            1.0,
            Kind::Unspecified,
        )));
    }
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
            let size = window.geometry().size;
            let title = window_title(window);
            let bar: Point<i32, Logical> = (loc.x, loc.y - deco::BAR).into();
            deco::with_deco(window, |d| {
                d.update(size.w, size.h, Some(window) == focused.as_ref(), &title);
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
    if covered {
        layer(Layer::Top, &mut elements, renderer);
    }
    layer(Layer::Bottom, &mut elements, renderer);
    layer(Layer::Background, &mut elements, renderer);
    elements
}

/// Frame callbacks: the windows and panels on `output` may draw again.
pub fn send_frames(state: &State, output: &Output) {
    let time = state.start_time.elapsed();
    if let Some(lock) = state.lock_surface(output) {
        smithay::desktop::utils::send_frames_surface_tree(
            lock.wl_surface(),
            output,
            time,
            Some(std::time::Duration::ZERO),
            |_, _| Some(output.clone()),
        );
    }
    for window in state.space.elements_for_output(output) {
        crate::protocols::prefer_window_scale(window, output);
        window.send_frame(output, time, Some(std::time::Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
    let map = smithay::desktop::layer_map_for_output(output);
    for layer in map.layers() {
        layer.with_surfaces(|surface, data| crate::protocols::prefer_scale(surface, data, output));
        layer.send_frame(output, time, Some(std::time::Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
}

/// The window's title, for its title bar.
pub fn window_title(window: &Window) -> String {
    if let Some(x) = window.x11_surface() {
        return x.title();
    }
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
