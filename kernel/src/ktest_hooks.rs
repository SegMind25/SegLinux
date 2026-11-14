//! Counters the test suite reads; free in normal builds.

use core::sync::atomic::{AtomicU64, Ordering};

static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);

pub fn breakpoint_hit() {
    BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn breakpoints() -> u64 {
    BREAKPOINTS.load(Ordering::Relaxed)
}
