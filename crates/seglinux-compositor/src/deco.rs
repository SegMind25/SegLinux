//! Title bars for applications that ask the compositor for them
//! (xdg-decoration, server side: Qt and KDE applications, foot, ...): the
//! title on the left, minimize, maximize and close on the right, as on
//! Windows. GTK applications draw their own and get none.

use std::cell::RefCell;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::element::{memory::MemoryRenderBuffer, solid::SolidColorBuffer},
    },
    desktop::Window,
    reexports::wayland_protocols::xdg::{
        decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
        shell::server::xdg_toplevel,
    },
    utils::{Logical, Point, Rectangle, Size, Transform},
};

/// Height of the title bar.
pub const BAR: i32 = 30;
/// Width of each button.
const BUTTON: i32 = 40;
const TITLE_SIZE: f32 = 13.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Bar,
    Minimize,
    Maximize,
    Close,
}

/// Which part of a `width` wide title bar is at `x` (from its left edge).
pub fn hit(width: i32, x: i32) -> Hit {
    match width - x {
        d if d <= BUTTON => Hit::Close,
        d if d <= 2 * BUTTON => Hit::Maximize,
        d if d <= 3 * BUTTON => Hit::Minimize,
        _ => Hit::Bar,
    }
}

type Rgba = [f32; 4];
const WHITE: Rgba = [0.92, 0.93, 0.95, 1.0];

/// The bar and the button glyphs, as rectangles relative to the bar.
pub fn layout(width: i32, focused: bool) -> Vec<(Rectangle<i32, Logical>, Rgba)> {
    let rect = |x, y, w, h| Rectangle::new((x, y).into(), (w, h).into());
    let bar = if focused {
        [0.118, 0.141, 0.188, 1.0]
    } else {
        [0.165, 0.184, 0.227, 1.0]
    };
    let glyph = if focused {
        WHITE
    } else {
        [0.6, 0.62, 0.66, 1.0]
    };
    let mut out = vec![(rect(0, 0, width, BAR), bar)];
    let x = |n: i32| width - n * BUTTON + (BUTTON - 10) / 2;
    let top = (BAR - 10) / 2;
    // Minimize: a line.
    out.push((rect(x(3), top + 9, 10, 1), glyph));
    // Maximize: a square outline.
    let m = x(2);
    out.push((rect(m, top, 10, 1), glyph));
    out.push((rect(m, top + 9, 10, 1), glyph));
    out.push((rect(m, top, 1, 10), glyph));
    out.push((rect(m + 9, top, 1, 10), glyph));
    // Close: an X of small squares.
    let c = x(1);
    for i in 0..5 {
        out.push((rect(c + 2 * i, top + 2 * i, 2, 2), glyph));
        out.push((rect(c + 8 - 2 * i, top + 2 * i, 2, 2), glyph));
    }
    out
}

/// One window's title bar: buffers kept between frames, so only what
/// changed is drawn again.
pub struct Deco {
    pub rects: Vec<(Point<i32, Logical>, SolidColorBuffer)>,
    key: (i32, bool),
    pub title: Option<(Point<i32, Logical>, MemoryRenderBuffer)>,
    title_key: (String, bool, i32),
}

impl Deco {
    fn new() -> Self {
        Deco {
            rects: Vec::new(),
            key: (-1, false),
            title: None,
            title_key: (String::new(), false, -1),
        }
    }

    /// Bring the buffers up to date for this width, focus and title.
    pub fn update(&mut self, width: i32, focused: bool, title: &str) {
        if self.key != (width, focused) {
            let layout = layout(width, focused);
            if self.rects.len() != layout.len() {
                self.rects = layout
                    .iter()
                    .map(|(r, c)| (r.loc, SolidColorBuffer::new(r.size, *c)))
                    .collect();
            } else {
                for ((loc, buf), (r, c)) in self.rects.iter_mut().zip(&layout) {
                    *loc = r.loc;
                    buf.update(r.size, *c);
                }
            }
            self.key = (width, focused);
        }
        let key = (title.to_string(), focused, width);
        if self.title_key != key {
            let room = (width - 3 * BUTTON - 20).max(0);
            let color = if focused {
                WHITE
            } else {
                [0.6, 0.62, 0.66, 1.0]
            };
            self.title = text(title, room, color)
                .map(|(size, buf)| (Point::from((10, (BAR - size.h) / 2)), buf));
            self.title_key = key;
        }
    }
}

