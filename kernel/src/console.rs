//! Text console on the boot framebuffer, drawn with SegOS's own font.

use crate::font;
use crate::hal::serial;
use crate::sync::SpinLock;
use bootinfo::{Framebuffer, PixelFormat};
use core::fmt::{self, Write};

const CELL_W: usize = font::WIDTH;
const CELL_H: usize = font::HEIGHT;
const FG: u32 = 0x00e6_e6e6;
const BG: u32 = 0x0010_1418;

pub struct Console {
    base: *mut u32,
    pitch: usize, // in pixels
    width: usize,
    height: usize,
    cols: usize,
    rows: usize,
    col: usize,
    row: usize,
    fg: u32,
    bg: u32,
    rgb: bool,
}

// The framebuffer pointer is only used under the console mutex.
unsafe impl Send for Console {}

static CONSOLE: SpinLock<Option<Console>> = SpinLock::new(None);

pub fn init(fb: &Framebuffer) {
    if fb.address == 0 || fb.bpp != 32 {
        crate::warn!("console: no usable framebuffer, serial only");
        return;
    }
    let (width, height) = (fb.width as usize, fb.height as usize);
    let rgb = fb.format == PixelFormat::Rgbx;
    let mut console = Console {
        base: crate::mm::phys_to_virt(fb.address).cast(),
        pitch: fb.pitch as usize / 4,
        width,
        height,
        cols: width / CELL_W,
        rows: height / CELL_H,
        col: 0,
        row: 0,
        fg: convert(FG, rgb),
        bg: convert(BG, rgb),
        rgb,
    };
    console.clear();
    *CONSOLE.lock() = Some(console);
}

/// 0x00RRGGBB to the framebuffer's byte order.
fn convert(rgb: u32, swap: bool) -> u32 {
    if swap {
        (rgb & 0xff) << 16 | (rgb & 0xff00) | (rgb >> 16) & 0xff
    } else {
        rgb
    }
}

impl Console {
    fn pixel(&mut self, x: usize, y: usize, color: u32) {
        unsafe { self.base.add(y * self.pitch + x).write_volatile(color) }
    }

    fn clear(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.pixel(x, y, self.bg);
            }
        }
    }

    fn glyph(&mut self, c: char) {
        let rows = font::glyph(c);
        let (x0, y0) = (self.col * CELL_W, self.row * CELL_H);
        for (dy, bits) in rows.iter().enumerate() {
            for dx in 0..CELL_W {
                let color = if bits & (0x80 >> dx) != 0 {
                    self.fg
                } else {
                    self.bg
                };
                self.pixel(x0 + dx, y0 + dy, color);
            }
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        self.row += 1;
        if self.row < self.rows {
            return;
        }
        // Scroll one text row up.
        let row_px = CELL_H * self.pitch;
        let total = self.rows * row_px;
        unsafe { core::ptr::copy(self.base.add(row_px), self.base, total - row_px) };
        for y in (self.rows - 1) * CELL_H..self.rows * CELL_H {
            for x in 0..self.width {
                self.pixel(x, y, self.bg);
            }
        }
        self.row = self.rows - 1;
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            match c {
                '\n' => self.newline(),
                c => {
                    if self.col >= self.cols {
                        self.newline();
                    }
                    self.glyph(c);
                    self.col += 1;
                }
            }
        }
        Ok(())
    }
}

/// Prints to serial and, once it exists, the screen.
pub fn print(args: fmt::Arguments) {
    crate::hal::x86::without_interrupts(|| {
        let _ = serial::SERIAL.lock().write_fmt(args);
        if let Some(console) = CONSOLE.lock().as_mut() {
            let _ = console.write_fmt(args);
        }
    });
}

/// For the panic handler: never blocks on a lock.
pub fn print_panic(args: fmt::Arguments) {
    serial::write_unlocked(args);
    if let Some(mut guard) = CONSOLE.try_lock()
        && let Some(console) = guard.as_mut()
    {
        console.fg = convert(0x00ff_6060, console.rgb);
        let _ = console.write_fmt(args);
    }
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::console::print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::console::print(format_args!("{}\n", format_args!($($arg)*))) };
}
