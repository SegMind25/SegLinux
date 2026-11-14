//! Kernel heap: `Box`, `Vec`, `String` and friends.
//!
//! Our own first-fit allocator over a list of free blocks kept in address
//! order, so neighbours merge back together on free. The heap lives in a
//! fixed virtual window, starts at 4 MiB and grows by mapping more pages
//! when an allocation does not fit. Size-class slabs can sit in front of it
//! once allocation patterns are known.

use crate::sync::SpinLock;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

pub const HEAP_START: u64 = 0xffff_c000_0000_0000;
const HEAP_MAX: u64 = 1 << 30; // 1 GiB window
const INITIAL: u64 = 4 << 20;
const GROW_STEP: u64 = 4 << 20;
/// Every block is a multiple of this, so a free block can hold its header.
const UNIT: usize = 16;

/// Header written into each free block.
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct Heap {
    /// Free blocks sorted by address.
    head: *mut FreeBlock,
    /// Bytes of the window that are mapped.
    size: usize,
    used: usize,
}

unsafe impl Send for Heap {}

impl Heap {
    const fn empty() -> Self {
        Self {
            head: null_mut(),
            size: 0,
            used: 0,
        }
    }

    fn block_size(layout: Layout) -> usize {
        layout.size().max(1).next_multiple_of(UNIT)
    }

    /// Adds `[addr, addr + size)` to the free list, merging with neighbours.
    unsafe fn free_range(&mut self, addr: usize, size: usize) {
        unsafe {
            let mut prev: *mut FreeBlock = null_mut();
            let mut cur = self.head;
            while !cur.is_null() && (cur as usize) < addr {
                prev = cur;
                cur = (*cur).next;
            }
            let block = addr as *mut FreeBlock;
            block.write(FreeBlock { size, next: cur });
            if !cur.is_null() && addr + size == cur as usize {
                (*block).size += (*cur).size;
                (*block).next = (*cur).next;
            }
            if prev.is_null() {
                self.head = block;
            } else if prev as usize + (*prev).size == addr {
                (*prev).size += (*block).size;
                (*prev).next = (*block).next;
            } else {
                (*prev).next = block;
            }
        }
    }

    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = Self::block_size(layout);
        let align = layout.align().max(UNIT);
        unsafe {
            let mut prev: *mut FreeBlock = null_mut();
            let mut cur = self.head;
            while !cur.is_null() {
                let start = cur as usize;
                let end = start + (*cur).size;
                let aligned = start.next_multiple_of(align);
                if aligned + size <= end {
                    let next = (*cur).next;
                    // Unlink, then give back what is left on either side.
                    if prev.is_null() {
                        self.head = next;
                    } else {
                        (*prev).next = next;
                    }
                    if aligned > start {
                        self.free_range(start, aligned - start);
                    }
                    if aligned + size < end {
                        self.free_range(aligned + size, end - (aligned + size));
                    }
                    self.used += size;
                    return aligned as *mut u8;
                }
                prev = cur;
                cur = (*cur).next;
            }
        }
        null_mut()
    }

    /// Maps `bytes` more of the window and frees them into the list.
    fn grow(&mut self, bytes: u64) -> bool {
        let bytes = bytes.next_multiple_of(super::PAGE_SIZE);
        let start = HEAP_START + self.size as u64;
        if self.size as u64 + bytes > HEAP_MAX
            || super::paging::map_new(start, bytes / super::PAGE_SIZE).is_err()
        {
            return false;
        }
        self.size += bytes as usize;
        unsafe { self.free_range(start as usize, bytes as usize) };
        true
    }
}

struct KernelHeap(SpinLock<Heap>);

#[global_allocator]
static ALLOCATOR: KernelHeap = KernelHeap(SpinLock::new(Heap::empty()));

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = self.0.lock();
        loop {
            let ptr = heap.alloc(layout);
            if !ptr.is_null() {
                return ptr;
            }
            let need = (Heap::block_size(layout) + layout.align()) as u64;
            if !heap.grow(GROW_STEP.max(need)) {
                return null_mut();
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut heap = self.0.lock();
        let size = Heap::block_size(layout);
        heap.used -= size;
        unsafe { heap.free_range(ptr as usize, size) };
    }
}

pub fn init() {
    assert!(ALLOCATOR.0.lock().grow(INITIAL), "heap: cannot map");
    crate::info!("heap: {} KiB at {HEAP_START:#x}", INITIAL / 1024);
}

/// (bytes in use, bytes mapped)
pub fn stats() -> (usize, usize) {
    let heap = ALLOCATOR.0.lock();
    (heap.used, heap.size)
}