thread_local! {
    static FONT: Option<fontdue::Font> = load_font();
}

fn load_font() -> Option<fontdue::Font> {
    [
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    ]
    .iter()
    .find_map(|path| {
        let data = std::fs::read(path).ok()?;
        fontdue::Font::from_bytes(data, fontdue::FontSettings::default()).ok()
    })
}

/// `title` drawn in one line, cut to `max_width` pixels.
fn text(
    title: &str,
    max_width: i32,
    color: Rgba,
) -> Option<(Size<i32, Logical>, MemoryRenderBuffer)> {
    FONT.with(|font| {
        let font = font.as_ref()?;
        let line = font.horizontal_line_metrics(TITLE_SIZE)?;
        let height = (line.ascent - line.descent).ceil() as i32 + 2;
        let mut glyphs = Vec::new();
        let mut x = 0.0f32;
        for ch in title.chars() {
            let (m, bitmap) = font.rasterize(ch, TITLE_SIZE);
            if (x + m.advance_width) as i32 > max_width {
                break;
            }
            glyphs.push((x, m, bitmap));
            x += m.advance_width;
        }
        let width = x.ceil() as i32;
        if width <= 0 {
            return None;
        }
        // Premultiplied BGRA (Argb8888 in memory, little-endian).
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let baseline = line.ascent.round() as i32 + 1;
        for (gx, m, bitmap) in glyphs {
            for row in 0..m.height as i32 {
                for col in 0..m.width as i32 {
                    let cov = bitmap[(row * m.width as i32 + col) as usize] as f32 / 255.0;
                    let px = gx.round() as i32 + m.xmin + col;
                    let py = baseline - m.ymin - m.height as i32 + row;
                    if px < 0 || py < 0 || px >= width || py >= height || cov == 0.0 {
                        continue;
                    }
                    let i = ((py * width + px) * 4) as usize;
                    let a = cov * color[3];
                    pixels[i] = (color[2] * a * 255.0) as u8;
                    pixels[i + 1] = (color[1] * a * 255.0) as u8;
                    pixels[i + 2] = (color[0] * a * 255.0) as u8;
                    pixels[i + 3] = (a * 255.0) as u8;
                }
            }
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (width, height),
            1,
            Transform::Normal,
            None,
        );
        Some((Size::from((width, height)), buffer))
    })
}

/// Does this window get a title bar from us?
pub fn has_bar(window: &Window) -> bool {
    window.toplevel().is_some_and(|t| {
        t.with_pending_state(|s| {
            s.decoration_mode == Some(DecorationMode::ServerSide)
                && !s.states.contains(xdg_toplevel::State::Fullscreen)
        })
    })
}

/// The window's title bar buffers, made on first use.
pub fn with_deco<T>(window: &Window, f: impl FnOnce(&mut Deco) -> T) -> T {
    let cell = window
        .user_data()
        .get_or_insert(|| RefCell::new(Deco::new()));
    f(&mut cell.borrow_mut())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_from_the_right() {
        assert_eq!(hit(400, 399), Hit::Close);
        assert_eq!(hit(400, 360), Hit::Close);
        assert_eq!(hit(400, 359), Hit::Maximize);
        assert_eq!(hit(400, 300), Hit::Minimize);
        assert_eq!(hit(400, 279), Hit::Bar);
        assert_eq!(hit(400, 0), Hit::Bar);
    }

    #[test]
    fn the_layout_stays_inside_the_bar() {
        for focused in [true, false] {
            let l = layout(300, focused);
            assert_eq!(l[0].0, Rectangle::new((0, 0).into(), (300, BAR).into()));
            for (r, _) in &l {
                assert!(r.loc.x >= 0 && r.loc.y >= 0, "{r:?}");
                assert!(
                    r.loc.x + r.size.w <= 300 && r.loc.y + r.size.h <= BAR,
                    "{r:?}"
                );
            }
        }
        // The same number of rectangles at every width: buffers are reused.
        assert_eq!(layout(200, true).len(), layout(900, false).len());
    }
}
