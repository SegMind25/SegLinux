//! Screenshots (wlr-screencopy v3: grim, the Print key and Super+Shift+S).
//! A client asks for an output or a part of it, gets the buffer size it
//! must provide, and the next frame drawn is copied into its buffer.
//! Screen sharing (xdg-desktop-portal-wlr) asks with `copy_with_damage`:
//! such a frame waits until something on screen changed, and says what.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self as frame, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self as manager, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            protocol::{wl_buffer::WlBuffer, wl_shm},
        },
    },
    utils::{Physical, Rectangle, Transform},
    wayland::shm::with_buffer_contents_mut,
};

use crate::{render::PointerImage, state::State};

/// What a frame copies: an output, and a part of it in physical pixels.
pub struct FrameData {
    output: Output,
    region: Rectangle<i32, Physical>,
    cursor: bool,
}

/// A frame whose client handed us its buffer: filled at the next draw.
pub struct Pending {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    with_damage: bool,
}

/// The last image of an output (with or without the pointer), redrawn
/// only where it changed: sharing a still screen costs nothing.
struct Image {
    output: Output,
    cursor: bool,
    size: smithay::utils::Size<i32, Physical>,
    scale: f64,
    texture: GlesTexture,
    tracker: OutputDamageTracker,
}

#[derive(Default)]
pub struct CaptureState {
    pub pending: Vec<Pending>,
    images: Vec<Image>,
}

impl CaptureState {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<State, ZwlrScreencopyManagerV1, ()>(3, ());
        Self::default()
    }
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn bind(
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        request: manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let (frame, cursor, wl_output, region) = match request {
            manager::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor, output, None),
            manager::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (frame, overlay_cursor, output, Some((x, y, width, height))),
            _ => return,
        };
        let Some(output) = Output::from_resource(&wl_output) else {
            data_init.init(frame, None).failed();
            return;
        };
        let Some(mode) = output.current_mode() else {
            data_init.init(frame, None).failed();
            return;
        };
        let full = Rectangle::from_size(mode.size);
        // The region is in logical coordinates of the output.
        let scale = output.current_scale().fractional_scale();
        let region = match region {
            Some((x, y, w, h)) => {
                Rectangle::<i32, smithay::utils::Logical>::new((x, y).into(), (w, h).into())
                    .to_f64()
                    .to_physical(scale)
                    .to_i32_round()
                    .intersection(full)
            }
            None => Some(full),
        };
        let Some(region) = region.filter(|r| r.size.w > 0 && r.size.h > 0) else {
            data_init.init(frame, None).failed();
            return;
        };
        let frame = data_init.init(
            frame,
            Some(FrameData {
                output,
                region,
                cursor: cursor != 0,
            }),
        );
        frame.buffer(
            wl_shm::Format::Argb8888,
            region.size.w as u32,
            region.size.h as u32,
            region.size.w as u32 * 4,
        );
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, Option<FrameData>> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        resource: &ZwlrScreencopyFrameV1,
        request: frame::Request,
        _data: &Option<FrameData>,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            frame::Request::Copy { buffer } => {
                state.capture.pending.push(Pending {
                    frame: resource.clone(),
                    buffer,
                    with_damage: false,
                });
                state.backend.schedule_render();
            }
            // Filled at the next draw that changes something.
            frame::Request::CopyWithDamage { buffer } => {
                state.capture.pending.push(Pending {
                    frame: resource.clone(),
                    buffer,
                    with_damage: true,
                });
                // A first frame (no image yet) is drawn right away.
                let has_image = resource
                    .data::<Option<FrameData>>()
                    .and_then(|d| d.as_ref())
                    .is_some_and(|d| {
                        state
                            .capture
                            .images
                            .iter()
                            .any(|i| i.output == d.output && i.cursor == d.cursor)
                    });
                if !has_image {
                    state.backend.schedule_render();
                }
            }
            frame::Request::Destroy => state.capture.pending.retain(|p| &p.frame != resource),
            _ => {}
        }
    }
}

