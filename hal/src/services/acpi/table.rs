//! Holding an ACPI table, and reading plain structs out of its bytes.
//!

use core::marker::PhantomData;
use core::mem::size_of;
use core::ptr::{self, NonNull};

use acpica_sys as sys;

use super::AcpiError;
use super::raw::check;

/// Which table a [`Table`] is: its four-letter signature, NUL-terminated
/// because that is what ACPICA wants.
pub(super) trait TableKind {
    const SIGNATURE: &'static [u8; 5];
}

/// A table ACPICA has mapped and verified, kept alive until dropped.
pub(super) struct Table<K: TableKind> {
    header: NonNull<sys::ACPI_TABLE_HEADER>,
    length: usize,
    _kind: PhantomData<K>,
}

impl<K: TableKind> Table<K> {
    /// Looks the table up by signature. `instance` counts from 1 (only SSDTs
    /// and a few others appear more than once).
    pub(super) fn get(instance: u32) -> Result<Self, AcpiError> {
        let mut found: *mut sys::ACPI_TABLE_HEADER = ptr::null_mut();
        // SAFETY: the signature is NUL-terminated and ACPICA only reads it;
        // `found` is a valid out-pointer.
        check(unsafe { sys::AcpiGetTable(K::SIGNATURE.as_ptr().cast_mut().cast(), instance, &mut found) })?;
        let header = NonNull::new(found).ok_or(AcpiError::NotFound)?;

        // SAFETY: ACPICA mapped at least the header; it is packed, so the
        // length is read unaligned.
        let length = unsafe { ptr::addr_of!((*header.as_ptr()).Length).read_unaligned() } as usize;
        // From here on `Drop` gives the table back, whatever happens next.
        let table = Self { header, length, _kind: PhantomData };
        if length < size_of::<sys::ACPI_TABLE_HEADER>() {
            return Err(AcpiError::BadParameter);
        }
        Ok(table)
    }

    /// The whole table, header included.
    pub(super) fn bytes(&self) -> &[u8] {
        // SAFETY: ACPICA mapped `length` bytes (the table's own length field,
        // checked against the header size) and keeps them mapped until
        // `AcpiPutTable`, which only `Drop` calls. Table memory is not
        // modified while the OS runs.
        unsafe { core::slice::from_raw_parts(self.header.as_ptr().cast::<u8>(), self.length) }
    }
}

impl<K: TableKind> Drop for Table<K> {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `AcpiGetTable` and is given back once.
        unsafe { sys::AcpiPutTable(self.header.as_ptr()) };
    }
}

/// A C struct made only of integers, so every bit pattern is a valid value.
///
/// # Safety
/// Implement it only for such `Copy` types.
pub(super) unsafe trait Pod: Copy {}

/// Copies a `T` out of `bytes` at `offset`, or `None` if it does not fit.
pub(super) fn read_pod<T: Pod>(bytes: &[u8], offset: usize) -> Option<T> {
    let end = offset.checked_add(size_of::<T>())?;
    if end > bytes.len() {
        return None;
    }
    // SAFETY: in bounds, any bit pattern is valid for a `Pod`, and the read
    // is unaligned because table structs are packed.
    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<T>()) })
}
