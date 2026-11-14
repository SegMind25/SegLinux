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
mod font;
mod hal;
#[macro_use]
mod klog;
#[cfg(feature = "ktest")]
mod ktest;
mod ktest_hooks;
mod mm;
mod sync;
mod task;
mod time;

use bootinfo::BootInfo;
use core::panic::PanicInfo;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Entry point, jumped to by segboot (our UEFI bootloader) on the bootstrap
/// CPU, interrupts off, on a boot stack. See the `bootinfo` crate.
#[unsafe(no_mangle)]
extern "sysv64" fn kmain(boot: &'static BootInfo) -> ! {
    hal::serial::init();
    kprintln!("SegOS {VERSION}");
    if boot.magic != bootinfo::MAGIC || boot.version != bootinfo::VERSION {
        panic!(
            "not started by segboot (boot protocol {:#x} v{})",
            boot.magic, boot.version
        );
    }

    hal::init();
    info!("cpu: GDT, TSS and IDT loaded");

    mm::init(boot.memory_map(), boot.hhdm_offset);
    info!(
        "kernel: loaded at phys {:#x}, virt {:#x}",
        boot.kernel_phys_base, boot.kernel_virt_base
    );

    // Screen console last: the boot log above is on serial already if
    // anything before this point fails.
    console::init(&boot.framebuffer);
    if boot.framebuffer.address != 0 {
        kprintln!("SegOS {VERSION}");
        info!(
            "console: {}x{} framebuffer",
            boot.framebuffer.width, boot.framebuffer.height
        );
    }

    // This CPU's timer and data, the other CPUs, the scheduler.
    hal::apic::init_bsp();
    hal::percpu::init(0, hal::apic::id());
    task::sched::init_cpu();
    hal::smp::start_aps(boot.rsdp);

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
    info!(
        "ready: {} CPUs, {} MiB free, heap {used}/{size} bytes",
        hal::percpu::count(),
        mm::frame::free_count() * mm::PAGE_SIZE / (1024 * 1024)
    );
    time::sleep_ms(1000);
    info!(
        "uptime {} ms, timer ticks per CPU: {:?}",
        time::uptime_ms(),
        hal::percpu::ticks()
    );
    info!("Phase 1: scheduler running; nothing more to run yet.");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    hal::x86::disable_interrupts();
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