/// Fill the buffers of the frames waiting for `output`.
pub fn fulfill(
    state: &mut State,
    renderer: &mut GlesRenderer,
    output: &Output,
    pointer: &mut PointerImage,
) {
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.capture.pending)
        .into_iter()
        .partition(|p| {
            p.frame
                .data::<Option<FrameData>>()
                .and_then(|d| d.as_ref())
                .is_some_and(|d| &d.output == output)
        });
    state.capture.pending = rest;
    // Forget the images of unplugged screens.
    let outputs: Vec<Output> = state.space.outputs().cloned().collect();
    state.capture.images.retain(|i| outputs.contains(&i.output));
    if mine.is_empty() {
        return;
    }
    let Some(mode) = output.current_mode() else {
        return;
    };
    let scale = output.current_scale().fractional_scale();
    let background = state.config.background;
    // Bring each image this output's frames need up to date, once.
    let mut changed: Vec<(bool, Vec<Rectangle<i32, Physical>>)> = Vec::new();
    for cursor in [true, false] {
        let wanted = mine.iter().any(|p| {
            p.frame
                .data::<Option<FrameData>>()
                .and_then(|d| d.as_ref())
                .is_some_and(|d| d.cursor == cursor)
        });
        if !wanted {
            continue;
        }
        let mut elements = crate::render::output_elements(state, renderer, output, pointer);
        if !cursor {
            elements.retain(|e| !matches!(e, crate::render::OutputElement::Pointer(_)));
        }
        match redraw(
            &mut state.capture.images,
            renderer,
            output,
            cursor,
            mode.size,
            scale,
            &elements,
            background,
        ) {
            Ok(damage) => changed.push((cursor, damage)),
            Err(e) => tracing::warn!("screenshot failed: {e}"),
        }
    }
    for p in mine {
        let Some(data) = p.frame.data::<Option<FrameData>>().and_then(|d| d.as_ref()) else {
            continue;
        };
        let Some((_, damage)) = changed.iter().find(|(c, _)| *c == data.cursor) else {
            p.frame.failed();
            continue;
        };
        // What changed inside the region, relative to it.
        let damage: Vec<_> = damage
            .iter()
            .filter_map(|d| d.intersection(data.region))
            .map(|d| Rectangle::new(d.loc - data.region.loc, d.size))
            .collect();
        if p.with_damage && damage.is_empty() {
            // Nothing new: wait for a draw that changes something.
            state.capture.pending.push(p);
            continue;
        }
        let Some(image) = state
            .capture
            .images
            .iter_mut()
            .find(|i| &i.output == output && i.cursor == data.cursor)
        else {
            p.frame.failed();
            continue;
        };
        match copy(renderer, image, data.region, &p.buffer) {
            Ok(()) => {
                p.frame.flags(frame::Flags::empty());
                if p.with_damage {
                    for d in &damage {
                        p.frame.damage(
                            d.loc.x as u32,
                            d.loc.y as u32,
                            d.size.w as u32,
                            d.size.h as u32,
                        );
                    }
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                let secs = now.as_secs();
                p.frame
                    .ready((secs >> 32) as u32, secs as u32, now.subsec_nanos());
            }
            Err(e) => {
                tracing::warn!("screenshot failed: {e}");
                p.frame.failed();
            }
        }
    }
}

/// Redraw what changed in the output's image; returns the changed areas
/// (all of it for a new image).
#[allow(clippy::too_many_arguments)]
fn redraw(
    images: &mut Vec<Image>,
    renderer: &mut GlesRenderer,
    output: &Output,
    cursor: bool,
    size: smithay::utils::Size<i32, Physical>,
    scale: f64,
    elements: &[crate::render::OutputElement<GlesRenderer>],
    background: [f32; 4],
) -> Result<Vec<Rectangle<i32, Physical>>, String> {
    let index = match images
        .iter()
        .position(|i| &i.output == output && i.cursor == cursor)
    {
        Some(i) if images[i].size == size && images[i].scale == scale => i,
        found => {
            if let Some(i) = found {
                images.remove(i);
            }
            let texture: GlesTexture = renderer
                .create_buffer(
                    Fourcc::Argb8888,
                    size.to_logical(1).to_buffer(1, Transform::Normal),
                )
                .map_err(|e| e.to_string())?;
            images.push(Image {
                output: output.clone(),
                cursor,
                size,
                scale,
                texture,
                tracker: OutputDamageTracker::new(size, scale, Transform::Normal),
            });
            images.len() - 1
        }
    };
    let image = &mut images[index];
    let mut fb = renderer
        .bind(&mut image.texture)
        .map_err(|e| e.to_string())?;
    // Age 1: the texture holds the previous image.
    let result = image
        .tracker
        .render_output(renderer, &mut fb, 1, elements, background)
        .map_err(|e| format!("{e:?}"))?;
    Ok(result.damage.cloned().unwrap_or_default())
}

/// Copy `region` of the image into the client's buffer.
fn copy(
    renderer: &mut GlesRenderer,
    image: &mut Image,
    region: Rectangle<i32, Physical>,
    buffer: &WlBuffer,
) -> Result<(), String> {
    let size = image.size;
    let fb = renderer
        .bind(&mut image.texture)
        .map_err(|e| e.to_string())?;
    // The texture holds the image bottom row first: read the matching rows.
    let area = Rectangle::new(
        (region.loc.x, size.h - region.loc.y - region.size.h).into(),
        (region.size.w, region.size.h).into(),
    );
    let mapping = renderer
        .copy_framebuffer(&fb, area, Fourcc::Argb8888)
        .map_err(|e| e.to_string())?;
    let pixels = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    let row = region.size.w as usize * 4;
    with_buffer_contents_mut(buffer, |ptr, len, data| {
        if data.format != wl_shm::Format::Argb8888 && data.format != wl_shm::Format::Xrgb8888 {
            return Err("unsupported buffer format".to_string());
        }
        if data.width != region.size.w || data.height != region.size.h {
            return Err("wrong buffer size".to_string());
        }
        let stride = data.stride as usize;
        let offset = data.offset as usize;
        if offset + stride * (data.height as usize - 1) + row > len
            || pixels.len() < row * data.height as usize
        {
            return Err("buffer too small".to_string());
        }
        for y in 0..data.height as usize {
            // Safety: bounds checked above against the pool's length.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add((data.height as usize - 1 - y) * row),
                    ptr.add(offset + y * stride),
                    row,
                );
            }
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?
}
