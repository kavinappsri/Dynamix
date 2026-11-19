//! `Notify` messages from firmware, and general-purpose events (GPEs).

use core::ffi::{CStr, c_void};
use core::ptr;

use acpica_sys as sys;

use super::raw::check;
use super::AcpiError;
use crate::services::klog::{self, Level, Source};

/// `ACPI_ROOT_OBJECT`: the root of the namespace, which is `-1` as a pointer.
const ROOT: sys::ACPI_HANDLE = usize::MAX as sys::ACPI_HANDLE;

/// Longest namespace path the `Notify` logger prints.
const PATH_CAPACITY: usize = 160;

/// Logs every `Notify` (system and device) in the namespace.
pub(super) fn install_notify_logger() {
    // SAFETY: the handler matches the signature ACPICA wants and uses no context.
    let installed = check(unsafe {
        sys::AcpiInstallNotifyHandler(ROOT, sys::ACPI_ALL_NOTIFY, Some(log_notify), ptr::null_mut())
    });
    if let Err(err) = installed {
        klog::log(Source::Acpi, Level::Warn, format_args!("Could not install the Notify logger: {err}"));
    }
}

/// ACPICA calls this (from the work queue) for every `Notify`.
unsafe extern "C" fn log_notify(device: sys::ACPI_HANDLE, value: u32, _context: *mut c_void) {
    let mut path = [0u8; PATH_CAPACITY];
    let mut buffer = sys::ACPI_BUFFER { Length: PATH_CAPACITY as _, Pointer: path.as_mut_ptr().cast() };
    // SAFETY: `device` is the node ACPICA is notifying; the buffer is ours.
    let named = check(unsafe { sys::AcpiGetName(device, sys::ACPI_FULL_PATHNAME, &mut buffer) }).is_ok();
    let name = named.then(|| CStr::from_bytes_until_nul(&path).ok()).flatten().and_then(|c| c.to_str().ok());

    klog::log(Source::Acpi, Level::Info, format_args!("Notify {}: {value:#x}", name.unwrap_or("?")));
}

/// Enables every GPE that has a method (`_Lxx` / `_Exx`) or a handler that
/// asked to be enabled.
pub(super) fn enable_gpes() -> Result<(), AcpiError> {
    // SAFETY: plain ACPICA call.
    check(unsafe { sys::AcpiUpdateAllGpes() })
}
