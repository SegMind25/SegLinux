//! segboot: the SegOS UEFI bootloader.
//!
//! Loads `\boot\segkernel` (an ELF64 executable) from the partition segboot
//! itself was loaded from, builds the page tables described in the
//! `bootinfo` crate, exits boot services and jumps to the kernel.

#![no_std]
#![no_main]

mod console;
mod efi;
mod elf;
mod mem;
mod paging;

use bootinfo::{BootInfo, Framebuffer, HHDM_OFFSET, MAGIC, MemoryRegion, PixelFormat, VERSION};
use core::arch::asm;
use core::ffi::c_void;
use core::fmt;
use core::ptr::null_mut;
use paging::{PAGE, PageTables};

const KERNEL_PATH_STR: &str = "\\boot\\segkernel";
static KERNEL_PATH: [u16; KERNEL_PATH_STR.len() + 1] = ucs2(KERNEL_PATH_STR);

const STACK_SIZE: u64 = 64 * 1024;
const GIB: u64 = 1 << 30;
/// Read the kernel file in pieces this large (some firmware dislikes huge reads).
const READ_CHUNK: usize = 1 << 20;

/// ASCII string to a NUL-terminated UCS-2 array.
const fn ucs2<const N: usize>(s: &str) -> [u16; N] {
    let b = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    while i < b.len() {
        out[i] = b[i] as u16;
        i += 1;
    }
    out
}

/// A boot failure: what went wrong, plus the firmware's status if any.
struct Error {
    msg: &'static str,
    status: Option<efi::Status>,
}

impl Error {
    fn new(msg: &'static str) -> Self {
        Error { msg, status: None }
    }
}

impl From<&'static str> for Error {
    fn from(msg: &'static str) -> Self {
        Error::new(msg)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.msg)?;
        if let Some(s) = self.status {
            write!(f, ": {} ({s:#x})", efi::status_name(s))?;
        }
        Ok(())
    }
}

/// Turns an EFI status into a `Result`, attaching `msg` on failure.
fn check(s: efi::Status, msg: &'static str) -> Result<(), Error> {
    if s == efi::SUCCESS {
        Ok(())
    } else {
        Err(Error {
            msg,
            status: Some(s),
        })
    }
}

#[unsafe(no_mangle)]
/// # Safety
/// Called by the firmware with its image handle and system table.
pub unsafe extern "efiapi" fn efi_main(image: efi::Handle, st: *mut efi::SystemTable) -> efi::Status {
    console::serial_init();
    // SAFETY: the firmware passes a valid system table.
    let st = unsafe { &*st };
    console::set_con_out(st.con_out);
    progress!("SegOS bootloader {}", env!("CARGO_PKG_VERSION"));
    let Err(e) = boot(image, st);
    progress!("error: {e}");
    info!("cannot boot SegOS; returning to the firmware in 10 seconds");
    // SAFETY: boot services are still active on every error path of boot().
    unsafe { ((*st.boot_services).stall)(10_000_000) };
    e.status.unwrap_or(efi::LOAD_ERROR)
}

/// The kernel file, read into pool memory.
struct KernelFile {
    data: *mut u8,
    len: usize,
}

fn protocol<T>(
    bs: &efi::BootServices,
    handle: efi::Handle,
    guid: &efi::Guid,
    what: &'static str,
) -> Result<*mut T, Error> {
    let mut p: *mut c_void = null_mut();
    // SAFETY: valid handle, GUID and out pointer.
    check(unsafe { (bs.handle_protocol)(handle, guid, &mut p) }, what)?;
    Ok(p.cast())
}

fn read_kernel(bs: &efi::BootServices, image: efi::Handle) -> Result<KernelFile, Error> {
    let loaded: *mut efi::LoadedImage = protocol(
        bs,
        image,
        &efi::LOADED_IMAGE_GUID,
        "cannot get the loaded image protocol",
    )?;
    // SAFETY: the firmware returned a valid protocol interface.
    let device = unsafe { (*loaded).device_handle };
    let fs: *mut efi::SimpleFileSystem = protocol(
        bs,
        device,
        &efi::SIMPLE_FILE_SYSTEM_GUID,
        "the boot partition has no file system the firmware can read",
    )?;
    let mut root: *mut efi::File = null_mut();
    // SAFETY: valid protocol interfaces and out pointers throughout.
    unsafe {
        check(
            ((*fs).open_volume)(fs, &mut root),
            "cannot open the boot partition",
        )?;
        let mut file: *mut efi::File = null_mut();
        let s = ((*root).open)(
            root,
            &mut file,
            KERNEL_PATH.as_ptr(),
            efi::FILE_MODE_READ,
            0,
        );
        ((*root).close)(root);
        if s == efi::NOT_FOUND {
            return Err(Error::new(
                "kernel not found: \\boot\\segkernel is missing from the boot partition",
            ));
        }
        check(s, "cannot open \\boot\\segkernel")?;
        let result = read_whole(bs, file);
        ((*file).close)(file);
        result
    }
}

