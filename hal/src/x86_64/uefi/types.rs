//! UEFI Type and protocol definitions

///UEFI's provided efi status
pub type EfiStatus = usize;

pub const EFI_SUCCESS: EfiStatus = 0;

const ERROR_BIT: EfiStatus = 1 << 63;
pub const EFI_LOAD_ERROR: EfiStatus = ERROR_BIT | 1;
pub const EFI_INVALID_PARAMETER: EfiStatus = ERROR_BIT | 2;
pub const EFI_BUFFER_TOO_SMALL: EfiStatus = ERROR_BIT | 5;
pub const EFI_NOT_FOUND: EfiStatus = ERROR_BIT | 14;

#[inline]
pub const fn efi_is_error(status: EfiStatus) -> bool {
    status & ERROR_BIT != 0
}

/// Opaque firmware-owned handle.
pub type EfiHandle = *mut core::ffi::c_void;

/// A UEFI GUID
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EfiGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x9042_a9de,
    data2: 0x23dc,
    data3: 0x4a38,
    data4: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};

pub const EFI_ACPI_20_TABLE_GUID: EfiGuid = EfiGuid {
    data1: 0x8868_e871,
    data2: 0xe4f1,
    data3: 0x11d3,
    data4: [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};

pub const EFI_ACPI_10_TABLE_GUID: EfiGuid = EfiGuid {
    data1: 0xeb9d_2d30,
    data2: 0x2d88,
    data3: 0x11d3,
    data4: [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4f],
};

// Console Output
#[repr(C)]
pub struct EfiSimpleTextOutputProtocol {
    pub reset: unsafe extern "efiapi" fn(
        this: *mut EfiSimpleTextOutputProtocol,
        extended_verification: bool,
    ) -> EfiStatus,
    pub output_string: unsafe extern "efiapi" fn(
        this: *mut EfiSimpleTextOutputProtocol,
        string: *const u16,
    ) -> EfiStatus,
    // Remaining fields exist in the real protocol but are unused by us;
    // keep their space so field offsets below stay correct if we add more.
    _test_string: usize,
    _query_mode: usize,
    _set_mode: usize,
    _set_attribute: usize,
    _clear_screen: usize,
    _set_cursor_position: usize,
    _enable_cursor: usize,
    _mode: usize,
}

// Memory Map

pub type EfiMemoryType = u32;

pub const EFI_CONVENTIONAL_MEMORY: EfiMemoryType = 7;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiMemoryDescriptor {
    pub ty: EfiMemoryType,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

//Boot Services

#[repr(C)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct EfiBootServices {
    pub hdr: EfiTableHeader,

    // Task Priority services - unused, kept for offset correctness.
    _raise_tpl: usize,
    _restore_tpl: usize,

    // Memory services
    pub allocate_pages: unsafe extern "efiapi" fn(
        alloc_type: u32,
        memory_type: EfiMemoryType,
        pages: usize,
        memory: *mut u64,
    ) -> EfiStatus,
    pub free_pages:
        unsafe extern "efiapi" fn(memory: u64, pages: usize) -> EfiStatus,
    pub get_memory_map: unsafe extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut EfiMemoryDescriptor,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> EfiStatus,
    pub allocate_pool: unsafe extern "efiapi" fn(
        pool_type: EfiMemoryType,
        size: usize,
        buffer: *mut *mut core::ffi::c_void,
    ) -> EfiStatus,
    pub free_pool:
        unsafe extern "efiapi" fn(buffer: *mut core::ffi::c_void) -> EfiStatus,

    // Event & timer services - unused, kept for offset correctness.
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,

    // Protocol handler services - unused except locate_protocol below.
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
    _handle_protocol: usize,
    _reserved: usize,
    _register_protocol_notify: usize,
    _locate_handle: usize,
    _locate_device_path: usize,
    _install_configuration_table: usize,

    // Image services - unused, kept for offset correctness.
    _load_image: usize,
    _start_image: usize,
    _exit: usize,
    _unload_image: usize,

    pub exit_boot_services:
        unsafe extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus,

    // Misc services - unused, kept for offset correctness.
    _get_next_monotonic_count: usize,
    _stall: usize,
    _set_watchdog_timer: usize,

    // Driver support services - unused, kept for offset correctness.
    _connect_controller: usize,
    _disconnect_controller: usize,

    // Open/close protocol - unused, kept for offset correctness.
    _open_protocol: usize,
    _close_protocol: usize,
    _open_protocol_information: usize,

    // Library services
    _protocols_per_handle: usize,
    _locate_handle_buffer: usize,
    pub locate_protocol: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        registration: *mut core::ffi::c_void,
        interface: *mut *mut core::ffi::c_void,
    ) -> EfiStatus,
    // The remainder of the real table (install/uninstall multiple protocol
    // interfaces, CalculateCrc32, Copy/SetMem, CreateEventEx) is unused.
}


// configuration table

#[repr(C)]
pub struct EfiConfigurationTable {
    pub vendor_guid: EfiGuid,
    pub vendor_table: *mut core::ffi::c_void,
}

// system table

#[repr(C)]
pub struct EfiSystemTable {
    pub hdr: EfiTableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: EfiHandle,
    _con_in: usize,
    pub console_out_handle: EfiHandle,
    pub con_out: *mut EfiSimpleTextOutputProtocol,
    pub standard_error_handle: EfiHandle,
    pub std_err: *mut EfiSimpleTextOutputProtocol,
    _runtime_services: usize,
    pub boot_services: *mut EfiBootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: *mut EfiConfigurationTable,
}

// GOP

pub type EfiGraphicsPixelFormat = u32;

pub const PIXEL_RGB_RESERVED_8BIT_PER_COLOR: EfiGraphicsPixelFormat = 0;
pub const PIXEL_BGR_RESERVED_8BIT_PER_COLOR: EfiGraphicsPixelFormat = 1;
pub const PIXEL_BIT_MASK: EfiGraphicsPixelFormat = 2;
pub const PIXEL_BLT_ONLY: EfiGraphicsPixelFormat = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiPixelBitmask {
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiGraphicsOutputModeInformation {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    pub pixel_format: EfiGraphicsPixelFormat,
    pub pixel_information: EfiPixelBitmask,
    pub pixels_per_scan_line: u32,
}

#[repr(C)]
pub struct EfiGraphicsOutputProtocolMode {
    pub max_mode: u32,
    pub mode: u32,
    pub info: *mut EfiGraphicsOutputModeInformation,
    pub size_of_info: usize,
    pub frame_buffer_base: u64,
    pub frame_buffer_size: usize,
}

#[repr(C)]
pub struct EfiGraphicsOutputProtocol {
    pub query_mode: unsafe extern "efiapi" fn(
        this: *mut EfiGraphicsOutputProtocol,
        mode_number: u32,
        size_of_info: *mut usize,
        info: *mut *mut EfiGraphicsOutputModeInformation,
    ) -> EfiStatus,
    pub set_mode: unsafe extern "efiapi" fn(
        this: *mut EfiGraphicsOutputProtocol,
        mode_number: u32,
    ) -> EfiStatus,
    _blt: usize,
    pub mode: *mut EfiGraphicsOutputProtocolMode,
}
