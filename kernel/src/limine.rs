//! The parts of the Limine boot protocol SegOS uses.
//!
//! Requests are statics placed in `.limine_requests`; the bootloader finds
//! them by their IDs and fills in the `response` pointers before jumping to
//! `kmain`. Spec: <https://github.com/limine-bootloader/limine/blob/trunk/PROTOCOL.md>.

use core::cell::UnsafeCell;

const COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

/// A request the bootloader writes a response pointer into.
#[repr(C)]
pub struct Request<R> {
    id: [u64; 4],
    revision: u64,
    response: UnsafeCell<*const R>,
}

// Written once by the bootloader before the kernel runs, read-only after.
unsafe impl<R> Sync for Request<R> {}

impl<R> Request<R> {
    const fn new(id: [u64; 2]) -> Self {
        Self {
            id: [COMMON_MAGIC[0], COMMON_MAGIC[1], id[0], id[1]],
            revision: 0,
            response: UnsafeCell::new(core::ptr::null()),
        }
    }

    pub fn response(&self) -> Option<&'static R> {
        // Volatile: the compiler must not assume the null initialiser.
        let ptr = unsafe { core::ptr::read_volatile(self.response.get()) };
        unsafe { ptr.as_ref() }
    }
}

#[repr(C)]
pub struct BaseRevision(UnsafeCell<[u64; 3]>);
unsafe impl Sync for BaseRevision {}

impl BaseRevision {
    /// The bootloader sets the last word to 0 when it supports the revision.
    pub fn is_supported(&self) -> bool {
        unsafe { core::ptr::read_volatile(self.0.get())[2] == 0 }
    }
}

#[repr(C)]
pub struct Marker<const N: usize>([u64; N]);

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static REQUESTS_START: Marker<4> = Marker([
    0xf6b8f4b39de7d1ae,
    0xfab91a6940fcb9cf,
    0x785c6ed015d3e316,
    0x181e920a7852b9d9,
]);

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static REQUESTS_END: Marker<2> = Marker([0xadc0e0531bb10d03, 0x9572709f31764c62]);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static BASE_REVISION: BaseRevision =
    BaseRevision(UnsafeCell::new([0xf9562b2d5c95a6c8, 0x6a7b384944536bdc, 3]));

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static FRAMEBUFFER: Request<FramebufferResponse> =
    Request::new([0x9d5827dcd881dd75, 0xa3148604f6fab11b]);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static MEMMAP: Request<MemmapResponse> = Request::new([0x67cf3d9d378a806f, 0xe304acdfc50c3c62]);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static HHDM: Request<HhdmResponse> = Request::new([0x48dcf1cb8ad2b852, 0x63984e959a98244b]);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static EXECUTABLE_ADDRESS: Request<ExecutableAddressResponse> =
    Request::new([0x71ba76863cc55f63, 0xb2644a48c516a487]);

#[repr(C)]
pub struct FramebufferResponse {
    pub revision: u64,
    count: u64,
    framebuffers: *const *const Framebuffer,
}

impl FramebufferResponse {
    pub fn framebuffers(&self) -> &[&Framebuffer] {
        unsafe { core::slice::from_raw_parts(self.framebuffers.cast(), self.count as usize) }
    }
}

#[repr(C)]
pub struct Framebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
}

#[repr(C)]
pub struct MemmapResponse {
    pub revision: u64,
    count: u64,
    entries: *const *const MemmapEntry,
}

impl MemmapResponse {
    pub fn entries(&self) -> &[&MemmapEntry] {
        unsafe { core::slice::from_raw_parts(self.entries.cast(), self.count as usize) }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct MemmapEntry {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

pub mod memmap_kind {
    pub const USABLE: u64 = 0;
    pub const RESERVED: u64 = 1;
    pub const ACPI_RECLAIMABLE: u64 = 2;
    pub const ACPI_NVS: u64 = 3;
    pub const BAD_MEMORY: u64 = 4;
    pub const BOOTLOADER_RECLAIMABLE: u64 = 5;
    pub const EXECUTABLE_AND_MODULES: u64 = 6;
    pub const FRAMEBUFFER: u64 = 7;

    pub fn name(kind: u64) -> &'static str {
        match kind {
            USABLE => "usable",
            RESERVED => "reserved",
            ACPI_RECLAIMABLE => "ACPI reclaimable",
            ACPI_NVS => "ACPI NVS",
            BAD_MEMORY => "bad",
            BOOTLOADER_RECLAIMABLE => "bootloader reclaimable",
            EXECUTABLE_AND_MODULES => "kernel and modules",
            FRAMEBUFFER => "framebuffer",
            _ => "unknown",
        }
    }
}

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct ExecutableAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

/// Multiprocessor request: Limine brings every CPU into long mode, parks
/// the application processors, and starts one when its `goto_address` is set.
#[repr(C)]
pub struct MpRequest {
    id: [u64; 4],
    revision: u64,
    response: UnsafeCell<*const MpResponse>,
    flags: u64,
}

unsafe impl Sync for MpRequest {}

impl MpRequest {
    pub fn response(&self) -> Option<&'static MpResponse> {
        let ptr = unsafe { core::ptr::read_volatile(self.response.get()) };
        unsafe { ptr.as_ref() }
    }
}

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static MP: MpRequest = MpRequest {
    id: [
        COMMON_MAGIC[0],
        COMMON_MAGIC[1],
        0x95a67b819a1b857e,
        0xa0b61b723b6a73e0,
    ],
    revision: 0,
    response: UnsafeCell::new(core::ptr::null()),
    flags: 0, // stay in xAPIC mode
};

#[repr(C)]
pub struct MpResponse {
    pub revision: u64,
    pub flags: u32,
    pub bsp_lapic_id: u32,
    count: u64,
    cpus: *const *mut MpInfo,
}

impl MpResponse {
    pub fn cpus(&self) -> &[*mut MpInfo] {
        unsafe { core::slice::from_raw_parts(self.cpus, self.count as usize) }
    }
}

#[repr(C)]
pub struct MpInfo {
    pub processor_id: u32,
    pub lapic_id: u32,
    reserved: u64,
    pub goto_address: core::sync::atomic::AtomicU64,
    pub extra_argument: u64,
}