/// Reads an open file completely into pool memory.
unsafe fn read_whole(bs: &efi::BootServices, file: *mut efi::File) -> Result<KernelFile, Error> {
    let mut info = [0u64; 128];
    let mut size = core::mem::size_of_val(&info);
    // SAFETY: valid file and a buffer of `size` bytes.
    unsafe {
        check(
            ((*file).get_info)(file, &efi::FILE_INFO_GUID, &mut size, info.as_mut_ptr().cast()),
            "cannot get the size of \\boot\\segkernel",
        )?;
        let info = &*(info.as_ptr() as *const efi::FileInfo);
        if info.attribute & efi::FILE_DIRECTORY != 0 {
            return Err(Error::new("\\boot\\segkernel is a directory"));
        }
        let len = info.file_size as usize;
        if len == 0 {
            return Err(Error::new("\\boot\\segkernel is empty"));
        }
        let mut data: *mut u8 = null_mut();
        check(
            (bs.allocate_pool)(efi::LOADER_DATA, len, &mut data),
            "out of memory reading the kernel",
        )?;
        let mut done = 0;
        while done < len {
            let mut n = (len - done).min(READ_CHUNK);
            let s = ((*file).read)(file, &mut n, data.add(done).cast());
            if s != efi::SUCCESS || n == 0 {
                (bs.free_pool)(data);
                if s == efi::SUCCESS {
                    return Err(Error::new("\\boot\\segkernel: unexpected end of file"));
                }
                return Err(Error {
                    msg: "error reading \\boot\\segkernel",
                    status: Some(s),
                });
            }
            done += n;
        }
        Ok(KernelFile { data, len })
    }
}

/// The kernel's segments, loaded into one physically contiguous block.
struct Kernel {
    phys: u64,
    virt: u64,
    pages: u64,
    entry: u64,
}

fn load_kernel(
    bs: *mut efi::BootServices,
    elf: &elf::Elf,
    pt: &mut PageTables,
) -> Result<Kernel, Error> {
    let lo = elf.segments().map(|s| s.vaddr).min().unwrap() & !(PAGE - 1);
    let hi = elf.segments().map(|s| s.vaddr + s.memsz).max().unwrap();
    let pages = (hi - lo).div_ceil(PAGE);
    let phys = mem::allocate_pages(bs, efi::ALLOCATE_ANY_PAGES, mem::KERNEL_TYPE, pages, 0)
        .map_err(|s| Error {
            msg: "out of memory for the kernel image",
            status: Some(s),
        })?;
    // SAFETY: freshly allocated, identity-mapped pages. Zeroing the whole
    // block also zeroes .bss and any gaps between segments.
    unsafe { core::ptr::write_bytes(phys as *mut u8, 0, (pages * PAGE) as usize) };
    for s in elf.segments() {
        let dst = phys + (s.vaddr - lo);
        let src = elf.bytes(&s);
        // SAFETY: `dst..dst + filesz` lies inside the block (filesz <= memsz).
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len()) };
        let first = s.vaddr & !(PAGE - 1);
        let mut v = first;
        while v < s.vaddr + s.memsz {
            pt.map_4k(
                v,
                phys + (v - lo),
                s.flags & elf::PF_W != 0,
                s.flags & elf::PF_X != 0,
            )?;
            v += PAGE;
        }
    }
    Ok(Kernel {
        phys,
        virt: lo,
        pages,
        entry: elf.entry,
    })
}

const NO_FRAMEBUFFER: Framebuffer = Framebuffer {
    address: 0,
    width: 0,
    height: 0,
    pitch: 0,
    bpp: 0,
    format: PixelFormat::Bgrx,
    _reserved: 0,
};

fn pixel_format(f: u32) -> Option<PixelFormat> {
    match f {
        efi::PIXEL_RGB_RESERVED_8BIT => Some(PixelFormat::Rgbx),
        efi::PIXEL_BGR_RESERVED_8BIT => Some(PixelFormat::Bgrx),
        // PixelBitMask and PixelBltOnly: no linear 32-bit framebuffer we
        // can describe.
        _ => None,
    }
}

