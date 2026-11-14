//! The SegOS boot protocol: what `segboot` hands to the kernel.
//!
//! Machine state when the kernel entry point runs:
//!
//! - 64-bit long mode, interrupts disabled, UEFI boot services exited.
//! - `rdi` holds a pointer to [`BootInfo`], valid through the direct map.
//!   The entry point is `extern "sysv64" fn(&'static BootInfo) -> !`.
//! - `rsp` points to the top of a 64 KiB stack inside the direct map,
//!   16-byte aligned (as just before a `call`).
//! - CR3 points to page tables allocated **below 4 GiB** (the kernel's
//!   application-processor trampoline loads CR3 from 32-bit code) that map:
//!   - the kernel's ELF segments at their link addresses (top 2 GiB), with
//!     W/NX from the segment flags;
//!   - all physical memory from 0 to max(4 GiB, end of the highest memory
//!     map region) at [`BootInfo::hhdm_offset`] (the "direct map"),
//!     read-write, no-execute, 2 MiB or 1 GiB pages;
//!   - the same range identity-mapped (virtual = physical), read-write,
//!     executable. The kernel may remove it once it no longer needs it.
//! - EFER.NXE and CR0.WP are set; the GDT and IDT are the firmware's (the
//!   kernel loads its own first thing).

#![no_std]

/// "SEGBOOT\0" little-endian.
pub const MAGIC: u64 = u64::from_le_bytes(*b"SEGBOOT\0");
pub const VERSION: u32 = 1;

/// Virtual address of the direct map of physical memory.
pub const HHDM_OFFSET: u64 = 0xffff_8000_0000_0000;

#[repr(C)]
#[derive(Debug)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub _reserved: u32,
    pub hhdm_offset: u64,
    /// Physical and virtual address of the lowest kernel segment.
    pub kernel_phys_base: u64,
    pub kernel_virt_base: u64,
    /// Direct-map pointer to `memory_map_len` regions, sorted by base,
    /// non-overlapping, page aligned.
    pub memory_map: *const MemoryRegion,
    pub memory_map_len: u64,
    /// `framebuffer.address == 0` when there is no graphics output.
    pub framebuffer: Framebuffer,
    /// Physical address of the ACPI RSDP (version 2 preferred), 0 if none.
    pub rsdp: u64,
}

impl BootInfo {
    pub fn memory_map(&self) -> &[MemoryRegion] {
        // SAFETY: the bootloader guarantees pointer and length.
        unsafe { core::slice::from_raw_parts(self.memory_map, self.memory_map_len as usize) }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryKind,
    pub _reserved: u32,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    /// Free RAM.
    Usable = 1,
    Reserved = 2,
    AcpiReclaimable = 3,
    AcpiNvs = 4,
    BadMemory = 5,
    /// Page tables, this structure, the memory map, the boot stack.
    /// Becomes usable once the kernel has its own copies.
    BootloaderReclaimable = 6,
    /// The kernel's loaded segments.
    Kernel = 7,
    Framebuffer = 8,
}

impl MemoryKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Reserved => "reserved",
            Self::AcpiReclaimable => "ACPI reclaimable",
            Self::AcpiNvs => "ACPI NVS",
            Self::BadMemory => "bad",
            Self::BootloaderReclaimable => "bootloader reclaimable",
            Self::Kernel => "kernel",
            Self::Framebuffer => "framebuffer",
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Framebuffer {
    /// Physical address.
    pub address: u64,
    pub width: u32,
    pub height: u32,
    /// Bytes per row.
    pub pitch: u32,
    /// Always 32 when present.
    pub bpp: u32,
    pub format: PixelFormat,
    pub _reserved: u32,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Byte order blue, green, red, unused (0x00RRGGBB as a u32).
    Bgrx = 0,
    /// Byte order red, green, blue, unused.
    Rgbx = 1,
}
