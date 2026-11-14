//! Per-CPU data, reached through the GS base register.

use super::x86;
use crate::sync::Once;
use crate::task::Task;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
pub struct PerCpu {
    /// Must stay first: `this_cpu` reads it from `gs:0`.
    self_ptr: *const PerCpu,
    pub index: u32,
    pub lapic_id: u32,
    pub ticks: AtomicU64,
    /// Touched only by this CPU, with interrupts disabled.
    pub(crate) current: UnsafeCell<Option<Arc<Task>>>,
    pub(crate) idle: Once<Arc<Task>>,
}

unsafe impl Sync for PerCpu {}

static CPUS: crate::sync::SpinLock<Vec<&'static PerCpu>> = crate::sync::SpinLock::new(Vec::new());

/// Creates this CPU's block and points GS at it.
pub fn init(index: u32, lapic_id: u32) {
    let cpu: &'static mut PerCpu = Box::leak(Box::new(PerCpu {
        self_ptr: core::ptr::null(),
        index,
        lapic_id,
        ticks: AtomicU64::new(0),
        current: UnsafeCell::new(None),
        idle: Once::new(),
    }));
    cpu.self_ptr = cpu;
    unsafe { x86::wrmsr(x86::msr::GS_BASE, cpu as *const PerCpu as u64) };
    CPUS.lock().push(cpu);
}

pub fn this_cpu() -> &'static PerCpu {
    let ptr: *const PerCpu;
    unsafe {
        core::arch::asm!("mov {}, gs:[0]", out(reg) ptr, options(nostack, readonly, preserves_flags))
    };
    unsafe { &*ptr }
}

pub fn count() -> usize {
    CPUS.lock().len()
}

/// Timer ticks each CPU has handled, in CPU index order.
pub fn ticks() -> Vec<(u32, u64)> {
    let mut v: Vec<_> = CPUS
        .lock()
        .iter()
        .map(|c| (c.index, c.ticks.load(Ordering::Relaxed)))
        .collect();
    v.sort_unstable();
    v
}