/// The GOP framebuffer: the current mode if it is 32-bit RGBX/BGRX,
/// otherwise the largest mode that is.
fn framebuffer(bs: &efi::BootServices, st: &efi::SystemTable) -> (Framebuffer, u64) {
    let mut gop: *mut c_void = null_mut();
    // SAFETY: valid GUID and out pointers. Prefer the GOP on the console.
    let found = unsafe {
        (bs.handle_protocol)(st.console_out_handle, &efi::GRAPHICS_OUTPUT_GUID, &mut gop)
            == efi::SUCCESS
            || (bs.locate_protocol)(&efi::GRAPHICS_OUTPUT_GUID, null_mut(), &mut gop)
                == efi::SUCCESS
    };
    if !found || gop.is_null() {
        info!("no graphics output protocol: booting without a framebuffer");
        return (NO_FRAMEBUFFER, 0);
    }
    let gop = gop as *mut efi::GraphicsOutput;
    // SAFETY: the GOP interface and its mode structures are valid while boot
    // services are active.
    unsafe {
        let mode = (*gop).mode;
        if pixel_format((*(*mode).info).pixel_format).is_none() {
            let mut best: Option<(u32, u64)> = None;
            for m in 0..(*mode).max_mode {
                let mut size = 0;
                let mut info: *const efi::GraphicsModeInfo = core::ptr::null();
                if ((*gop).query_mode)(gop, m, &mut size, &mut info) != efi::SUCCESS {
                    continue;
                }
                let area =
                    (*info).horizontal_resolution as u64 * (*info).vertical_resolution as u64;
                if pixel_format((*info).pixel_format).is_some()
                    && best.is_none_or(|(_, a)| area > a)
                {
                    best = Some((m, area));
                }
                (bs.free_pool)(info as *mut u8);
            }
            if let Some((m, _)) = best {
                ((*gop).set_mode)(gop, m);
            }
        }
        let mode = (*gop).mode;
        let info = &*(*mode).info;
        let Some(format) = pixel_format(info.pixel_format) else {
            info!("graphics mode has no 32-bit linear framebuffer: booting without one");
            return (NO_FRAMEBUFFER, 0);
        };
        let pitch = info.pixels_per_scan_line * 4;
        let fb = Framebuffer {
            address: (*mode).frame_buffer_base,
            width: info.horizontal_resolution,
            height: info.vertical_resolution,
            pitch,
            bpp: 32,
            format,
            _reserved: 0,
        };
        let size = ((*mode).frame_buffer_size as u64).max(pitch as u64 * fb.height as u64);
        (fb, size)
    }
}

fn find_rsdp(st: &efi::SystemTable) -> u64 {
    if st.configuration_table.is_null() {
        return 0;
    }
    // SAFETY: the firmware provides `number_of_table_entries` entries.
    let tables = unsafe {
        core::slice::from_raw_parts(st.configuration_table, st.number_of_table_entries)
    };
    let find = |g: efi::Guid| {
        tables
            .iter()
            .find(|t| t.vendor_guid == g)
            .map(|t| t.vendor_table as u64)
    };
    find(efi::ACPI_20_TABLE_GUID)
        .or_else(|| find(efi::ACPI_10_TABLE_GUID))
        .unwrap_or(0)
}

fn alloc_reclaimable(bs: *mut efi::BootServices, bytes: u64, what: &'static str) -> Result<u64, Error> {
    let pages = bytes.div_ceil(PAGE);
    let p = mem::allocate_pages(bs, efi::ALLOCATE_ANY_PAGES, mem::RECLAIMABLE_TYPE, pages, 0)
        .map_err(|s| Error {
            msg: what,
            status: Some(s),
        })?;
    // SAFETY: freshly allocated, identity-mapped pages.
    unsafe { core::ptr::write_bytes(p as *mut u8, 0, (pages * PAGE) as usize) };
    Ok(p)
}

