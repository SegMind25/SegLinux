//! Starting the application processors (APs).
//!
//! Limine has already brought each AP into long mode with the kernel's page
//! tables and a stack; it waits for us to write an entry point into its
//! `goto_address`.

use super::{apic, gdt, idt, percpu};
use crate::limine::{MP, MpInfo};
use core::sync::atomic::{AtomicU32, Ordering};

static ONLINE: AtomicU32 = AtomicU32::new(1);

/// Starts every AP and waits until all of them run their idle loop.
pub fn start_aps() -> u32 {
    let Some(mp) = MP.response() else {
        log::warn!("smp: bootloader gave no CPU list, running on one CPU");
        return 1;
    };
    let mut index = 1;
    for &info in mp.cpus() {
        let info = unsafe { &mut *info };
        if info.lapic_id == mp.bsp_lapic_id {
            continue;
        }
        info.extra_argument = index as u64;
        info.goto_address
            .store(ap_entry as *const () as u64, Ordering::SeqCst);
        index += 1;
    }
    while ONLINE.load(Ordering::Acquire) < index {
        core::hint::spin_loop();
    }
    log::info!("smp: {index} CPUs online");
    index
}

extern "C" fn ap_entry(info: *const MpInfo) -> ! {
    let info = unsafe { &*info };
    gdt::init_ap();
    idt::init();
    percpu::init(info.extra_argument as u32, info.lapic_id);
    apic::init_ap();
    crate::task::sched::init_cpu();
    ONLINE.fetch_add(1, Ordering::Release);
    crate::task::sched::idle_loop()
}
