//! Global descriptor table and task state segment, one pair per CPU.
//!
//! Long mode barely uses segmentation, but the CPU still needs code/data
//! selectors, and a TSS for the interrupt stacks (IST) and, from Phase 2,
//! the ring-0 stack used when user code traps into the kernel.

use super::x86::{self, DescriptorPointer};
use alloc::boxed::Box;

// Order matters for `syscall`/`sysret` (Phase 2): kernel code, kernel data,
// then user data before user code.
pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
pub const USER_DATA: u16 = 0x18 | 3;
pub const USER_CODE: u16 = 0x20 | 3;
pub const TSS: u16 = 0x28;

/// IST slot (1-based in the IDT, 0-based here) for the double-fault
/// handler, so a kernel stack overflow still produces a readable report.
pub const DOUBLE_FAULT_IST: usize = 0;
const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(C, packed(4))]
pub struct TaskStateSegment {
    _reserved0: u32,
    pub rsp: [u64; 3],
    _reserved1: u64,
    pub ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            _reserved0: 0,
            rsp: [0; 3],
            _reserved1: 0,
            ist: [0; 7],
            _reserved2: 0,
            _reserved3: 0,
            iomap_base: size_of::<Self>() as u16, // no I/O permission bitmap
        }
    }
}

#[repr(C, align(16))]
struct Gdt([u64; 7]);

impl Gdt {
    fn new(tss: &'static TaskStateSegment) -> Self {
        let base = tss as *const _ as u64;
        let limit = (size_of::<TaskStateSegment>() - 1) as u64;
        let tss_low = (limit & 0xffff)
            | (base & 0xff_ffff) << 16
            | 0x89 << 40 // present, 64-bit TSS (available)
            | ((limit >> 16) & 0xf) << 48
            | ((base >> 24) & 0xff) << 56;
        Self([
            0,
            0x00af_9a00_0000_ffff, // kernel code: present, ring 0, executable, long mode
            0x00cf_9200_0000_ffff, // kernel data
            0x00cf_f200_0000_ffff, // user data, ring 3
            0x00af_fa00_0000_ffff, // user code, ring 3, long mode
            tss_low,
            base >> 32,
        ])
    }
}

#[repr(align(16))]
struct Stack([u8; IST_STACK_SIZE]);

static mut BSP_DOUBLE_FAULT_STACK: Stack = Stack([0; IST_STACK_SIZE]);
static mut BSP_TSS: TaskStateSegment = TaskStateSegment::new();
static mut BSP_GDT: Gdt = Gdt([0; 7]);

/// Bootstrap CPU: static tables, usable before the heap exists.
pub fn init_bsp() {
    // SAFETY: runs once, on the BSP, before anything else uses these.
    unsafe {
        let tss = &mut *(&raw mut BSP_TSS);
        tss.ist[DOUBLE_FAULT_IST] =
            (&raw const BSP_DOUBLE_FAULT_STACK) as u64 + IST_STACK_SIZE as u64;
        let gdt = &mut *(&raw mut BSP_GDT);
        *gdt = Gdt::new(&*(&raw const BSP_TSS));
        load(gdt);
    }
}

/// Application processors: tables on the heap, never freed.
pub fn init_ap() {
    let stack = Box::leak(alloc::vec![0u8; IST_STACK_SIZE].into_boxed_slice());
    let mut tss = TaskStateSegment::new();
    tss.ist[DOUBLE_FAULT_IST] = stack.as_ptr() as u64 + IST_STACK_SIZE as u64;
    let tss: &'static TaskStateSegment = Box::leak(Box::new(tss));
    load(Box::leak(Box::new(Gdt::new(tss))));
}

fn load(gdt: &'static Gdt) {
    let ptr = DescriptorPointer {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: gdt as *const _ as u64,
    };
    unsafe {
        x86::lgdt(&ptr);
        x86::load_segments(KERNEL_CODE, KERNEL_DATA);
        x86::ltr(TSS);
    }
}