fn boot(image: efi::Handle, st: &efi::SystemTable) -> Result<core::convert::Infallible, Error> {
    let bs_ptr = st.boot_services;
    // SAFETY: valid while boot services are active, i.e. for all of boot()
    // until ExitBootServices succeeds.
    let bs = unsafe { &*bs_ptr };
    // No firmware watchdog reset while we work.
    // SAFETY: plain boot service call.
    unsafe { (bs.set_watchdog_timer)(0, 0, 0, core::ptr::null()) };

    // SAFETY: reading CR4 has no side effects.
    let cr4: u64 = unsafe {
        let v;
        asm!("mov {}, cr4", out(reg) v, options(nomem, nostack));
        v
    };
    if cr4 & (1 << 12) != 0 {
        return Err(Error::new(
            "the firmware enabled 5-level paging, which segboot does not support",
        ));
    }

    info!("loading \\boot\\segkernel");
    let file = read_kernel(bs, image)?;
    // SAFETY: read_whole filled `len` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(file.data, file.len) };
    let elf = match elf::Elf::parse(bytes) {
        Ok(e) => e,
        Err(msg) => {
            progress!("bad kernel ELF: {msg}");
            // SAFETY: allocated by read_whole.
            unsafe { (bs.free_pool)(file.data) };
            return Err(Error::new("\\boot\\segkernel is not a valid SegOS kernel"));
        }
    };

    let mut pt = PageTables::new(bs_ptr)?;
    let kernel = load_kernel(bs_ptr, &elf, &mut pt)?;
    // SAFETY: allocated by read_whole; the segments have been copied.
    unsafe { (bs.free_pool)(file.data) };
    progress!(
        "kernel loaded: {} KiB at {:#x}, entry {:#x}",
        kernel.pages * 4,
        kernel.phys,
        kernel.entry
    );

    let (fb, fb_size) = framebuffer(bs, st);
    if fb.address != 0 {
        info!(
            "framebuffer {}x{} at {:#x}, pitch {}, {:?}",
            fb.width, fb.height, fb.address, fb.pitch, fb.format
        );
    }
    let rsdp = find_rsdp(st);
    if rsdp == 0 {
        info!("warning: no ACPI RSDP in the configuration table");
    }

    // Size the memory map buffers now, with room to spare: every allocation
    // from here on can split a map entry.
    let mut map_size = 0usize;
    let mut map_key = 0usize;
    let mut desc_size = 0usize;
    let mut desc_version = 0u32;
    // SAFETY: a zero-sized query only writes the out parameters.
    let s = unsafe {
        (bs.get_memory_map)(
            &mut map_size,
            null_mut(),
            &mut map_key,
            &mut desc_size,
            &mut desc_version,
        )
    };
    if s != efi::BUFFER_TOO_SMALL || desc_size < core::mem::size_of::<efi::MemoryDescriptor>() {
        return Err(Error {
            msg: "cannot query the memory map size",
            status: Some(s),
        });
    }
    let raw_cap = (map_size * 2 + 64 * desc_size) as u64;
    let raw_map = alloc_reclaimable(bs_ptr, raw_cap, "out of memory for the memory map")?;
    let regions_cap = raw_cap as usize / desc_size + 8;
    let regions_phys = alloc_reclaimable(
        bs_ptr,
        (regions_cap * core::mem::size_of::<MemoryRegion>()) as u64,
        "out of memory for the memory map",
    )?;
    let info_phys = alloc_reclaimable(
        bs_ptr,
        core::mem::size_of::<BootInfo>() as u64,
        "out of memory for the boot information",
    )?;
    let stack_phys = alloc_reclaimable(bs_ptr, STACK_SIZE, "out of memory for the kernel stack")?;

    // Highest address to direct-map and identity-map.
    let mut size = raw_cap as usize;
    // SAFETY: the buffer holds `size` bytes.
    check(
        unsafe {
            (bs.get_memory_map)(
                &mut size,
                raw_map as *mut efi::MemoryDescriptor,
                &mut map_key,
                &mut desc_size,
                &mut desc_version,
            )
        },
        "cannot read the memory map",
    )?;
    let top = mem::top_of_memory(raw_map, size, desc_size)
        .max(4 * GIB)
        .max(fb.address + fb_size)
        .next_multiple_of(GIB);
    let gib_pages = paging::has_gib_pages();
    pt.map_large(0, 0, top, true, gib_pages)?;
    pt.map_large(HHDM_OFFSET, 0, top, false, gib_pages)?;
    info!(
        "page tables at {:#x}: {} GiB mapped with {} pages, {} KiB of tables",
        pt.pml4,
        top / GIB,
        if gib_pages { "1 GiB" } else { "2 MiB" },
        pt.used * 4
    );

    let info = info_phys as *mut BootInfo;
    // SAFETY: a zeroed, page-sized, page-aligned allocation.
    unsafe {
        info.write(BootInfo {
            magic: MAGIC,
            version: VERSION,
            _reserved: 0,
            hhdm_offset: HHDM_OFFSET,
            kernel_phys_base: kernel.phys,
            kernel_virt_base: kernel.virt,
            memory_map: (HHDM_OFFSET + regions_phys) as *const MemoryRegion,
            memory_map_len: 0,
            framebuffer: fb,
            rsdp,
        })
    };

    progress!("exiting boot services");
    console::drop_con_out();
    let mut tries = 0;
    loop {
        let mut size = raw_cap as usize;
        // SAFETY: the buffer holds `size` bytes. After a failed
        // ExitBootServices only GetMemoryMap and ExitBootServices are used.
        let s = unsafe {
            (bs.get_memory_map)(
                &mut size,
                raw_map as *mut efi::MemoryDescriptor,
                &mut map_key,
                &mut desc_size,
                &mut desc_version,
            )
        };
        if s != efi::SUCCESS {
            fatal(format_args!("cannot read the final memory map ({s:#x})"));
        }
        // SAFETY: the key belongs to the map just read.
        let s = unsafe { (bs.exit_boot_services)(image, map_key) };
        if s == efi::SUCCESS {
            map_size = size;
            break;
        }
        tries += 1;
        if s != efi::INVALID_PARAMETER || tries == 16 {
            fatal(format_args!("ExitBootServices failed ({s:#x})"));
        }
    }

    // Boot services are gone: from here on only COM1 works.
    let fb_range = if fb.address != 0 {
        (fb.address, fb.address + fb_size)
    } else {
        (0, 0)
    };
    // SAFETY: allocated above with room for `regions_cap` entries; every
    // entry is written before it is read.
    let out = unsafe {
        core::slice::from_raw_parts_mut(regions_phys as *mut MemoryRegion, regions_cap)
    };
    let len = match mem::convert(raw_map, map_size, desc_size, fb_range, out) {
        Ok(n) => n,
        Err(e) => fatal(format_args!("{e}")),
    };
    let usable: u64 = out[..len]
        .iter()
        .filter(|r| r.kind == bootinfo::MemoryKind::Usable)
        .map(|r| r.length)
        .sum();
    // SAFETY: written above.
    unsafe { (*info).memory_map_len = len as u64 };
    serial!(
        "boot services exited; {} memory regions, {} MiB usable",
        len,
        usable >> 20
    );
    serial!("jumping to the kernel at {:#x}", kernel.entry);

    // SAFETY: the page tables map the kernel, the direct map (holding the
    // new stack and BootInfo) and the identity map this code runs from.
    unsafe {
        jump(
            pt.pml4,
            HHDM_OFFSET + stack_phys + STACK_SIZE,
            kernel.entry,
            HHDM_OFFSET + info_phys,
        )
    }
}

