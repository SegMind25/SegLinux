//! Kernel page tables.
//!
//! Phase 0 keeps the tables Limine built (kernel image in the top 2 GiB,
//! physical memory direct-mapped at the HHDM offset) and maps new memory
//! into them. Per-process address spaces arrive in Phase 2.

use super::frame::Frames;
use crate::sync::SpinLock;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

static KERNEL_TABLE: SpinLock<Option<OffsetPageTable<'static>>> = SpinLock::new(None);

pub fn init() {
    let (pml4_frame, _) = Cr3::read();
    let pml4 = super::phys_to_virt(pml4_frame.start_address().as_u64()).cast::<PageTable>();
    let offset = VirtAddr::new(super::hhdm_offset());
    let table = unsafe { OffsetPageTable::new(&mut *pml4, offset) };
    *KERNEL_TABLE.lock() = Some(table);
}

#[derive(Debug)]
pub enum MapError {
    OutOfMemory,
    AlreadyMapped,
}

/// Maps `pages` fresh zeroed frames at `virt`, read-write, not executable.
pub fn map_new(virt: u64, pages: u64) -> Result<(), MapError> {
    let mut guard = KERNEL_TABLE.lock();
    let table = guard.as_mut().expect("paging not initialised");
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
    for i in 0..pages {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt + i * super::PAGE_SIZE));
        let phys = super::frame::alloc_zeroed().ok_or(MapError::OutOfMemory)?;
        let frame = PhysFrame::containing_address(PhysAddr::new(phys));
        match unsafe { table.map_to(page, frame, flags, &mut Frames) } {
            Ok(flush) => flush.flush(),
            Err(x86_64::structures::paging::mapper::MapToError::FrameAllocationFailed) => {
                super::frame::free(phys);
                return Err(MapError::OutOfMemory);
            }
            Err(_) => {
                super::frame::free(phys);
                return Err(MapError::AlreadyMapped);
            }
        }
    }
    Ok(())
}

/// Unmaps one page and frees its frame.
pub fn unmap(virt: u64) {
    let mut guard = KERNEL_TABLE.lock();
    let table = guard.as_mut().expect("paging not initialised");
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let (frame, flush) = table.unmap(page).expect("unmap of an unmapped page");
    flush.flush();
    super::frame::free(frame.start_address().as_u64());
}

pub fn translate(virt: u64) -> Option<u64> {
    let guard = KERNEL_TABLE.lock();
    guard
        .as_ref()?
        .translate_addr(VirtAddr::new(virt))
        .map(|p| p.as_u64())
}

/// Maps device memory (`phys`, `pages` long) at `virt`, uncached.
pub fn map_mmio(virt: u64, phys: u64, pages: u64) -> Result<(), MapError> {
    let mut guard = KERNEL_TABLE.lock();
    let table = guard.as_mut().expect("paging not initialised");
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH
        | PageTableFlags::NO_EXECUTE;
    for i in 0..pages {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt + i * super::PAGE_SIZE));
        let frame = PhysFrame::containing_address(PhysAddr::new(phys + i * super::PAGE_SIZE));
        match unsafe { table.map_to(page, frame, flags, &mut Frames) } {
            Ok(flush) => flush.flush(),
            Err(x86_64::structures::paging::mapper::MapToError::FrameAllocationFailed) => {
                return Err(MapError::OutOfMemory);
            }
            Err(_) => return Err(MapError::AlreadyMapped),
        }
    }
    Ok(())
}
