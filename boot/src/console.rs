//! Output: the UEFI text console (until ExitBootServices) and COM1.

use crate::efi;
use core::arch::asm;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicPtr, Ordering};

/// ConOut while boot services are active, null afterwards.
static CON_OUT: AtomicPtr<efi::SimpleTextOutput> = AtomicPtr::new(core::ptr::null_mut());

pub fn set_con_out(p: *mut efi::SimpleTextOutput) {
    CON_OUT.store(p, Ordering::SeqCst);
}

/// Called right before ExitBootServices: ConOut must not be used after it.
pub fn drop_con_out() {
    CON_OUT.store(core::ptr::null_mut(), Ordering::SeqCst);
}

struct ConOut(*mut efi::SimpleTextOutput);

impl Write for ConOut {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut buf = [0u16; 128];
        let mut n = 0;
        let flush = |buf: &mut [u16; 128], n: &mut usize| {
            buf[*n] = 0;
            // SAFETY: ConOut is valid while boot services are active and the
            // buffer is NUL-terminated.
            unsafe { ((*self.0).output_string)(self.0, buf.as_ptr()) };
            *n = 0;
        };
        for c in s.chars() {
            if n + 3 >= buf.len() {
                flush(&mut buf, &mut n);
            }
            if c == '\n' {
                buf[n] = b'\r' as u16;
                n += 1;
            }
            let mut tmp = [0u16; 2];
            let enc = c.encode_utf16(&mut tmp);
            // Only the basic multilingual plane is printable.
            buf[n] = if enc.len() == 1 { enc[0] } else { b'?' as u16 };
            n += 1;
        }
        if n > 0 {
            flush(&mut buf, &mut n);
        }
        Ok(())
    }
}

const COM1: u16 = 0x3f8;

unsafe fn outb(port: u16, v: u8) {
    // SAFETY: the caller owns the port.
    unsafe { asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags)) };
}

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    // SAFETY: the caller owns the port.
    unsafe { asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}

/// 115200 baud, 8 data bits, no parity, one stop bit, FIFO on.
pub fn serial_init() {
    // SAFETY: COM1 is the standard PC serial port; nothing else drives it
    // from segboot.
    unsafe {
        outb(COM1 + 1, 0x00); // interrupts off
        outb(COM1 + 3, 0x80); // DLAB on
        outb(COM1, 0x01); // divisor 1 = 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8N1, DLAB off
        outb(COM1 + 2, 0xc7); // FIFO on, cleared, 14-byte threshold
        outb(COM1 + 4, 0x03); // DTR + RTS
    }
}

struct Serial;

impl Serial {
    fn byte(b: u8) {
        // Bounded wait: a missing UART reads 0xff (always "ready") or never
        // becomes ready; either way do not hang.
        for _ in 0..100_000 {
            // SAFETY: reading the line status register has no side effect.
            if unsafe { inb(COM1 + 5) } & 0x20 != 0 {
                break;
            }
        }
        // SAFETY: COM1 data register.
        unsafe { outb(COM1, b) };
    }
}

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                Self::byte(b'\r');
            }
            Self::byte(b);
        }
        Ok(())
    }
}

/// Text console only (on OVMF it is mirrored to the serial port anyway).
pub fn con_print(args: fmt::Arguments) {
    let p = CON_OUT.load(Ordering::SeqCst);
    if !p.is_null() {
        let _ = ConOut(p).write_fmt(args);
    }
}

pub fn serial_print(args: fmt::Arguments) {
    let _ = Serial.write_fmt(args);
}

/// Text console (if still available) and COM1.
pub fn both_print(args: fmt::Arguments) {
    con_print(args);
    serial_print(args);
}

/// Progress message on the text console.
#[macro_export]
macro_rules! info {
    ($($t:tt)*) => { $crate::console::con_print(format_args!("segboot: {}\n", format_args!($($t)*))) };
}

/// Progress message on the text console and COM1.
#[macro_export]
macro_rules! progress {
    ($($t:tt)*) => { $crate::console::both_print(format_args!("segboot: {}\n", format_args!($($t)*))) };
}

/// Message on COM1 only (after ExitBootServices).
#[macro_export]
macro_rules! serial {
    ($($t:tt)*) => { $crate::console::serial_print(format_args!("segboot: {}\n", format_args!($($t)*))) };
}
