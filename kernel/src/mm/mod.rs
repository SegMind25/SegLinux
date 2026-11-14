//! Memory management: physical frames, page tables, kernel heap.

pub mod frame;
pub mod heap;
pub mod paging;

use crate::limine::{MemmapResponse, memmap_kind};
use core::sync::atomic::{AtomicU64, Ordering};

pub const PAGE_SIZE: u64 = 4096;

/// Virtual address where all physical memory is mapped (Limine's HHDM).
static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);

pub fn hhdm_offset() -> u64 {
    HHDM_OFFSET.load(Ordering::Relaxed)
}

/// Physical address to a pointer through the direct map.
pub fn phys_to_virt(phys: u64) -> *mut u8 {
    (phys + hhdm_offset()) as *mut u8
}

pub fn init(memmap: &MemmapResponse, hhdm: u64) {
    HHDM_OFFSET.store(hhdm, Ordering::Relaxed);

    let mut usable = 0;
    for e in memmap.entries() {
        log::debug!(
            "  {:#014x}-{:#014x} {}",
            e.base,
            e.base + e.length,
            memmap_kind::name(e.kind)
        );
        if e.kind == memmap_kind::USABLE {
            usable += e.length;
        }
    }
    log::info!("memory: {} MiB usable", usable / (1024 * 1024));

    frame::init(memmap);
    paging::init();
    heap::init();
}
