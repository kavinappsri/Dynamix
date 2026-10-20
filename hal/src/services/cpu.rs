//! Architecture-neutral CPU control.


pub fn hlt_frvr() -> ! {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: masking interrupts has no memory effects, and nothing here
    // depends on them being enabled.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    loop {
        // SAFETY: both instructions only idle the core until the next event.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }

        #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
        unsafe {
            core::arch::asm!("wfe", options(nomem, nostack));
        }
    }
}
