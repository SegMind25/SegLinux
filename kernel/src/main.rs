//! segkernel: the SegOS kernel.
//!
//! Boot order: serial → logger → CPU tables → memory → screen console.
//! See docs/plan.md for where this is going.

#![no_std]
#![no_main]
// Phase 0 defines interfaces (user segments, unmap, …) that later phases use.
#![allow(dead_code)]

extern crate alloc;

#[macro_use]
mod console;
mod hal;
#[cfg(feature = "ktest")]
mod ktest;
mod ktest_hooks;
mod limine;
mod mm;
mod sync;
mod task;
mod time;

use core::panic::PanicInfo;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Entry point, called by Limine on the bootstrap CPU with interrupts off,
/// on a bootloader-provided stack.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    hal::serial::init();
    console::init_logger();
    kprintln!("SegOS {VERSION}");

    if !limine::BASE_REVISION.is_supported() {
        panic!("the bootloader does not support Limine base revision 3");
    }

    hal::init();
    log::info!("cpu: GDT, TSS and IDT loaded");

    let hhdm = limine::HHDM.response().expect("no HHDM response").offset;
    let memmap = limine::MEMMAP.response().expect("no memory map");
    mm::init(memmap, hhdm);

    if let Some(kaddr) = limine::EXECUTABLE_ADDRESS.response() {
        log::info!(
            "kernel: loaded at phys {:#x}, virt {:#x}",
            kaddr.physical_base,
            kaddr.virtual_base
        );
    }

    // Screen console last: it needs nothing, but the boot log above is
    // on serial already if anything before this point fails.
    match limine::FRAMEBUFFER
        .response()
        .and_then(|r| r.framebuffers().first())
    {
        Some(fb) => {
            console::init(fb);
            kprintln!("SegOS {VERSION}");
            log::info!("console: {}x{} framebuffer", fb.width, fb.height);
        }
        None => log::warn!("console: no framebuffer, serial only"),
    }

    // Phase 1: this CPU's data and timer, the other CPUs, the scheduler.
    hal::percpu::init(0, hal::apic::bsp_id_from_bootloader());
    hal::apic::init_bsp();
    task::sched::init_cpu();
    hal::smp::start_aps();

    #[cfg(feature = "ktest")]
    task::spawn_named("ktest", || ktest::run());
    #[cfg(not(feature = "ktest"))]
    task::spawn_named("init", kernel_init);

    // The boot context becomes this CPU's idle task.
    task::sched::idle_loop()
}

/// First kernel thread. Starts PID 1 from Phase 2 on; for now it reports.
#[cfg(not(feature = "ktest"))]
fn kernel_init() {
    let (used, size) = mm::heap::stats();
    log::info!(
        "ready: {} CPUs, {} MiB free, heap {used}/{size} bytes",
        hal::percpu::count(),
        mm::frame::free_count() * mm::PAGE_SIZE / (1024 * 1024)
    );
    time::sleep_ms(1000);
    log::info!(
        "uptime {} ms, timer ticks per CPU: {:?}",
        time::uptime_ms(),
        hal::percpu::ticks()
    );
    log::info!("Phase 1: scheduler running; nothing more to run yet.");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    x86_64::instructions::interrupts::disable();
    console::print_panic(format_args!("\nKERNEL PANIC: {}\n", info));
    console::print_panic(format_args!(
        "backtrace (llvm-addr2line -e segkernel <addr>):\n"
    ));
    hal::cpu::backtrace(|depth, addr| {
        console::print_panic(format_args!("  #{depth:<2} {addr:#018x}\n"));
    });
    if cfg!(feature = "ktest") {
        hal::cpu::exit_qemu(hal::cpu::QemuExit::Failure);
    }
    hal::cpu::halt_forever()
}
