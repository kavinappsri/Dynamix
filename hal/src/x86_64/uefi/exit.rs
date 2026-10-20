//! `ExitBootServices`, with the retry dance firmware requires.

use super::memory_map::{MemoryMap, MemoryMapError};
use super::types::{self, EfiBootServices, EfiHandle, EfiStatus};

/// How many times `ExitBootServices` is retried 
const EXIT_BOOT_SERVICES_ATTEMPTS: usize = 4;

/// Why [`exit_boot_services`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitBootServicesError {
    /// Fetching the memory map failed.
    MemoryMap(MemoryMapError),
    /// `ExitBootServices` itself kept returning an error status.
    Exit(EfiStatus),
}

/// Fetches the memory map into `map` and calls `ExitBootServices` with its
/// key, refetching if firmware reports the key as stale.
///
/// # Safety
/// `image_handle` must be the handle `efi_main` was given, and
/// `boot_services` must still be valid 
pub unsafe fn exit_boot_services(
    image_handle: EfiHandle,
    boot_services: *mut EfiBootServices,
    map: &mut MemoryMap,
) -> Result<(), ExitBootServicesError> {
    let mut last_status = types::EFI_INVALID_PARAMETER;

    for _ in 0..EXIT_BOOT_SERVICES_ATTEMPTS {
        // SAFETY: upheld by this function's own contract.
        unsafe { map.refresh(boot_services) }.map_err(ExitBootServicesError::MemoryMap)?;

        // SAFETY: as above; `map.map_key` is from the call just made.
        let status = unsafe { ((*boot_services).exit_boot_services)(image_handle, map.map_key) };

        if !types::efi_is_error(status) {
            return Ok(());
        }

        last_status = status;
        if status != types::EFI_INVALID_PARAMETER {
            break;
        }
    }

    Err(ExitBootServicesError::Exit(last_status))
}
