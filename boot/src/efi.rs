//! Minimal hand-written UEFI bindings: only what segboot uses.
//!
//! Layouts follow the UEFI 2.10 specification. Function slots segboot never
//! calls are declared as `usize` so the structures keep their size.

use core::ffi::c_void;

pub type Handle = *mut c_void;
pub type Status = usize;

pub const SUCCESS: Status = 0;
const ERROR_BIT: usize = 1 << 63;
pub const INVALID_PARAMETER: Status = ERROR_BIT | 2;
pub const UNSUPPORTED: Status = ERROR_BIT | 3;
pub const BUFFER_TOO_SMALL: Status = ERROR_BIT | 5;
pub const OUT_OF_RESOURCES: Status = ERROR_BIT | 9;
pub const NOT_FOUND: Status = ERROR_BIT | 14;
pub const LOAD_ERROR: Status = ERROR_BIT | 1;

/// Human-readable name of an EFI status code.
pub fn status_name(s: Status) -> &'static str {
    match s {
        SUCCESS => "success",
        LOAD_ERROR => "load error",
        INVALID_PARAMETER => "invalid parameter",
        UNSUPPORTED => "unsupported",
        BUFFER_TOO_SMALL => "buffer too small",
        OUT_OF_RESOURCES => "out of resources",
        NOT_FOUND => "not found",
        x if x == ERROR_BIT | 7 => "device error",
        x if x == ERROR_BIT | 8 => "write protected",
        x if x == ERROR_BIT | 10 => "volume corrupted",
        x if x == ERROR_BIT | 15 => "access denied",
        _ => "error",
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Guid {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

pub const LOADED_IMAGE_GUID: Guid = Guid {
    a: 0x5b1b31a1,
    b: 0x9562,
    c: 0x11d2,
    d: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};
pub const SIMPLE_FILE_SYSTEM_GUID: Guid = Guid {
    a: 0x964e5b22,
    b: 0x6459,
    c: 0x11d2,
    d: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};
pub const FILE_INFO_GUID: Guid = Guid {
    a: 0x09576e92,
    b: 0x6d3f,
    c: 0x11d2,
    d: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};
pub const GRAPHICS_OUTPUT_GUID: Guid = Guid {
    a: 0x9042a9de,
    b: 0x23dc,
    c: 0x4a38,
    d: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};
pub const ACPI_20_TABLE_GUID: Guid = Guid {
    a: 0x8868e871,
    b: 0xe4f1,
    c: 0x11d3,
    d: [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};
pub const ACPI_10_TABLE_GUID: Guid = Guid {
    a: 0xeb9d2d30,
    b: 0x2d88,
    c: 0x11d3,
    d: [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3c, 0xc1, 0x4d],
};

#[repr(C)]
pub struct TableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct SystemTable {
    pub hdr: TableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: Handle,
    pub con_in: *mut c_void,
    pub console_out_handle: Handle,
    pub con_out: *mut SimpleTextOutput,
    pub standard_error_handle: Handle,
    pub std_err: *mut SimpleTextOutput,
    pub runtime_services: *mut c_void,
    pub boot_services: *mut BootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: *const ConfigurationTable,
}

#[repr(C)]
pub struct ConfigurationTable {
    pub vendor_guid: Guid,
    pub vendor_table: *mut c_void,
}

#[repr(C)]
pub struct SimpleTextOutput {
    pub reset: usize,
    pub output_string: unsafe extern "efiapi" fn(*mut SimpleTextOutput, *const u16) -> Status,
    pub test_string: usize,
    pub query_mode: usize,
    pub set_mode: usize,
    pub set_attribute: usize,
    pub clear_screen: usize,
    pub set_cursor_position: usize,
    pub enable_cursor: usize,
    pub mode: *mut c_void,
}

// EFI_ALLOCATE_TYPE
pub const ALLOCATE_ANY_PAGES: u32 = 0;
pub const ALLOCATE_MAX_ADDRESS: u32 = 1;

// EFI_MEMORY_TYPE
pub const LOADER_CODE: u32 = 1;
pub const LOADER_DATA: u32 = 2;
pub const BOOT_SERVICES_CODE: u32 = 3;
pub const BOOT_SERVICES_DATA: u32 = 4;
pub const CONVENTIONAL_MEMORY: u32 = 7;
pub const UNUSABLE_MEMORY: u32 = 8;
pub const ACPI_RECLAIM_MEMORY: u32 = 9;
pub const ACPI_MEMORY_NVS: u32 = 10;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryDescriptor {
    pub ty: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

#[repr(C)]
pub struct BootServices {
    pub hdr: TableHeader,
    pub raise_tpl: usize,
    pub restore_tpl: usize,
    pub allocate_pages: unsafe extern "efiapi" fn(u32, u32, usize, *mut u64) -> Status,
    pub free_pages: unsafe extern "efiapi" fn(u64, usize) -> Status,
    pub get_memory_map: unsafe extern "efiapi" fn(
        *mut usize,
        *mut MemoryDescriptor,
        *mut usize,
        *mut usize,
        *mut u32,
    ) -> Status,
    pub allocate_pool: unsafe extern "efiapi" fn(u32, usize, *mut *mut u8) -> Status,
    pub free_pool: unsafe extern "efiapi" fn(*mut u8) -> Status,
    pub create_event: usize,
    pub set_timer: usize,
    pub wait_for_event: usize,
    pub signal_event: usize,
    pub close_event: usize,
    pub check_event: usize,
    pub install_protocol_interface: usize,
    pub reinstall_protocol_interface: usize,
    pub uninstall_protocol_interface: usize,
    pub handle_protocol: unsafe extern "efiapi" fn(Handle, *const Guid, *mut *mut c_void) -> Status,
    pub reserved: usize,
    pub register_protocol_notify: usize,
    pub locate_handle: usize,
    pub locate_device_path: usize,
    pub install_configuration_table: usize,
    pub load_image: usize,
    pub start_image: usize,
    pub exit: usize,
    pub unload_image: usize,
    pub exit_boot_services: unsafe extern "efiapi" fn(Handle, usize) -> Status,
    pub get_next_monotonic_count: usize,
    pub stall: unsafe extern "efiapi" fn(usize) -> Status,
    pub set_watchdog_timer: unsafe extern "efiapi" fn(usize, u64, usize, *const u16) -> Status,
    pub connect_controller: usize,
    pub disconnect_controller: usize,
    pub open_protocol: usize,
    pub close_protocol: usize,
    pub open_protocol_information: usize,
    pub protocols_per_handle: usize,
    pub locate_handle_buffer: usize,
    pub locate_protocol:
        unsafe extern "efiapi" fn(*const Guid, *mut c_void, *mut *mut c_void) -> Status,
    pub install_multiple_protocol_interfaces: usize,
    pub uninstall_multiple_protocol_interfaces: usize,
    pub calculate_crc32: usize,
    pub copy_mem: usize,
    pub set_mem: usize,
    pub create_event_ex: usize,
}

#[repr(C)]
pub struct LoadedImage {
    pub revision: u32,
    pub parent_handle: Handle,
    pub system_table: *mut SystemTable,
    pub device_handle: Handle,
    pub file_path: *mut c_void,
    pub reserved: *mut c_void,
    pub load_options_size: u32,
    pub load_options: *mut c_void,
    pub image_base: *mut c_void,
    pub image_size: u64,
    pub image_code_type: u32,
    pub image_data_type: u32,
    pub unload: usize,
}

#[repr(C)]
pub struct SimpleFileSystem {
    pub revision: u64,
    pub open_volume: unsafe extern "efiapi" fn(*mut SimpleFileSystem, *mut *mut File) -> Status,
}

pub const FILE_MODE_READ: u64 = 1;
pub const FILE_DIRECTORY: u64 = 0x10;

#[repr(C)]
pub struct File {
    pub revision: u64,
    pub open: unsafe extern "efiapi" fn(*mut File, *mut *mut File, *const u16, u64, u64) -> Status,
    pub close: unsafe extern "efiapi" fn(*mut File) -> Status,
    pub delete: usize,
    pub read: unsafe extern "efiapi" fn(*mut File, *mut usize, *mut c_void) -> Status,
    pub write: usize,
    pub get_position: usize,
    pub set_position: usize,
    pub get_info: unsafe extern "efiapi" fn(*mut File, *const Guid, *mut usize, *mut c_void) -> Status,
    pub set_info: usize,
    pub flush: usize,
}

/// Fixed-size head of EFI_FILE_INFO (a variable-length file name follows).
#[repr(C)]
pub struct FileInfo {
    pub size: u64,
    pub file_size: u64,
    pub physical_size: u64,
    pub create_time: [u8; 16],
    pub last_access_time: [u8; 16],
    pub modification_time: [u8; 16],
    pub attribute: u64,
}

// EFI_GRAPHICS_PIXEL_FORMAT
pub const PIXEL_RGB_RESERVED_8BIT: u32 = 0;
pub const PIXEL_BGR_RESERVED_8BIT: u32 = 1;

#[repr(C)]
pub struct GraphicsOutput {
    pub query_mode: unsafe extern "efiapi" fn(
        *mut GraphicsOutput,
        u32,
        *mut usize,
        *mut *const GraphicsModeInfo,
    ) -> Status,
    pub set_mode: unsafe extern "efiapi" fn(*mut GraphicsOutput, u32) -> Status,
    pub blt: usize,
    pub mode: *const GraphicsMode,
}

#[repr(C)]
pub struct GraphicsMode {
    pub max_mode: u32,
    pub mode: u32,
    pub info: *const GraphicsModeInfo,
    pub size_of_info: usize,
    pub frame_buffer_base: u64,
    pub frame_buffer_size: usize,
}

#[repr(C)]
pub struct GraphicsModeInfo {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    pub pixel_format: u32,
    pub pixel_information: [u32; 4],
    pub pixels_per_scan_line: u32,
}
