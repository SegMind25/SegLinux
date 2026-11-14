//! Starting the application processors (APs).
//!
//! An AP wakes up in 16-bit real mode at a page below 1 MiB that we choose
//! (the STARTUP IPI's vector). The trampoline below is copied there; it
//! switches straight to 64-bit long mode with the kernel's page tables and
//! calls [`ap_entry`] on a fresh stack. The bootstrap CPU starts the APs
//! one at a time and waits for each to report in.

use super::{acpi, apic, gdt, idt, percpu};
use crate::mm::{PAGE_SIZE, frame, paging, phys_to_virt};
use core::sync::atomic::{AtomicU32, Ordering};

core::arch::global_asm!(
    r#"
    .section .rodata.ap_trampoline, "a"
    .global ap_trampoline_start
    .global ap_trampoline_end
    .code16
ap_trampoline_start:
    cli
    cld
    mov ax, cs
    mov ds, ax
    .byte 0x0f, 0x01, 0x16            // lgdt [ap_gdt_ptr] (16-bit offset)
    .word ap_gdt_ptr - ap_trampoline_start
    mov eax, cr4
    or eax, 0x20                     // PAE
    mov cr4, eax
    .byte 0x66, 0xa1                  // mov eax, [ap_cr3] (16-bit offset)
    .word ap_cr3 - ap_trampoline_start
    mov cr3, eax
    mov ecx, 0xc0000080              // EFER
    rdmsr
    or eax, 0x900                    // LME | NXE
    wrmsr
    mov eax, cr0
    or eax, 0x80010001               // PG | WP | PE: real mode to long mode
    mov cr0, eax
    .byte 0x66, 0xff, 0x2e           // jmp far dword [ap_long_ptr]
    .word ap_long_ptr - ap_trampoline_start
    .code64
ap_long_mode:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    xor eax, eax
    mov fs, ax
    mov gs, ax
    mov rsp, [rip + ap_stack]
    mov rdi, [rip + ap_arg]
    mov rax, [rip + ap_entry_addr]
    xor ebp, ebp
    call rax
    ud2
    .balign 8
ap_gdt:
    .quad 0
    .quad 0x00af9a000000ffff         // 64-bit code
    .quad 0x00cf92000000ffff         // data
ap_gdt_ptr:
    .word 23
    .long 0                          // patched: physical address of ap_gdt
ap_long_ptr:
    .long 0                          // patched: physical address of ap_long_mode
    .word 0x08
    .balign 8
ap_cr3:
    .quad 0
ap_stack:
    .quad 0
ap_arg:
    .quad 0
ap_entry_addr:
    .quad 0
ap_trampoline_end:
    .text
"#
);

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_end: u8;
    static ap_gdt: u8;
    static ap_gdt_ptr: u8;
    static ap_long_mode: u8;
    static ap_long_ptr: u8;
    static ap_cr3: u8;
    static ap_stack: u8;
    static ap_arg: u8;
    static ap_entry_addr: u8;
}

static ONLINE: AtomicU32 = AtomicU32::new(1);
static FOUND: AtomicU32 = AtomicU32::new(1);

/// Offset of a trampoline symbol from its start.
fn offset(sym: &u8) -> u64 {
    sym as *const u8 as u64 - (&raw const ap_trampoline_start) as u64
}

/// Starts every AP listed in the ACPI MADT, one at a time.
pub fn start_aps(rsdp: u64) -> u32 {
    let ids = acpi::processor_apic_ids(rsdp);
    FOUND.store(ids.len().max(1) as u32, Ordering::Relaxed);
    let me = apic::id();
    if ids.len() <= 1 {
        crate::info!("smp: 1 CPU");
        return 1;
    }

    // The page must be below 1 MiB; the boot page tables identity-map it.
    let page = frame::alloc_below(0x10_0000).expect("smp: no free page below 1 MiB");
    let size = (&raw const ap_trampoline_end) as u64 - (&raw const ap_trampoline_start) as u64;
    assert!(size <= PAGE_SIZE);
    let base = phys_to_virt(page);
    let pml4 = paging::kernel_pml4();
    assert!(pml4 < 1 << 32, "smp: page tables above 4 GiB");
    unsafe {
        core::ptr::copy_nonoverlapping(&raw const ap_trampoline_start, base, size as usize);
        let field = |sym: &u8| base.add(offset(sym) as usize);
        let put32 = |sym: &u8, v: u32| field(sym).cast::<u32>().write_unaligned(v);
        let put64 = |sym: &u8, v: u64| field(sym).cast::<u64>().write_unaligned(v);
        // The GDT pointer's base is 2 bytes in; the far pointer's offset first.
        field(&ap_gdt_ptr)
            .add(2)
            .cast::<u32>()
            .write_unaligned((page + offset(&ap_gdt)) as u32);
        put32(&ap_long_ptr, (page + offset(&ap_long_mode)) as u32);
        put64(&ap_cr3, pml4);
        put64(&ap_entry_addr, ap_entry as *const () as u64);

        let mut index = 1u32;
        for &id in ids.iter().filter(|&&id| id != me) {
            let stack = crate::task::KernelStack::new().expect("smp: no memory for a stack");
            put64(&ap_stack, stack.top());
            put64(&ap_arg, (index as u64) << 32 | id as u64);
            core::mem::forget(stack); // the AP's idle stack lives forever
            let before = ONLINE.load(Ordering::Acquire);
            apic::start_ap(id, (page / PAGE_SIZE) as u8);
            // Up to a second for the AP to report in.
            let mut waited = 0;
            while ONLINE.load(Ordering::Acquire) == before && waited < 100 {
                apic::delay_us(10_000);
                waited += 1;
            }
            if ONLINE.load(Ordering::Acquire) == before {
                crate::warn!("smp: CPU with APIC ID {id} did not start");
            } else {
                index += 1;
            }
        }
    }
    frame::free(page);
    let online = ONLINE.load(Ordering::Acquire);
    crate::info!("smp: {online} CPUs online");
    online
}

/// Number of CPUs ACPI lists (1 if none).
pub fn cpu_count_from_acpi() -> usize {
    FOUND.load(Ordering::Relaxed) as usize
}

extern "sysv64" fn ap_entry(arg: u64) -> ! {
    let (index, apic_id) = ((arg >> 32) as u32, arg as u32);
    gdt::init_ap();
    idt::init();
    percpu::init(index, apic_id);
    apic::init_ap();
    crate::task::sched::init_cpu();
    ONLINE.fetch_add(1, Ordering::Release);
    crate::task::sched::idle_loop()
}
