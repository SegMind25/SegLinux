//! Kernel heap: `Box`, `Vec`, `String` and friends.
//!
//! A linked-list allocator over a fixed virtual window that is mapped on
//! demand: it starts at 4 MiB and grows when an allocation fails.
//! A slab allocator replaces it once allocation patterns are known.

use crate::sync::SpinLock;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use linked_list_allocator::Heap;

pub const HEAP_START: u64 = 0xffff_c000_0000_0000;
const HEAP_MAX: u64 = 1 << 30; // 1 GiB window
const INITIAL: u64 = 4 << 20;
const GROW_STEP: u64 = 4 << 20;

struct KernelHeap {
    heap: SpinLock<Heap>,
}

#[global_allocator]
static ALLOCATOR: KernelHeap = KernelHeap {
    heap: SpinLock::new(Heap::empty()),
};

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = self.heap.lock();
        loop {
            if let Ok(ptr) = heap.allocate_first_fit(layout) {
                return ptr.as_ptr();
            }
            let size = heap.size() as u64;
            let grow = GROW_STEP
                .max((layout.size() + layout.align()) as u64)
                .next_multiple_of(4096);
            if size + grow > HEAP_MAX
                || super::paging::map_new(HEAP_START + size, grow / super::PAGE_SIZE).is_err()
            {
                return core::ptr::null_mut();
            }
            unsafe { heap.extend(grow as usize) };
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            self.heap
                .lock()
                .deallocate(NonNull::new_unchecked(ptr), layout)
        }
    }
}

pub fn init() {
    super::paging::map_new(HEAP_START, INITIAL / super::PAGE_SIZE).expect("heap: cannot map");
    unsafe {
        ALLOCATOR
            .heap
            .lock()
            .init(HEAP_START as *mut u8, INITIAL as usize)
    };
    log::info!("heap: {} KiB at {HEAP_START:#x}", INITIAL / 1024);
}

pub fn stats() -> (usize, usize) {
    let heap = ALLOCATOR.heap.lock();
    (heap.used(), heap.size())
}
