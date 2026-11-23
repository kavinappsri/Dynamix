//! Running AML objects: methods like `_Q3A`, or reading a value like `_GPE`.
//!
//! This can currently run one and ignore the result, or read
//! one integer. Decoding the whole zoo of AML result types would
//! go here too when we need it.

use alloc::vec::Vec;
use core::mem::{MaybeUninit, size_of};
use core::ptr::{self, NonNull};
use core::slice;

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

    ///  the entries of `_DEP` that are device references
    /// `NotFound` if the device has no `_DEP`.
    ///
    /// Runs AML.
    pub fn dependencies(&self) -> Result<Vec<Device>, AcpiError> {
        let name = c_name::<32>("_DEP")?;
        // ACPICA allocates a buffer of the right size when the length is -1.
        let mut buffer = sys::ACPI_BUFFER { Length: usize::MAX as _, Pointer: ptr::null_mut() };

        // SAFETY: valid handle and NUL-terminated name; ACPICA fills in `buffer`.
        check(unsafe {
            sys::AcpiEvaluateObjectTyped(
                self.handle.as_ptr(),
                name.as_ptr().cast_mut().cast(),
                ptr::null_mut(),
                &mut buffer,
                sys::ACPI_TYPE_PACKAGE,
            )
        })?;
        let result = ReturnBuffer(NonNull::new(buffer.Pointer).ok_or(AcpiError::NotFound)?);

        // SAFETY: the typed evaluation succeeded, so the buffer holds a
        // package object whose `Count` elements follow it in the same
        // allocation, which `result` keeps alive.
        let elements = unsafe {
            let package = (*result.0.as_ptr().cast::<sys::ACPI_OBJECT>()).Package;
            if package.Elements.is_null() {
                &[][..]
            } else {
                slice::from_raw_parts(package.Elements, package.Count as usize)
            }
        };

        let mut devices = Vec::new();
        for element in elements {
            // SAFETY: `Type` is the tag every variant of the union starts with.
            if unsafe { element.Type } != sys::ACPI_TYPE_LOCAL_REFERENCE {
                continue;
            }
            // SAFETY: it is a reference, so `Reference` is the live variant.
            let handle = unsafe { element.Reference.Handle };
            if let Some(handle) = NonNull::new(handle) {
                devices.try_reserve(1).map_err(|_| AcpiError::NoMemory)?;
                devices.push(Device { handle });
            }
        }
        Ok(devices)
    }
}

/// A result buffer ACPICA allocated for us, freed on drop.
struct ReturnBuffer(NonNull<core::ffi::c_void>);

impl Drop for ReturnBuffer {
    fn drop(&mut self) {
        // SAFETY: allocated by ACPICA through `AcpiOsAllocate`, freed once.
        unsafe { crate::services::acpica_osl::AcpiOsFree(self.0.as_ptr().cast()) };
    }
}
