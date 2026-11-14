//! Page allocation and conversion of the UEFI memory map to the SegOS one.

use crate::efi;
use crate::paging::PAGE;
use bootinfo::{MemoryKind, MemoryRegion};

/// OEM memory type for the kernel's segments: reported as `Kernel`.
pub const KERNEL_TYPE: u32 = 0x8000_0001;
/// OEM memory type for page tables, BootInfo, the memory map and the boot
/// stack: reported as `BootloaderReclaimable`.
pub const RECLAIMABLE_TYPE: u32 = 0x8000_0002;

/// `AllocatePages`; `max` is the highest address for ALLOCATE_MAX_ADDRESS.
pub fn allocate_pages(
    bs: *mut efi::BootServices,
    alloc_type: u32,
    mem_type: u32,
    pages: u64,
    max: u64,
) -> Result<u64, efi::Status> {
    let mut addr = max;
    // SAFETY: boot services are active; `addr` is a valid out pointer.
    let s = unsafe { ((*bs).allocate_pages)(alloc_type, mem_type, pages as usize, &mut addr) };
    if s == efi::SUCCESS {
        Ok(addr)
    } else {
        Err(s)
    }
}

fn kind_of(ty: u32) -> MemoryKind {
    match ty {
        efi::CONVENTIONAL_MEMORY
        | efi::BOOT_SERVICES_CODE
        | efi::BOOT_SERVICES_DATA
        | efi::LOADER_CODE => MemoryKind::Usable,
        efi::LOADER_DATA | RECLAIMABLE_TYPE => MemoryKind::BootloaderReclaimable,
        KERNEL_TYPE => MemoryKind::Kernel,
        efi::ACPI_RECLAIM_MEMORY => MemoryKind::AcpiReclaimable,
        efi::ACPI_MEMORY_NVS => MemoryKind::AcpiNvs,
        efi::UNUSABLE_MEMORY => MemoryKind::BadMemory,
        // Runtime services code/data, MMIO, port space, PAL code,
        // persistent memory, reserved and unknown types.
        _ => MemoryKind::Reserved,
    }
}

/// Iterates over the descriptors of a raw UEFI memory map.
pub fn descriptors(
    buf: u64,
    size: usize,
    desc_size: usize,
) -> impl Iterator<Item = efi::MemoryDescriptor> {
    (0..size / desc_size).map(move |i| {
        // SAFETY: the firmware wrote `size` bytes of descriptors at `buf`;
        // descriptors may be larger than our struct, never smaller.
        unsafe { ((buf as usize + i * desc_size) as *const efi::MemoryDescriptor).read_unaligned() }
    })
}

/// End of the highest region in the map.
pub fn top_of_memory(buf: u64, size: usize, desc_size: usize) -> u64 {
    descriptors(buf, size, desc_size)
        .map(|d| d.physical_start + d.number_of_pages * PAGE)
        .max()
        .unwrap_or(0)
}

/// Builds the SegOS memory map in `out` from the final UEFI map and returns
/// its length: sorted, non-overlapping, page aligned, adjacent regions of the
/// same kind merged, the framebuffer `[fb_start, fb_end)` (if non-empty)
/// marked `Framebuffer`. Runs after ExitBootServices: no allocation.
pub fn convert(
    buf: u64,
    size: usize,
    desc_size: usize,
    fb: (u64, u64),
    out: &mut [MemoryRegion],
) -> Result<usize, &'static str> {
    let (fb_start, fb_end) = (fb.0 & !(PAGE - 1), (fb.1 + PAGE - 1) & !(PAGE - 1));
    let mut n = 0;
    let mut push = |base: u64, end: u64, kind: MemoryKind| {
        if end <= base {
            return Ok(());
        }
        let r = out.get_mut(n).ok_or("memory map array too small")?;
        *r = MemoryRegion {
            base,
            length: end - base,
            kind,
            _reserved: 0,
        };
        n += 1;
        Ok(())
    };
    for d in descriptors(buf, size, desc_size) {
        // UEFI regions are 4 KiB granular; align defensively inward.
        let base = (d.physical_start + PAGE - 1) & !(PAGE - 1);
        let end = (d.physical_start + d.number_of_pages * PAGE) & !(PAGE - 1);
        let kind = kind_of(d.ty);
        if fb_end > fb_start && base < fb_end && end > fb_start {
            // Cut the framebuffer out of whatever the firmware called it.
            push(base, base.max(fb_start).min(end), kind)?;
            push(fb_end.max(base), end, kind)?;
        } else {
            push(base, end, kind)?;
        }
    }
    push(fb_start, fb_end, MemoryKind::Framebuffer)?;

    let regions = &mut out[..n];
    // Insertion sort: the map has at most a few hundred entries.
    for i in 1..regions.len() {
        let mut j = i;
        while j > 0 && regions[j - 1].base > regions[j].base {
            regions.swap(j - 1, j);
            j -= 1;
        }
    }

    // Merge, and resolve overlaps (the spec forbids them, but never let a
    // later region overlap an earlier one: the earlier wins).
    let mut len = 0;
    for i in 0..regions.len() {
        let mut r = regions[i];
        if len > 0 {
            let prev = &mut regions[len - 1];
            let prev_end = prev.base + prev.length;
            if r.base < prev_end {
                let end = r.base + r.length;
                if end <= prev_end {
                    continue;
                }
                r.length = end - prev_end;
                r.base = prev_end;
            }
            if r.base == prev_end && r.kind == prev.kind {
                prev.length += r.length;
                continue;
            }
        }
        regions[len] = r;
        len += 1;
    }
    Ok(len)
}
