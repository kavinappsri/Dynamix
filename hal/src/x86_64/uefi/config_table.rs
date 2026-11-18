//! RSDP lookup via the EFI Configuration Table.
//!
//! Only locates the RSDP pointer and checks it looks like one; reading the
//! tables behind it is ACPICA's job (`services::acpi`).

use super::types::{EfiConfigurationTable, EfiGuid, EfiSystemTable, EFI_ACPI_10_TABLE_GUID, EFI_ACPI_20_TABLE_GUID};

/// Errors that can occur while locating the RSDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsdpError {
    /// No configuration table entry had an ACPI GUID.
    NotFound,
    /// The RSDP's signature or checksum is wrong.
    Invalid,
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

/// Size of the ACPI 1.0 part of the RSDP, which the first checksum covers.
const RSDP_V1_LEN: usize = 20;
/// Offsets inside the RSDP.
const RSDP_REVISION: usize = 15;
const RSDP_LENGTH: usize = 20;
/// Smallest and largest sane size of an ACPI 2.0 RSDP.
const RSDP_V2_MIN_LEN: usize = 36;
const RSDP_V2_MAX_LEN: usize = 4096;

/// Checks the RSDP's signature and checksums, and returns its revision
/// (0 for ACPI 1.0, 2 or more for 2.0+).
///
/// # Safety
/// `rsdp` must point at readable memory of at least the RSDP's size (firmware
/// maps it while Boot Services are alive).
pub unsafe fn check_rsdp(rsdp: *const core::ffi::c_void) -> Result<u8, RsdpError> {
    let sum = |bytes: &[u8]| bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));

    // SAFETY: the first 20 bytes exist in every RSDP revision.
    let v1 = unsafe { core::slice::from_raw_parts(rsdp.cast::<u8>(), RSDP_V1_LEN) };
    if &v1[..8] != b"RSD PTR " || sum(v1) != 0 {
        return Err(RsdpError::Invalid);
    }

    let revision = v1[RSDP_REVISION];
    if revision >= 2 {
        // SAFETY: a revision 2+ RSDP has the length field at offset 20, and
        // the table is at least that big.
        let length = unsafe { core::ptr::read_unaligned(rsdp.cast::<u8>().add(RSDP_LENGTH).cast::<u32>()) } as usize;
        if !(RSDP_V2_MIN_LEN..=RSDP_V2_MAX_LEN).contains(&length) {
            return Err(RsdpError::Invalid);
        }
        // SAFETY: the RSDP says it is `length` bytes long.
        let whole = unsafe { core::slice::from_raw_parts(rsdp.cast::<u8>(), length) };
        if sum(whole) != 0 {
            return Err(RsdpError::Invalid);
        }
    }
    Ok(revision)
}
