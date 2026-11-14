//! Interrupt descriptor table and the common trap path.
//!
//! build.rs generates one tiny stub per vector (`isr_N`) that pushes a dummy
//! error code where the CPU does not, then the vector number, and jumps to
//! `isr_common`. That saves all registers into a [`TrapFrame`] and calls
//! [`trap_dispatch`]. Ring-3 processes (Phase 2) reuse the same frame.

use super::x86::{self, DescriptorPointer};

core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/isr_stubs.s")));

core::arch::global_asm!(
    ".section .text",
    ".global isr_common",
    "isr_common:",
    "    push rax",
    "    push rbx",
    "    push rcx",
    "    push rdx",
    "    push rsi",
    "    push rdi",
    "    push rbp",
    "    push r8",
    "    push r9",
    "    push r10",
    "    push r11",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    "    mov rdi, rsp",
    "    cld",
    "    call trap_dispatch",
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop r11",
    "    pop r10",
    "    pop r9",
    "    pop r8",
    "    pop rbp",
    "    pop rdi",
    "    pop rsi",
    "    pop rdx",
    "    pop rcx",
    "    pop rbx",
    "    pop rax",
    "    add rsp, 16", // vector and error code
    "    iretq",
);

unsafe extern "C" {
    static isr_table: [u64; 256];
}

/// Registers as saved by `isr_common`, lowest address first.
#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    // Pushed by the CPU.
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub mod vector {
    pub const DIVIDE_ERROR: u64 = 0;
    pub const BREAKPOINT: u64 = 3;
    pub const INVALID_OPCODE: u64 = 6;
    pub const DOUBLE_FAULT: u64 = 8;
    pub const GENERAL_PROTECTION: u64 = 13;
    pub const PAGE_FAULT: u64 = 14;
}

const EXCEPTION_NAMES: [&str; 32] = [
    "divide error",
    "debug",
    "non-maskable interrupt",
    "breakpoint",
    "overflow",
    "bound range exceeded",
    "invalid opcode",
    "device not available",
    "double fault",
    "coprocessor segment overrun",
    "invalid TSS",
    "segment not present",
    "stack-segment fault",
    "general protection fault",
    "page fault",
    "reserved",
    "x87 floating-point exception",
    "alignment check",
    "machine check",
    "SIMD floating-point exception",
    "virtualization exception",
    "control protection exception",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "hypervisor injection exception",
    "VMM communication exception",
    "security exception",
    "reserved",
];

#[repr(C)]
#[derive(Clone, Copy)]
struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl Gate {
    const EMPTY: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_mid: 0,
        offset_high: 0,
        _reserved: 0,
    };

    /// Present, ring-0 interrupt gate (interrupts stay off in the handler).
    fn interrupt(handler: u64, ist: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector: super::gdt::KERNEL_CODE,
            ist,
            attributes: 0x8e,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

#[repr(C, align(16))]
struct Idt([Gate; 256]);

static IDT: crate::sync::Once<Idt> = crate::sync::Once::new();

/// Builds the table once and loads it on this CPU.
pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = Idt([Gate::EMPTY; 256]);
        let table = unsafe { &*(&raw const isr_table) };
        for (vector, &handler) in table.iter().enumerate() {
            let ist = if vector as u64 == vector::DOUBLE_FAULT {
                super::gdt::DOUBLE_FAULT_IST as u8 + 1
            } else {
                0
            };
            idt.0[vector] = Gate::interrupt(handler, ist);
        }
        idt
    });
    let ptr = DescriptorPointer {
        limit: (size_of::<Idt>() - 1) as u16,
        base: idt as *const _ as u64,
    };
    unsafe { x86::lidt(&ptr) };
}

#[unsafe(no_mangle)]
extern "C" fn trap_dispatch(frame: &mut TrapFrame) {
    match frame.vector {
        vector::BREAKPOINT => {
            crate::debug!("breakpoint at {:#x}", frame.rip);
            crate::ktest_hooks::breakpoint_hit();
        }
        vector::PAGE_FAULT => {
            let addr = x86::read_cr2();
            panic!(
                "page fault at {addr:#x} ({}{}{}) rip={:#x}\n{frame:#x?}",
                if frame.error_code & 1 != 0 {
                    "protection violation"
                } else {
                    "not present"
                },
                if frame.error_code & 2 != 0 {
                    ", write"
                } else {
                    ", read"
                },
                if frame.error_code & 4 != 0 {
                    ", user"
                } else {
                    ", kernel"
                },
                frame.rip,
            );
        }
        v if v == super::apic::TIMER_VECTOR as u64 => {
            super::apic::eoi();
            let cpu = super::percpu::this_cpu();
            cpu.ticks
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cpu.index == 0 {
                crate::time::tick();
            }
            crate::task::sched::preempt();
        }
        v if v == super::apic::SPURIOUS_VECTOR as u64 => {}
        v @ 0..32 => panic!(
            "CPU exception {v}: {} (error code {:#x}) rip={:#x}\n{frame:#x?}",
            EXCEPTION_NAMES[v as usize], frame.error_code, frame.rip
        ),
        v => crate::warn!("unexpected interrupt vector {v}"),
    }
}
