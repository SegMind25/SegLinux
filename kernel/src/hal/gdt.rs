//! Global descriptor table and task state segment.
//!
//! Long mode barely uses segmentation, but the CPU still needs code/data
//! selectors, and a TSS for the interrupt stacks (IST) and, from Phase 2,
//! the ring-0 stack used when user code traps into the kernel.

use spin::Lazy;
use x86_64::VirtAddr;
use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

/// IST slot used by the double-fault handler, so a kernel stack overflow
/// still produces a readable report instead of a triple fault.
pub const DOUBLE_FAULT_IST: u16 = 0;
const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(align(16))]
struct Stack([u8; IST_STACK_SIZE]);
static mut DOUBLE_FAULT_STACK: Stack = Stack([0; IST_STACK_SIZE]);

static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    let start = VirtAddr::from_ptr(&raw const DOUBLE_FAULT_STACK);
    tss.interrupt_stack_table[DOUBLE_FAULT_IST as usize] = start + IST_STACK_SIZE as u64;
    tss
});

pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    tss: SegmentSelector,
}

// Order matters for `syscall`/`sysret` (Phase 2): kernel code, kernel data,
// then user data before user code.
static GDT: Lazy<(GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());
    let tss = gdt.append(Descriptor::tss_segment(&TSS));
    (
        gdt,
        Selectors {
            kernel_code,
            kernel_data,
            user_data,
            user_code,
            tss,
        },
    )
});

pub fn selectors() -> &'static Selectors {
    &GDT.1
}

pub fn init() {
    GDT.0.load();
    let s = &GDT.1;
    unsafe {
        CS::set_reg(s.kernel_code);
        DS::set_reg(s.kernel_data);
        ES::set_reg(s.kernel_data);
        SS::set_reg(s.kernel_data);
        load_tss(s.tss);
    }
}
