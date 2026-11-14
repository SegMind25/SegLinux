//! Hardware abstraction for x86-64. Most of the kernel's `unsafe` lives here.

pub mod apic;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod percpu;
pub mod serial;
pub mod smp;

/// Brings the CPU into a known state: segments, TSS, exception handlers,
/// legacy PIC masked (the APIC takes over in Phase 1).
pub fn init() {
    gdt::init();
    idt::init();
    cpu::mask_legacy_pic();
}
