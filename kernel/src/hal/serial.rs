//! 16550 UART on COM1: the first output that works and the last that fails.

use core::fmt;
use spin::Mutex;
use x86_64::instructions::port::Port;

const COM1: u16 = 0x3f8;

pub struct Serial {
    base: u16,
}

impl Serial {
    const fn new(base: u16) -> Self {
        Self { base }
    }

    fn port(&self, offset: u16) -> Port<u8> {
        Port::new(self.base + offset)
    }

    fn init(&mut self) {
        unsafe {
            self.port(1).write(0x00); // no interrupts
            self.port(3).write(0x80); // DLAB on
            self.port(0).write(0x01); // divisor 1: 115200 baud
            self.port(1).write(0x00);
            self.port(3).write(0x03); // 8N1, DLAB off
            self.port(2).write(0xc7); // FIFO on, cleared, 14-byte threshold
            self.port(4).write(0x0b); // DTR, RTS, OUT2
        }
    }

    fn write_byte(&mut self, byte: u8) {
        unsafe {
            while self.port(5).read() & 0x20 == 0 {
                core::hint::spin_loop();
            }
            self.port(0).write(byte);
        }
    }
}

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }
}

pub static SERIAL: Mutex<Serial> = Mutex::new(Serial::new(COM1));

pub fn init() {
    SERIAL.lock().init();
}

/// Writes without waiting for the lock: used by the panic handler, where the
/// lock may be held by the code that panicked.
pub fn write_unlocked(args: fmt::Arguments) {
    let mut serial = Serial::new(COM1);
    let _ = fmt::Write::write_fmt(&mut serial, args);
}
