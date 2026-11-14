//! 4-level x86_64 page tables, built while boot services are still active
//! (UEFI identity-maps memory, so physical addresses are usable pointers).

use crate::efi;
use crate::mem;

pub const PAGE: u64 = 4096;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const HUGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

const SIZE_2M: u64 = 2 << 20;
const SIZE_1G: u64 = 1 << 30;

/// Page-table pages are taken from chunks this many pages long.
const CHUNK_PAGES: u64 = 32;

pub struct PageTables {
    bs: *mut efi::BootServices,
    pub pml4: u64,
    next: u64,
    end: u64,
    /// Number of 4 KiB page-table pages used.
    pub used: u64,
}

impl PageTables {
    pub fn new(bs: *mut efi::BootServices) -> Result<Self, &'static str> {
        let mut pt = PageTables {
            bs,
            pml4: 0,
            next: 0,
            end: 0,
            used: 0,
        };
        pt.pml4 = pt.frame()?;
        Ok(pt)
    }

    /// A zeroed page below 4 GiB (the kernel's AP trampoline loads CR3 from
    /// 32-bit code, so the whole hierarchy must be reachable from there; only
    /// the PML4 strictly needs it, but keeping them all low is simplest).
    fn frame(&mut self) -> Result<u64, &'static str> {
        if self.next == self.end {
            let base = mem::allocate_pages(
                self.bs,
                efi::ALLOCATE_MAX_ADDRESS,
                mem::RECLAIMABLE_TYPE,
                CHUNK_PAGES,
                0xffff_ffff,
            )
            .map_err(|_| "out of memory below 4 GiB for page tables")?;
            self.next = base;
            self.end = base + CHUNK_PAGES * PAGE;
        }
        let f = self.next;
        self.next += PAGE;
        self.used += 1;
        // SAFETY: freshly allocated, identity-mapped page.
        unsafe { core::ptr::write_bytes(f as *mut u8, 0, PAGE as usize) };
        Ok(f)
    }

    /// The table that `table[index]` points to, created if missing.
    fn next_table(&mut self, table: u64, index: u64) -> Result<u64, &'static str> {
        let e = (table + index * 8) as *mut u64;
        // SAFETY: `table` is one of our page-table pages.
        let v = unsafe { e.read() };
        if v & PRESENT != 0 {
            if v & HUGE != 0 {
                return Err("internal error: page table overlaps a huge page");
            }
            return Ok(v & ADDR_MASK);
        }
        let t = self.frame()?;
        // Intermediate entries allow everything; the leaf decides.
        // SAFETY: as above.
        unsafe { e.write(t | PRESENT | WRITABLE) };
        Ok(t)
    }

    fn index(virt: u64, level: u32) -> u64 {
        (virt >> (12 + 9 * level)) & 0x1ff
    }

    /// Maps `[phys, phys + len)` at `virt` with 1 GiB pages if `gib_pages`,
    /// else 2 MiB pages. All three values must be 1 GiB aligned.
    pub fn map_large(
        &mut self,
        virt: u64,
        phys: u64,
        len: u64,
        executable: bool,
        gib_pages: bool,
    ) -> Result<(), &'static str> {
        let flags = PRESENT | WRITABLE | HUGE | if executable { 0 } else { NO_EXECUTE };
        let mut off = 0;
        while off < len {
            let v = virt + off;
            let pdpt = self.next_table(self.pml4, Self::index(v, 3))?;
            if gib_pages {
                // SAFETY: pdpt is one of our page-table pages.
                unsafe { ((pdpt + Self::index(v, 2) * 8) as *mut u64).write((phys + off) | flags) };
            } else {
                let pd = self.next_table(pdpt, Self::index(v, 2))?;
                for i in 0..512 {
                    let p = phys + off + i * SIZE_2M;
                    // SAFETY: pd is one of our page-table pages.
                    unsafe { ((pd + i * 8) as *mut u64).write(p | flags) };
                }
            }
            off += SIZE_1G;
        }
        Ok(())
    }

    /// Maps one 4 KiB page. If the page is already mapped (two segments
    /// sharing a page), the permissions are the union of both.
    pub fn map_4k(
        &mut self,
        virt: u64,
        phys: u64,
        writable: bool,
        executable: bool,
    ) -> Result<(), &'static str> {
        let pdpt = self.next_table(self.pml4, Self::index(virt, 3))?;
        let pd = self.next_table(pdpt, Self::index(virt, 2))?;
        let pt = self.next_table(pd, Self::index(virt, 1))?;
        let e = (pt + Self::index(virt, 0) * 8) as *mut u64;
        // SAFETY: pt is one of our page-table pages.
        let old = unsafe { e.read() };
        let (mut w, mut x) = (writable, executable);
        if old & PRESENT != 0 {
            if old & ADDR_MASK != phys {
                return Err("ELF segments map one page to two places");
            }
            w |= old & WRITABLE != 0;
            x |= old & NO_EXECUTE == 0;
        }
        let v = phys | PRESENT | if w { WRITABLE } else { 0 } | if x { 0 } else { NO_EXECUTE };
        // SAFETY: as above.
        unsafe { e.write(v) };
        Ok(())
    }
}

/// Whether the CPU supports 1 GiB pages (CPUID 0x80000001 EDX bit 26).
pub fn has_gib_pages() -> bool {
    let max = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    max >= 0x8000_0001 && core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 26) != 0
}
