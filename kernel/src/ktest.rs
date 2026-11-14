//! In-kernel tests: `cargo xtask test` boots the kernel with the `ktest`
//! feature in QEMU; it runs these and exits QEMU with a success/failure code.
//! A failing test panics, and the panic handler reports failure.

use crate::hal::cpu::{QemuExit, exit_qemu};
use crate::mm;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

const TESTS: &[(&str, fn())] = &[
    ("breakpoint exception returns", breakpoint_returns),
    ("frames: alloc, zeroed, free", frames_alloc_free),
    ("frames: distinct", frames_distinct),
    ("paging: map, translate, unmap", paging_map_unmap),
    ("heap: Box and String", heap_box_string),
    ("heap: large Vec grows the heap", heap_grows),
    ("heap: many small allocations are reused", heap_reuse),
];

pub fn run() -> ! {
    kprintln!("running {} kernel tests", TESTS.len());
    for (name, test) in TESTS {
        crate::console::print(format_args!("test {name} ... "));
        test();
        kprintln!("ok");
    }
    kprintln!("all {} tests passed", TESTS.len());
    exit_qemu(QemuExit::Success)
}

fn breakpoint_returns() {
    let before = crate::ktest_hooks::breakpoints();
    x86_64::instructions::interrupts::int3();
    assert_eq!(crate::ktest_hooks::breakpoints(), before + 1);
}

fn frames_alloc_free() {
    let free = mm::frame::free_count();
    let phys = mm::frame::alloc_zeroed().expect("out of frames");
    assert_eq!(phys % mm::PAGE_SIZE, 0);
    let page = unsafe { core::slice::from_raw_parts(mm::phys_to_virt(phys), 4096) };
    assert!(page.iter().all(|&b| b == 0));
    assert_eq!(mm::frame::free_count(), free - 1);
    mm::frame::free(phys);
    assert_eq!(mm::frame::free_count(), free);
}

fn frames_distinct() {
    let frames: Vec<u64> = (0..64)
        .map(|_| mm::frame::alloc_zeroed().unwrap())
        .collect();
    let mut sorted = frames.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), frames.len());
    frames.into_iter().for_each(mm::frame::free);
}

fn paging_map_unmap() {
    const VIRT: u64 = 0xffff_d000_0000_0000;
    let free = mm::frame::free_count();
    mm::paging::map_new(VIRT, 1).unwrap();
    assert!(mm::paging::translate(VIRT).is_some());
    unsafe {
        (VIRT as *mut u64).write_volatile(0x5e605);
        assert_eq!((VIRT as *const u64).read_volatile(), 0x5e605);
    }
    assert!(matches!(
        mm::paging::map_new(VIRT, 1),
        Err(mm::paging::MapError::AlreadyMapped)
    ));
    mm::paging::unmap(VIRT);
    assert!(mm::paging::translate(VIRT).is_none());
    // Intermediate page tables stay allocated; only the data frame returns.
    assert!(mm::frame::free_count() >= free - 3);
}

fn heap_box_string() {
    let b = Box::new(41u64);
    assert_eq!(*b + 1, 42);
    let mut s = String::from("Seg");
    s.push_str("OS");
    assert_eq!(s, "SegOS");
}

fn heap_grows() {
    let (_, before) = mm::heap::stats();
    let v: Vec<u8> = alloc::vec![7; 8 << 20]; // 8 MiB > initial 4 MiB heap
    assert!(v.iter().all(|&b| b == 7));
    let (_, after) = mm::heap::stats();
    assert!(after > before);
}

fn heap_reuse() {
    let (used_before, _) = mm::heap::stats();
    for i in 0..10_000u64 {
        let b = Box::new(i);
        assert_eq!(*b, i);
    }
    assert_eq!(mm::heap::stats().0, used_before);
}
