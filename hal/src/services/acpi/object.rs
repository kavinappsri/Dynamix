//! Running AML objects: methods like `_Q3A`, or reading a value like `_GPE`.
//!
//! This can currently run one and ignore the result, or read
//! one integer. Decoding the whole zoo of AML result types would
//! go here too when we need it.

use core::mem::{MaybeUninit, size_of};
use core::ptr;

use acpica_sys as sys;

use super::namespace::Device;
use super::raw::{c_name, check};
use super::AcpiError;

impl Device {
    /// Runs the method `name` (a path relative to the device, like `_Q3A`)
    /// with no arguments and throws away what it returns.
    ///
    /// Runs AML, so it can be slow and it can fail on bad firmware.
    /// `NotFound` means the device has no such object.
    pub fn evaluate(&self, name: &str) -> Result<(), AcpiError> {
        let name = c_name::<32>(name)?;
        // SAFETY: valid handle, NUL-terminated name, no arguments, no result wanted.
        check(unsafe {
            sys::AcpiEvaluateObject(self.handle.as_ptr(), name.as_ptr().cast_mut().cast(), ptr::null_mut(), ptr::null_mut())
        })
    }

    /// Reads the integer `name` (a path relative to the device, like `_GPE`).
    /// Anything that is not an integer fails with `Status(AE_TYPE)`.
    pub fn evaluate_integer(&self, name: &str) -> Result<u64, AcpiError> {
        let name = c_name::<32>(name)?;
        let mut object = MaybeUninit::<sys::ACPI_OBJECT>::zeroed();
        let mut buffer =
            sys::ACPI_BUFFER { Length: size_of::<sys::ACPI_OBJECT>() as _, Pointer: object.as_mut_ptr().cast() };

        // SAFETY: valid handle and NUL-terminated name; the buffer is big
        // enough for any integer object, which ACPICA copies into it.
        check(unsafe {
            sys::AcpiEvaluateObjectTyped(
                self.handle.as_ptr(),
                name.as_ptr().cast_mut().cast(),
                ptr::null_mut(),
                &mut buffer,
                sys::ACPI_TYPE_INTEGER,
            )
        })?;
        // SAFETY: the typed evaluation succeeded, so ACPICA wrote an
        // integer object.
        Ok(unsafe { (*object.as_ptr()).Integer.Value })
    }
}
