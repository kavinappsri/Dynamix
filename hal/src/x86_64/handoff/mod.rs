mod pe_image;
pub mod paging;
pub mod stack;

use crate::services::boot_info::BootInfo;
use crate::services::cpu;

/// The address range the kernel image occupies right now, for diagnostics.
/// `None` until the image has been measured.
pub fn image_bounds() -> Option<(usize, usize)> {
    let size = paging::image_size()?;
    let base = pe_image::runtime_base();
    Some((base, base + size))
}

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
    unsafe { crate::boot::enter_kernel() }
}
