//! Uptime from the bootstrap CPU's timer ticks.
//!
//! Resolution is one tick (1/HZ s). A TSC- or HPET-based clock with
//! nanoseconds comes with `clock_gettime` in Phase 2.

use crate::hal::apic::HZ;
use core::sync::atomic::{AtomicU64, Ordering};

static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn uptime_ms() -> u64 {
    TICKS.load(Ordering::Relaxed) * 1000 / HZ
}

/// Sleeps at least `ms` milliseconds, yielding the CPU meanwhile.
/// (Yields in a loop; a timer wheel that blocks the thread comes later.)
pub fn sleep_ms(ms: u64) {
    let until = uptime_ms() + ms;
    while uptime_ms() < until {
        crate::task::yield_now();
        core::hint::spin_loop();
    }
}
