//! In-kernel tests: `cargo xtask test` boots the kernel with the `ktest`
//! feature in QEMU; it runs these and exits QEMU with a success/failure code.
//! A failing test panics, and the panic handler reports failure.

use crate::hal::cpu::{QemuExit, exit_qemu};
use crate::mm;
use crate::{hal::percpu, sync::Mutex, task, time};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const TESTS: &[(&str, fn())] = &[
    ("breakpoint exception returns", breakpoint_returns),
    ("frames: alloc, zeroed, free", frames_alloc_free),
    ("frames: distinct", frames_distinct),
    ("paging: map, translate, unmap", paging_map_unmap),
    ("heap: Box and String", heap_box_string),
    ("heap: large Vec grows the heap", heap_grows),
    ("heap: many small allocations are reused", heap_reuse),
    ("timer: uptime advances while sleeping", timer_advances),
    ("smp: every CPU is online and ticking", smp_online),
    ("sched: threads run on several CPUs", threads_spread),
    (
        "sched: more spinning threads than CPUs (preemption)",
        preemption,
    ),
    (
        "sync: Mutex keeps an exact count under contention",
        mutex_count,
    ),
    (
        "sched: 200 short threads start, exit and are freed",
        thread_churn,
    ),
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
    crate::hal::x86::breakpoint();
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

fn timer_advances() {
    let start = time::uptime_ms();
    time::sleep_ms(100);
    let elapsed = time::uptime_ms() - start;
    assert!(elapsed >= 100, "slept {elapsed} ms");
}

fn smp_online() {
    let expected = crate::hal::smp::cpu_count_from_acpi();
    assert_eq!(percpu::count(), expected);
    let before = percpu::ticks();
    time::sleep_ms(50);
    for ((cpu, a), (_, b)) in before.iter().zip(percpu::ticks()) {
        assert!(b > *a, "CPU {cpu} timer is not ticking");
    }
}

fn threads_spread() {
    let cpus_seen = Arc::new(AtomicU64::new(0));
    let threads: Vec<_> = (0..percpu::count() * 4)
        .map(|_| {
            let seen = cpus_seen.clone();
            task::spawn(move || {
                for _ in 0..20 {
                    seen.fetch_or(1 << percpu::this_cpu().index, Ordering::Relaxed);
                    spin_for_ms(2);
                }
            })
        })
        .collect();
    threads.iter().for_each(|t| t.join());
    let used = cpus_seen.load(Ordering::Relaxed).count_ones() as usize;
    if percpu::count() > 1 {
        assert!(used > 1, "all threads ran on one CPU");
    }
}

/// n spinning threads, more than CPUs, each waiting (without yielding) until
/// all have started: only possible if the timer preempts them.
fn preemption() {
    let n = percpu::count() + 2;
    let started = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..n)
        .map(|_| {
            let started = started.clone();
            task::spawn(move || {
                started.fetch_add(1, Ordering::SeqCst);
                while started.load(Ordering::SeqCst) < n {
                    core::hint::spin_loop();
                }
            })
        })
        .collect();
    threads.iter().for_each(|t| t.join());
    assert_eq!(started.load(Ordering::SeqCst), n);
}

fn mutex_count() {
    const THREADS: u64 = 8;
    const ROUNDS: u64 = 500;
    let counter = Arc::new(Mutex::new(0u64));
    let threads: Vec<_> = (0..THREADS)
        .map(|_| {
            let counter = counter.clone();
            task::spawn(move || {
                for i in 0..ROUNDS {
                    let mut c = counter.lock();
                    let v = *c;
                    if i % 50 == 0 {
                        task::yield_now(); // hold the lock across a switch
                    }
                    *c = v + 1;
                }
            })
        })
        .collect();
    threads.iter().for_each(|t| t.join());
    assert_eq!(*counter.lock(), THREADS * ROUNDS);
}

fn thread_churn() {
    let free = mm::frame::free_count();
    let done = Arc::new(AtomicUsize::new(0));
    for _ in 0..10 {
        let batch: Vec<_> = (0..20)
            .map(|_| {
                let done = done.clone();
                task::spawn(move || {
                    done.fetch_add(1, Ordering::Relaxed);
                })
            })
            .collect();
        batch.iter().for_each(|t| t.join());
    }
    assert_eq!(done.load(Ordering::Relaxed), 200);
    // Let other CPUs finish switching away from the last ones.
    time::sleep_ms(20);
    // Stacks (16 frames each) are returned; page-table pages may remain.
    let lost = free.saturating_sub(mm::frame::free_count());
    assert!(lost < 64, "{lost} frames not returned");
}

fn spin_for_ms(ms: u64) {
    let until = time::uptime_ms() + ms;
    while time::uptime_ms() < until {
        core::hint::spin_loop();
    }
}
