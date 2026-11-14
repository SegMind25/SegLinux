//! x86-64 instructions and registers the kernel uses: port I/O, MSRs,
//! control registers, interrupts, descriptor tables, TLB.

use core::arch::asm;

pub unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags))
    }
}

pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe {
        asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags))
    };
    v
}

pub unsafe fn outl(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags))
    }
}

pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags))
    };
    (hi as u64) << 32 | lo as u64
}

pub unsafe fn wrmsr(msr: u32, value: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32,
             options(nostack, preserves_flags))
    }
}

pub mod msr {
    pub const APIC_BASE: u32 = 0x1b;
    pub const EFER: u32 = 0xc000_0080;
    pub const GS_BASE: u32 = 0xc000_0101;
}

pub fn read_cr2() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

pub fn read_cr3() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

pub unsafe fn invlpg(addr: u64) {
    unsafe { asm!("invlpg [{}]", in(reg) addr, options(nostack, preserves_flags)) }
}

pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) };
    flags & (1 << 9) != 0
}

pub fn enable_interrupts() {
    unsafe { asm!("sti", options(nomem, nostack)) }
}

pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nomem, nostack)) }
}

/// `sti; hlt` as one step: an interrupt arriving between them still wakes
/// the CPU (sti takes effect after the next instruction).
pub fn enable_interrupts_and_halt() {
    unsafe { asm!("sti", "hlt", options(nomem, nostack)) }
}

pub fn halt() {
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) }
}

pub fn breakpoint() {
    unsafe { asm!("int3", options(nomem, nostack)) }
}

pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let was = interrupts_enabled();
    disable_interrupts();
    let r = f();
    if was {
        enable_interrupts();
    }
    r
}

/// Operand of `lgdt`/`lidt`.
#[repr(C, packed)]
pub struct DescriptorPointer {
    pub limit: u16,
    pub base: u64,
}

pub unsafe fn lgdt(p: &DescriptorPointer) {
    unsafe { asm!("lgdt [{}]", in(reg) p, options(readonly, nostack, preserves_flags)) }
}

pub unsafe fn lidt(p: &DescriptorPointer) {
    unsafe { asm!("lidt [{}]", in(reg) p, options(readonly, nostack, preserves_flags)) }
}

pub unsafe fn ltr(selector: u16) {
    unsafe { asm!("ltr {:x}", in(reg) selector, options(nostack, preserves_flags)) }
}

/// Reloads CS with a far return, and the data segment registers.
pub unsafe fn load_segments(code: u16, data: u16) {
    unsafe {
        asm!(
            "push {code}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov ss, {data:x}",
            code = in(reg) code as u64,
            data = in(reg) data,
            tmp = lateout(reg) _,
            options(preserves_flags),
        )
    }
}
