//! Reusable UEFI boot time helpers

use crate::x86_64::memory_map;
use crate::x86_64::memory_map::{MemoryMap, MemoryMapError};
use crate::x86_64::uefi_types::{self, EfiBootServices, EfiHandle, EfiSimpleTextOutputProtocol, EfiStatus,};

/// `L"Dynamix UEFI boot stage reached.\r\n"` as UCS-2 code units, NUL terminated.

pub const BOOT_MESSAGE: &[u16] = &[
    'D' as u16, 'y' as u16, 'n' as u16, 'a' as u16, 'm' as u16, 'i' as u16, 'x' as u16,
    ' ' as u16, 'U' as u16, 'E' as u16, 'F' as u16, 'I' as u16, ' ' as u16, 'b' as u16,
    'o' as u16, 'o' as u16, 't' as u16, ' ' as u16, 's' as u16, 't' as u16, 'a' as u16,
    'g' as u16, 'e' as u16, ' ' as u16, 'r' as u16, 'e' as u16, 'a' as u16, 'c' as u16,
    'h' as u16, 'e' as u16, 'd' as u16, '.' as u16, '\r' as u16, '\n' as u16, 0,
];

/// Writes a NUL-terminated UCS-2 string to the given ConOut protocol.
///
/// # Safety
/// `con_out` must be a currently-valid `EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL*`
/// (i.e. Boot Services must not have exited yet), and `message` must be
/// NUL-terminated UCS-2 as UEFI's `OutputString` requires.
pub unsafe fn print(con_out: *mut EfiSimpleTextOutputProtocol, message: &[u16]) {
    // SAFETY: upheld by this function's own contract.
    unsafe {
        ((*con_out).output_string)(con_out, message.as_ptr());
    }
}

pub unsafe fn println_ascii(con_out: *mut EfiSimpleTextOutputProtocol, message: &str) {
    // UEFI's OutputString has no length-based overload, so this writes in
    // fixed-size NUL-terminated chunks rather than allocating one buffer
    // sized to the whole (unbounded-length) input.
    const CHUNK: usize = 63;
    let mut buf = [0u16; CHUNK + 1];

    let bytes = message.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let take = core::cmp::min(CHUNK, bytes.len() - offset);
        for i in 0..take {
            let byte = bytes[offset + i];
            buf[i] = if byte.is_ascii() { byte as u16 } else { b'?' as u16 };
        }
        buf[take] = 0;
        // SAFETY: upheld by this function's own contract; `buf[..=take]` is
        // NUL-terminated UCS-2 by construction above.
        unsafe { print(con_out, &buf[..=take]) };
        offset += take;
    }

    // SAFETY: as above; this is the fixed `\r\n\0` terminator chunk.
    unsafe { print(con_out, &['\r' as u16, '\n' as u16, 0]) };
}

pub fn format_hex(value: usize, buf: &mut [u8]) -> &str {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let width = size_of::<usize>() * 2;
    assert!(buf.len() >= width + 2, "format_hex buffer too small");

    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..width {
        let shift = (width - 1 - i) * 4;
        let nibble = (value >> shift) & 0xF;
        buf[2 + i] = HEX_DIGITS[nibble];
    }

    // SAFETY: every byte written above is ASCII ('0'-'9', 'a'-'f', 'x').
    unsafe { core::str::from_utf8_unchecked(&buf[..width + 2]) }
}

/// How many times `ExitBootServices` is retried when firmware says the memory
/// map key went stale.
const EXIT_BOOT_SERVICES_ATTEMPTS: usize = 4;

/// Why [`exit_boot_services`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitBootServicesError {
    /// Fetching the memory map failed.
    MemoryMap(MemoryMapError),
    /// `ExitBootServices` itself kept returning an error status.
    Exit(EfiStatus),
}

/// Fetches the memory map and calls `ExitBootServices` with its key,
/// retrying with a fresh map if firmware reports the key as stale.
///
/// On success, returns the map the exit succeeded with: the final, accurate
/// picture of memory.
///
/// After `Ok`, Boot Services are gone: `ConOut`, `LocateProtocol`, and every
/// other firmware call made through the boot services table or the console
/// are invalid. Nothing may be allocated or printed between the map fetch and
/// the exit - that is what invalidates the key, which is why this does both.
///
/// # Safety
/// `image_handle` must be the handle `efi_main` was given, and
/// `boot_services` must still be valid (i.e. not yet exited).
pub unsafe fn exit_boot_services(
    image_handle: EfiHandle,
    boot_services: *mut EfiBootServices,
) -> Result<MemoryMap, ExitBootServicesError> {
    let mut last_status = uefi_types::EFI_INVALID_PARAMETER;

    for _ in 0..EXIT_BOOT_SERVICES_ATTEMPTS {
        // SAFETY: upheld by this function's own contract.
        let map = unsafe { memory_map::get_memory_map(boot_services) }
            .map_err(ExitBootServicesError::MemoryMap)?;

        // SAFETY: as above; `map.map_key` is from the call just made.
        let status = unsafe { ((*boot_services).exit_boot_services)(image_handle, map.map_key) };

        if !uefi_types::efi_is_error(status) {
            return Ok(map);
        }

        last_status = status;
        if status != uefi_types::EFI_INVALID_PARAMETER {
            break;
        }
    }

    Err(ExitBootServicesError::Exit(last_status))
}

