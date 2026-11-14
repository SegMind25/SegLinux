//! Kernel page tables: our own 4-level page table walker.
//!
//! The kernel keeps the tables segboot built (kernel image in the top
//! 2 GiB, physical memory direct-mapped at the HHDM offset) and maps new
//! memory into them. Per-process address spaces arrive in Phase 2.

use super::{PAGE_SIZE, frame, phys_to_virt};
use crate::hal::x86;
use crate::sync::SpinLock;

pub mod flags {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const NO_CACHE: u64 = 1 << 4;
    pub const HUGE: u64 = 1 << 7;
    pub const NO_EXECUTE: u64 = 1 << 63;
}

const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

#[derive(Debug, PartialEq, Eq)]
pub enum MapError {
    OutOfMemory,
    AlreadyMapped,
}

/// Physical address of the top-level table.
static KERNEL_PML4: SpinLock<u64> = SpinLock::new(0);

pub fn init() {
    *KERNEL_PML4.lock() = x86::read_cr3() & ADDR_MASK;
}

pub fn kernel_pml4() -> u64 {
    *KERNEL_PML4.lock()
}

fn table(phys: u64) -> &'static mut [u64; 512] {
    unsafe { &mut *phys_to_virt(phys).cast::<[u64; 512]>() }
}

fn index(virt: u64, level: u32) -> usize {
    ((virt >> (12 + 9 * level)) & 0x1ff) as usize
}

/// The level-1 entry for `virt`, creating intermediate tables if `create`.
fn walk(pml4: u64, virt: u64, create: bool) -> Result<Option<&'static mut u64>, MapError> {
    let mut table_phys = pml4;
    for level in (1..4).rev() {
        let entry = &mut table(table_phys)[index(virt, level)];
        if *entry & flags::PRESENT == 0 {
            if !create {
                return Ok(None);
            }
            let new = frame::alloc_zeroed().ok_or(MapError::OutOfMemory)?;
            // Intermediate entries are permissive; the leaf decides.
            *entry =
                new | flags::PRESENT | flags::WRITABLE | (virt_is_user(virt) as u64 * flags::USER);
        } else if *entry & flags::HUGE != 0 {
            return Err(MapError::AlreadyMapped);
        }
        table_phys = *entry & ADDR_MASK;
    }
    Ok(Some(&mut table(table_phys)[index(virt, 0)]))
}

fn virt_is_user(virt: u64) -> bool {
    virt < 0x0000_8000_0000_0000
}

/// Maps one 4 KiB page.
pub fn map_page(virt: u64, phys: u64, page_flags: u64) -> Result<(), MapError> {
    let pml4 = *KERNEL_PML4.lock();
    let _guard = MAP_LOCK.lock();
    let entry = walk(pml4, virt, true)?.unwrap();
    if *entry & flags::PRESENT != 0 {
        return Err(MapError::AlreadyMapped);
    }
    *entry = phys | page_flags | flags::PRESENT;
    unsafe { x86::invlpg(virt) };
    Ok(())
}

/// Serialises changes to the kernel tables.
static MAP_LOCK: SpinLock<()> = SpinLock::new(());

/// Maps `pages` fresh zeroed frames at `virt`, read-write, not executable.
pub fn map_new(virt: u64, pages: u64) -> Result<(), MapError> {
    for i in 0..pages {
        let phys = frame::alloc_zeroed().ok_or(MapError::OutOfMemory)?;
        if let Err(e) = map_page(
            virt + i * PAGE_SIZE,
            phys,
            flags::WRITABLE | flags::NO_EXECUTE,
        ) {
            frame::free(phys);
            return Err(e);
        }
    }
    Ok(())
}

/// Maps device memory (`phys`, `pages` long) at `virt`, uncached.
pub fn map_mmio(virt: u64, phys: u64, pages: u64) -> Result<(), MapError> {
    for i in 0..pages {
        map_page(
            virt + i * PAGE_SIZE,
            phys + i * PAGE_SIZE,
            flags::WRITABLE | flags::NO_CACHE | flags::WRITE_THROUGH | flags::NO_EXECUTE,
        )?;
    }
    Ok(())
}

/// Unmaps one page and frees its frame.
pub fn unmap(virt: u64) {
    let pml4 = *KERNEL_PML4.lock();
    let _guard = MAP_LOCK.lock();
    let entry = walk(pml4, virt, false)
        .ok()
        .flatten()
        .expect("unmap of an unmapped page");
    assert!(*entry & flags::PRESENT != 0, "unmap of an unmapped page");
    let phys = *entry & ADDR_MASK;
    *entry = 0;
    unsafe { x86::invlpg(virt) };
    frame::free(phys);
}

/// Physical address `virt` maps to, following huge pages.
pub fn translate(virt: u64) -> Option<u64> {
    let mut table_phys = *KERNEL_PML4.lock();
    for level in (0..4).rev() {
        let entry = table(table_phys)[index(virt, level)];
        if entry & flags::PRESENT == 0 {
            return None;
        }
        if level == 0 || entry & flags::HUGE != 0 {
            let page_mask = (1u64 << (12 + 9 * level)) - 1;
            return Some((entry & ADDR_MASK & !page_mask) | (virt & page_mask));
        }
        table_phys = entry & ADDR_MASK;
    }
    None
}
