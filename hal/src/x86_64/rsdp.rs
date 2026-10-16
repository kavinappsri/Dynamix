//! RSDP lookup via the EFI Configuration Table.
//!
//! Currently, this only locates the RSDP pointer. It doesn't parse
//! the ACPI tables.

use crate::x86_64::uefi_types;

/// Errors that can occur while locating the RSDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsdpError {
    /// No configuration table entry had the ACPI 2.0 GUID.
    NotFound,
}

/// Walks the EFI Configuration Table looking for the ACPI 2.0 RSDP.
pub unsafe fn locate_rsdp(system_table: *const uefi_types::EfiSystemTable) -> Result<*const core::ffi::c_void, RsdpError> {
    let system_table = unsafe { &*system_table };

    if system_table.configuration_table.is_null() {
        return Err(RsdpError::NotFound);
    }

    let entries = unsafe {
        core::slice::from_raw_parts(
            system_table.configuration_table,
            system_table.number_of_table_entries,
        )
    };

    entries
        .iter()
        .find(|entry| guid_eq(&entry.vendor_guid, &uefi_types::EFI_ACPI_20_TABLE_GUID))
        .map(|entry: &uefi_types::EfiConfigurationTable| entry.vendor_table as *const core::ffi::c_void)
        .ok_or(RsdpError::NotFound)
}

fn guid_eq(a: &uefi_types::EfiGuid, b: &uefi_types::EfiGuid) -> bool {
    a == b
}
