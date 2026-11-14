//! Small CPU helpers.

use super::x86;

pub fn halt_forever() -> ! {
    loop {
        x86::disable_interrupts();
        x86::halt();
    }
}

/// Masks every line of the two 8259 PICs so they never fire.
pub fn mask_legacy_pic() {
    unsafe {
        x86::outb(0x21, 0xff);
        x86::outb(0xa1, 0xff);
    }
}

/// Exit codes for QEMU's `isa-debug-exit` device (iobase 0xf4).
/// QEMU exits with `(code << 1) | 1`: 33 for success, 35 for failure.
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum QemuExit {
    Success = 0x10,
    Failure = 0x11,
}

pub fn exit_qemu(code: QemuExit) -> ! {
    unsafe { x86::outl(0xf4, code as u32) };
    // Not running under QEMU with the device: stop here.
    halt_forever()
}

/// Walks the frame-pointer chain (the kernel is built with frame pointers)
/// and calls `f` with each return address.
pub fn backtrace(mut f: impl FnMut(usize, u64)) {
    let mut rbp: u64;
    unsafe { core::arch::asm!("mov {}, rbp", out(reg) rbp) };
    for depth in 0..24 {
        // Only follow frames on kernel stacks (higher half), aligned.
        if rbp < 0xffff_8000_0000_0000 || !rbp.is_multiple_of(8) {
            break;
        }
        let ret = unsafe { *((rbp + 8) as *const u64) };
        if ret == 0 {
            break;
        }
        f(depth, ret);
        rbp = unsafe { *(rbp as *const u64) };
    }
}
