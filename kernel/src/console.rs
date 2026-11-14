//! Text console on the Limine framebuffer, and the `log` backend that writes
//! to both the serial port and the screen.

use crate::hal::serial;
use crate::limine::Framebuffer;
use core::fmt::{self, Write};
use spin::Mutex;

const SCALE: usize = 2;
const CELL: usize = 8 * SCALE;
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
}

// The framebuffer pointer is only used under the console mutex.
unsafe impl Send for Console {}

static CONSOLE: Mutex<Option<Console>> = Mutex::new(None);

pub fn init(fb: &Framebuffer) {
    if fb.bpp != 32 {
        log::warn!(
            "console: framebuffer has {} bpp, only 32 is supported",
            fb.bpp
        );
        return;
    }
    let (width, height) = (fb.width as usize, fb.height as usize);
    let mut console = Console {
        base: fb.address.cast(),
        pitch: fb.pitch as usize / 4,
        width,
        height,
        cols: width / CELL,
        rows: height / CELL,
        col: 0,
        row: 0,
        fg: FG,
    };
    console.clear();
    *CONSOLE.lock() = Some(console);
}

impl Console {
    fn pixel(&mut self, x: usize, y: usize, color: u32) {
        unsafe { self.base.add(y * self.pitch + x).write_volatile(color) }
    }

    fn clear(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.pixel(x, y, BG);
            }
        }
    }

    fn glyph(&mut self, c: char) {
        use font8x8::UnicodeFonts;
        let bitmap = font8x8::BASIC_FONTS.get(c).unwrap_or([0xff; 8]);
        let (x0, y0) = (self.col * CELL, self.row * CELL);
        for (dy, bits) in bitmap.iter().enumerate() {
            for dx in 0..8 {
                let color = if bits & (1 << dx) != 0 { self.fg } else { BG };
                for sy in 0..SCALE {
                    for sx in 0..SCALE {
                        self.pixel(x0 + dx * SCALE + sx, y0 + dy * SCALE + sy, color);
                    }
                }
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
        let row_px = CELL * self.pitch;
        let total = self.rows * row_px;
        unsafe { core::ptr::copy(self.base.add(row_px), self.base, total - row_px) };
        for y in (self.rows - 1) * CELL..self.rows * CELL {
            for x in 0..self.width {
                self.pixel(x, y, BG);
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
    x86_64::instructions::interrupts::without_interrupts(|| {
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
        console.fg = 0x00ff_6060;
        let _ = console.write_fmt(args);
    }
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::console::print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::console::print(format_args!("{}\n", format_args!($($arg)*))) };
}

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let level = match record.level() {
            log::Level::Error => "error",
            log::Level::Warn => "warn ",
            log::Level::Info => "info ",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        };
        print(format_args!("[{level}] {}\n", record.args()));
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

pub fn init_logger() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
}