/// Unrecoverable failure after boot services were (being) exited.
fn fatal(args: fmt::Arguments) -> ! {
    console::serial_print(format_args!("segboot: fatal: {args}\n"));
    halt()
}

fn halt() -> ! {
    loop {
        // SAFETY: stops the CPU.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

/// Switches to the new page tables and stack and calls the kernel entry.
unsafe fn jump(pml4: u64, stack_top: u64, entry: u64, info: u64) -> ! {
    // SAFETY: the caller guarantees the mappings described in bootinfo.
    unsafe {
        asm!("cli", options(nomem, nostack));
        // EFER.NXE before CR3: our tables use the NX bit.
        asm!(
            "rdmsr",
            "or eax, 1 << 11",
            "wrmsr",
            in("ecx") 0xc000_0080u32,
            out("eax") _,
            out("edx") _,
            options(nomem, nostack)
        );
        asm!(
            "mov cr3, {pml4}",
            "mov rax, cr0",
            "or rax, 1 << 16", // CR0.WP
            "mov cr0, rax",
            // rsp 16-byte aligned just before the call, as the SysV ABI
            // expects; the pushed return address is never used.
            "mov rsp, {stack}",
            "xor ebp, ebp",
            "call {entry}",
            "2:",
            "cli",
            "hlt",
            "jmp 2b",
            pml4 = in(reg) pml4,
            stack = in(reg) stack_top,
            entry = in(reg) entry,
            in("rdi") info,
            // Scratch register (clobbering an input is fine: noreturn).
            in("rax") 0u64,
            options(noreturn)
        )
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    console::both_print(format_args!("segboot: panic: {info}\n"));
    halt()
}
