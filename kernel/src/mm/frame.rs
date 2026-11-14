//! Physical frame allocator: one bit per 4 KiB frame.
//!
//! Simple and good enough until SMP and NUMA (a buddy allocator with
//! per-CPU caches replaces it in Phase 1/4). The bitmap itself lives in the
//! first usable region large enough, reached through the direct map.

use super::{PAGE_SIZE, phys_to_virt};
use crate::limine::{MemmapResponse, memmap_kind};
use crate::sync::SpinLock;

pub struct FrameAllocator {
    bitmap: &'static mut [u64],
    frames: u64,
    free: u64,
    /// Search hint: no free frame below this index.
    next: u64,
}

static ALLOCATOR: SpinLock<Option<FrameAllocator>> = SpinLock::new(None);

impl FrameAllocator {
    fn is_used(&self, i: u64) -> bool {
        self.bitmap[(i / 64) as usize] & (1 << (i % 64)) != 0
    }

    fn set(&mut self, i: u64, used: bool) {
        let word = &mut self.bitmap[(i / 64) as usize];
        if used {
            *word |= 1 << (i % 64);
        } else {
            *word &= !(1 << (i % 64));
        }
    }

    fn alloc(&mut self) -> Option<u64> {
        let words = self.bitmap.len() as u64;
        let mut w = self.next / 64;
        while w < words {
            let word = self.bitmap[w as usize];
            if word != u64::MAX {
                let i = w * 64 + (!word).trailing_zeros() as u64;
                if i >= self.frames {
                    return None;
                }
                self.set(i, true);
                self.free -= 1;
                self.next = i + 1;
                return Some(i * PAGE_SIZE);
            }
            w += 1;
        }
        None
    }

    fn dealloc(&mut self, phys: u64) {
        let i = phys / PAGE_SIZE;
        assert!(
            i < self.frames && self.is_used(i),
            "double free of frame {phys:#x}"
        );
        self.set(i, false);
        self.free += 1;
        self.next = self.next.min(i);
    }
}

pub fn init(memmap: &MemmapResponse) {
    let entries = memmap.entries();
    let top = entries
        .iter()
        .filter(|e| e.kind == memmap_kind::USABLE)
        .map(|e| e.base + e.length)
        .max()
        .expect("no usable memory");
    let frames = top / PAGE_SIZE;
    let words = frames.div_ceil(64);
    let bytes = words * 8;

    let home = entries
        .iter()
        .find(|e| e.kind == memmap_kind::USABLE && e.length >= bytes)
        .expect("no region large enough for the frame bitmap");
    let bitmap = unsafe {
        core::slice::from_raw_parts_mut(phys_to_virt(home.base).cast::<u64>(), words as usize)
    };
    bitmap.fill(u64::MAX);

    let mut fa = FrameAllocator {
        bitmap,
        frames,
        free: 0,
        next: 0,
    };
    for e in entries.iter().filter(|e| e.kind == memmap_kind::USABLE) {
        // Usable regions are page aligned by the protocol.
        for i in e.base / PAGE_SIZE..(e.base + e.length) / PAGE_SIZE {
            fa.set(i, false);
            fa.free += 1;
        }
    }
    // The bitmap's own frames, and frame 0 (keeps null physical addresses invalid).
    for i in home.base / PAGE_SIZE..(home.base + bytes).div_ceil(PAGE_SIZE) {
        fa.set(i, true);
        fa.free -= 1;
    }
    if !fa.is_used(0) {
        fa.set(0, true);
        fa.free -= 1;
    }
    log::info!(
        "frames: {} free of {} ({} KiB bitmap)",
        fa.free,
        frames,
        bytes / 1024
    );
    *ALLOCATOR.lock() = Some(fa);
}

/// Allocates one zeroed 4 KiB frame; returns its physical address.
pub fn alloc_zeroed() -> Option<u64> {
    let phys = ALLOCATOR.lock().as_mut()?.alloc()?;
    unsafe { core::ptr::write_bytes(phys_to_virt(phys), 0, PAGE_SIZE as usize) };
    Some(phys)
}

pub fn free(phys: u64) {
    ALLOCATOR
        .lock()
        .as_mut()
        .expect("frame allocator not ready")
        .dealloc(phys);
}

pub fn free_count() -> u64 {
    ALLOCATOR.lock().as_ref().map_or(0, |a| a.free)
}

/// Adapter for the x86_64 crate's page table code.
pub struct Frames;

unsafe impl x86_64::structures::paging::FrameAllocator<x86_64::structures::paging::Size4KiB>
    for Frames
{
    fn allocate_frame(&mut self) -> Option<x86_64::structures::paging::PhysFrame> {
        let phys = alloc_zeroed()?;
        Some(x86_64::structures::paging::PhysFrame::containing_address(
            x86_64::PhysAddr::new(phys),
        ))
    }
}
