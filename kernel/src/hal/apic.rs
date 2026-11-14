//! Local APIC: per-CPU interrupt controller and timer.
//!
//! Every CPU sees its own local APIC at the same physical address, so one
//! mapping serves all of them. The timer runs periodically at [`HZ`] and
//! drives timekeeping and preemption. Its frequency is unknown, so the
//! bootstrap CPU measures it once against the legacy PIT.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use x86_64::instructions::port::Port;
use x86_64::registers::model_specific::Msr;

pub const TIMER_VECTOR: u8 = 0x20;
pub const SPURIOUS_VECTOR: u8 = 0xff;
/// Timer interrupts per second, per CPU.
pub const HZ: u64 = 100;

const IA32_APIC_BASE: u32 = 0x1b;
const MMIO_VIRT: u64 = 0xffff_e000_0000_0000;

const REG_ID: u32 = 0x20;
const REG_TPR: u32 = 0x80;
const REG_EOI: u32 = 0xb0;
const REG_SVR: u32 = 0xf0;
const REG_LVT_TIMER: u32 = 0x320;
const REG_TIMER_INIT: u32 = 0x380;
const REG_TIMER_CURRENT: u32 = 0x390;
const REG_TIMER_DIVIDE: u32 = 0x3e0;

const TIMER_PERIODIC: u32 = 1 << 17;
const LVT_MASKED: u32 = 1 << 16;
const DIVIDE_BY_16: u32 = 0x3;

static BASE: AtomicU64 = AtomicU64::new(0);
/// Timer counts per tick at divide-by-16, measured on the BSP.
static COUNTS_PER_TICK: AtomicU32 = AtomicU32::new(0);

fn read(reg: u32) -> u32 {
    unsafe { ((BASE.load(Ordering::Relaxed) + reg as u64) as *const u32).read_volatile() }
}

fn write(reg: u32, value: u32) {
    unsafe { ((BASE.load(Ordering::Relaxed) + reg as u64) as *mut u32).write_volatile(value) }
}

pub fn id() -> u32 {
    read(REG_ID) >> 24
}

pub fn eoi() {
    write(REG_EOI, 0);
}

fn enable_this_cpu() {
    let mut msr = Msr::new(IA32_APIC_BASE);
    unsafe { msr.write(msr.read() | (1 << 11)) };
    write(REG_TPR, 0);
    write(REG_SVR, 0x100 | SPURIOUS_VECTOR as u32);
}

/// Bootstrap CPU: map the APIC, enable it, measure and start the timer.
pub fn init_bsp() {
    let phys = unsafe { Msr::new(IA32_APIC_BASE).read() } & 0xf_ffff_f000;
    crate::mm::paging::map_mmio(MMIO_VIRT, phys, 1).expect("cannot map the local APIC");
    BASE.store(MMIO_VIRT, Ordering::Relaxed);
    enable_this_cpu();

    let per_10ms = calibrate_10ms();
    let per_tick = (per_10ms as u64 * 100 / HZ) as u32;
    COUNTS_PER_TICK.store(per_tick, Ordering::Relaxed);
    log::info!(
        "apic: id {}, timer {} MHz bus, {} Hz tick",
        id(),
        per_10ms as u64 * 16 * 100 / 1_000_000,
        HZ
    );
    start_timer();
}

/// Application processors: the mapping already exists.
pub fn init_ap() {
    enable_this_cpu();
    start_timer();
}

fn start_timer() {
    write(REG_TIMER_DIVIDE, DIVIDE_BY_16);
    write(REG_LVT_TIMER, TIMER_VECTOR as u32 | TIMER_PERIODIC);
    write(REG_TIMER_INIT, COUNTS_PER_TICK.load(Ordering::Relaxed));
}

/// Counts the APIC timer makes in 10 ms, timed with PIT channel 2.
fn calibrate_10ms() -> u32 {
    const PIT_HZ: u32 = 1_193_182;
    let reload = (PIT_HZ / 100) as u16;
    let mut gate = Port::<u8>::new(0x61);
    let mut cmd = Port::<u8>::new(0x43);
    let mut ch2 = Port::<u8>::new(0x42);
    unsafe {
        // Channel 2 gate on, speaker off; one-shot mode, binary.
        let g = gate.read();
        gate.write((g & !0x02) | 0x01);
        cmd.write(0b1011_0000);
        ch2.write(reload as u8);
        ch2.write((reload >> 8) as u8);
        // Restart the count by toggling the gate.
        let g = gate.read() & !0x01;
        gate.write(g);
        gate.write(g | 0x01);

        write(REG_TIMER_DIVIDE, DIVIDE_BY_16);
        write(REG_LVT_TIMER, LVT_MASKED);
        write(REG_TIMER_INIT, u32::MAX);
        while gate.read() & 0x20 == 0 {
            core::hint::spin_loop();
        }
        let elapsed = u32::MAX - read(REG_TIMER_CURRENT);
        write(REG_TIMER_INIT, 0);
        elapsed
    }
}

/// The BSP's APIC ID before the APIC is mapped (from the bootloader's CPU list).
pub fn bsp_id_from_bootloader() -> u32 {
    crate::limine::MP.response().map_or(0, |mp| mp.bsp_lapic_id)
}
