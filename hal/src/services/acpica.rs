//! ACPICA integration, step 1: proof that the C library is linked into the
//! kernel and runs on top of our OS services layer ([`super::acpica_osl`]).
//!
//! TEMPORARY. Step 2 replaces this file with the real safe wrapper; until
//! then [`smoke_test`] is all there is. It exists on every architecture so
//! the kernel can call it without a per-arch path, and simply reports
//! "unsupported" where ACPICA is not built.

/// Runs ACPICA far enough to read a table back, then shuts it down again.
///
/// 1. calls a pure-C function (links the library, touches no OSL service);
/// 2. `AcpiInitializeSubsystem`, which exercises allocation, locks and the
///    clock of the OSL;
/// 3. `AcpiInitializeTables` + `AcpiGetTable("FACP")`, which exercise
///    `AcpiOsGetRootPointer` and `AcpiOsMapMemory` on the firmware's real
///    tables;
/// 4. `AcpiTerminate`, so the real initialization in step 2 starts clean.
///    It logs "Could not remove SCI handler": the handler is only installed
///    by a full `AcpiEnableSubsystem`, which this test does not do. Harmless.
///
/// Needs the kernel heap, so call it after the PMM and heap are up.
/// `rsdp` is the physical address of the RSDP. On failure the stage is
/// returned and ACPICA's own status name is written to the serial console.
pub fn smoke_test(rsdp: u64) -> Result<(), &'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        imp::run(rsdp)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = rsdp;
        Err("ACPICA is only built for x86_64")
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::ffi::CStr;
    use core::fmt::Write;
    use core::ptr;

    use acpica_sys as sys;

    use crate::driver_traits::serial::{SerialWriter, active_serial};
    use crate::services::acpica_osl;

    /// ACPICA's own name for a status code (`AE_NOT_FOUND`, ...).
    fn status_name(status: u32) -> &'static str {
        // SAFETY: AcpiFormatException returns a pointer to a static,
        // NUL-terminated string (never null).
        unsafe { CStr::from_ptr(sys::AcpiFormatException(status)) }.to_str().unwrap_or("AE_?")
    }

    /// Turns a failed status into `Err(stage)`, writing the status name out.
    fn check(status: u32, stage: &'static str) -> Result<(), &'static str> {
        if sys::acpi_success(status) {
            return Ok(());
        }
        if let Some(uart) = active_serial() {
            let _ = writeln!(SerialWriter(uart), "[acpica] {} -> {}", stage, status_name(status));
        }
        Err(stage)
    }

    pub fn run(rsdp: u64) -> Result<(), &'static str> {
        // Stage 1: C code that needs nothing from the OSL.
        if status_name(sys::AE_NOT_FOUND) != "AE_NOT_FOUND" {
            return Err("AcpiFormatException gave the wrong name");
        }

        // Stage 2 to 4: ACPICA on top of the OSL.
        acpica_osl::init(rsdp);

        // SAFETY: single-threaded, called once; pointers passed are valid for
        // the duration of each call, and the table header is only read while
        // the table is held (before `AcpiPutTable`).
        unsafe {
            check(sys::AcpiInitializeSubsystem(), "AcpiInitializeSubsystem")?;
            check(sys::AcpiInitializeTables(ptr::null_mut(), 16, 1), "AcpiInitializeTables")?;

            let mut fadt: *mut sys::ACPI_TABLE_HEADER = ptr::null_mut();
            check(sys::AcpiGetTable(sys::ACPI_SIG_FADT.as_ptr().cast_mut().cast(), 1, &mut fadt), "AcpiGetTable(FACP)")?;

            // The header is a packed struct, so its fields are read unaligned.
            let signature = ptr::addr_of!((*fadt).Signature).read_unaligned();
            let length = ptr::addr_of!((*fadt).Length).read_unaligned();
            sys::AcpiPutTable(fadt);
            if signature.map(|byte| byte as u8) != *b"FACP" || (length as usize) < core::mem::size_of::<sys::ACPI_TABLE_HEADER>() {
                return Err("FADT header is malformed");
            }

            check(sys::AcpiTerminate(), "AcpiTerminate")?;
        }
        Ok(())
    }
}
