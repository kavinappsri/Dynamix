//! The one error type of the ACPI layer.

use core::fmt;

/// Why an ACPI operation failed.
///
/// ACPICA reports `AE_*` status codes; the common ones get a name here, the
/// rest are kept as they came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AcpiError {
    /// ACPICA is not built for this architecture.
    Unsupported,
    /// Asked too early: ACPICA (or its namespace) is not up yet.
    NotReady,
    /// `init` was already called.
    AlreadyStarted,
    /// The table, object or device is not there (`AE_NOT_FOUND`).
    NotFound,
    /// ACPICA ran out of memory (`AE_NO_MEMORY`).
    NoMemory,
    /// ACPICA did not like an argument (`AE_BAD_PARAMETER`).
    BadParameter,
    /// ACPICA says it is there already (`AE_ALREADY_EXISTS`).
    AlreadyExists,
    /// A feature ACPICA (or our OS layer) does not do (`AE_NOT_IMPLEMENTED`).
    NotImplemented,
    /// Something waited too long (`AE_TIME`).
    Timeout,
    /// The AML interpreter tripped over firmware code (`AE_AML_*`). The
    /// number is ACPICA's status code.
    Aml(u32),
    /// Any other ACPICA status code.
    Status(u32),
}

impl fmt::Display for AcpiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcpiError::Unsupported => write!(f, "ACPICA is not available on this architecture"),
            AcpiError::NotReady => write!(f, "ACPI is not ready for that yet"),
            AcpiError::AlreadyStarted => write!(f, "ACPI was already started"),
            AcpiError::NotFound => write!(f, "not found"),
            AcpiError::NoMemory => write!(f, "out of memory"),
            AcpiError::BadParameter => write!(f, "bad parameter"),
            AcpiError::AlreadyExists => write!(f, "already exists"),
            AcpiError::NotImplemented => write!(f, "not implemented"),
            AcpiError::Timeout => write!(f, "timed out"),
            AcpiError::Aml(code) | AcpiError::Status(code) => {
                write!(f, "{} ({:#06x})", status_name(*code), code)
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn status_name(code: u32) -> &'static str {
    super::raw::status_name(code)
}

#[cfg(not(target_arch = "x86_64"))]
fn status_name(_code: u32) -> &'static str {
    "AE_?"
}