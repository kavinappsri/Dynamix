mod pe_image;
pub mod paging;
pub mod stack;

use crate::services::acpi::AcpiQuery;
use crate::services::boot_info::{BootInfo, HardwareDescription};
use crate::services::cpu;

/// Moves onto the kernel's boot stack and continues into `rust_main`.
///
/// # Safety
/// Boot Services must have exited, interrupts must be masked, and `boot`
/// must be complete.
pub unsafe fn enter_kernel(boot: &mut BootInfo<'static>) -> ! {
    // SAFETY: upheld by the caller.
    unsafe { stack::switch_and_call(boot, stage2) }
}


unsafe extern "efiapi" fn stage2(boot: *mut BootInfo<'static>) -> ! {
    // SAFETY: `enter_kernel` passed a valid, initialised `BootInfo`.
    crate::boot::store(unsafe { core::ptr::read(boot) });

    // SAFETY: first and only call, on the image as firmware loaded it.
    match unsafe { paging::prepare() } {
        // SAFETY: `prepared` was just built for this image.
        Ok(prepared) => unsafe { paging::activate(prepared, higher_half_entry) },
        Err(_) => {
            // CR3 was not touched, so firmware's mappings (and the
            // framebuffer) are still live: paint red and stop.
            // SAFETY: stored just above.
            if let Some(framebuffer) = unsafe { crate::boot::stored() }.framebuffer {
                framebuffer.fill(0x00_FF_00_00);
            }
            cpu::hlt_frvr()
        }
    }
}


unsafe extern "efiapi" fn higher_half_entry() -> ! {
    // SAFETY: `stage2` stored the boot info before the jump.
    let boot = unsafe { crate::boot::stored() };

    // The ACPI query was built under firmware's page tables; its cached root
    // table pointer is not mapped in ours. Re-read it before the kernel uses it.
    let acpi_unusable = match &mut boot.hardware {
        Some(HardwareDescription::Acpi(acpi)) => unsafe { AcpiQuery::remap(acpi) }.is_err(),
        _ => false,
    };
    if acpi_unusable {
        boot.hardware = None;
    }

    // SAFETY: the boot info is complete and `rust_main` never returns.
    unsafe { crate::boot::enter_kernel() }
}
