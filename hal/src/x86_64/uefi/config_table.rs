//! RSDP lookup via the EFI Configuration Table.
//!
//! Only locates the RSDP pointer; parsing is `services::acpi`'s job.

use super::types::{EfiConfigurationTable, EfiGuid, EfiSystemTable, EFI_ACPI_10_TABLE_GUID, EFI_ACPI_20_TABLE_GUID};

/// Errors that can occur while locating the RSDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsdpError {
    /// No configuration table entry had an ACPI GUID.
    NotFound,
}

/// Walks the EFI Configuration Table looking for the RSDP, preferring the
/// ACPI 2.0 entry and falling back to the ACPI 1.0 one.
///
/// # Safety
/// `system_table` must be the valid system table handed to `efi_main`.
pub unsafe fn locate_rsdp(system_table: *const EfiSystemTable) -> Result<*const core::ffi::c_void, RsdpError> {
    // SAFETY: upheld by the caller.
    let system_table = unsafe { &*system_table };

    if system_table.configuration_table.is_null() {
        return Err(RsdpError::NotFound);
    }

    // SAFETY: firmware guarantees `number_of_table_entries` valid entries.
    let entries = unsafe {
        core::slice::from_raw_parts(system_table.configuration_table, system_table.number_of_table_entries)
    };

    let find = |guid: &EfiGuid| {
        entries
            .iter()
            .find(|entry: &&EfiConfigurationTable| entry.vendor_guid == *guid)
            .map(|entry| entry.vendor_table as *const core::ffi::c_void)
    };

    find(&EFI_ACPI_20_TABLE_GUID)
        .or_else(|| find(&EFI_ACPI_10_TABLE_GUID))
        .ok_or(RsdpError::NotFound)
}
