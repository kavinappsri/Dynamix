//! Turns `ACPI_STATUS` into a result, and rust-compatible types

use core::ffi::CStr;

use acpica_sys as sys;

use super::AcpiError;

/// ACPICA's own name for a status (`AE_NOT_FOUND`, ...).
pub(super) fn status_name(status: u32) -> &'static str {
    // SAFETY: AcpiFormatException returns a pointer to a static,
    // NUL-terminated string, never null.
    unsafe { CStr::from_ptr(sys::AcpiFormatException(status)) }.to_str().unwrap_or("AE_?")
}

/// `AE_CODE_AML` and friends live in the top nibble of a status.
const CODE_CLASS_MASK: u32 = 0xF000;
const CODE_CLASS_AML: u32 = 0x3000;

impl AcpiError {
    /// Maps a failed `ACPI_STATUS`.
    pub(super) fn from_status(status: u32) -> Self {
        match status {
            sys::AE_NOT_FOUND | sys::AE_NOT_EXIST => AcpiError::NotFound,
            sys::AE_NO_MEMORY => AcpiError::NoMemory,
            sys::AE_BAD_PARAMETER => AcpiError::BadParameter,
            sys::AE_ALREADY_EXISTS => AcpiError::AlreadyExists,
            sys::AE_NOT_IMPLEMENTED | sys::AE_SUPPORT => AcpiError::NotImplemented,
            sys::AE_TIME => AcpiError::Timeout,
            code if code & CODE_CLASS_MASK == CODE_CLASS_AML => AcpiError::Aml(code),
            code => AcpiError::Status(code),
        }
    }
}

/// `ACPI_SUCCESS` as a `Result`.
pub(super) fn check(status: u32) -> Result<(), AcpiError> {
    if sys::acpi_success(status) { Ok(()) } else { Err(AcpiError::from_status(status)) }
}
