//! Platform power-control primitives.

use core::arch::asm;

/// Requests a PSCI system shutdown through an HVC call.
///
/// The current platform path uses an environment that handles the PSCI
/// `SYSTEM_OFF` function ID through `hvc #0`; it does not return on success.
#[inline]
pub fn shut() {
    unsafe {
        asm!(
        "hvc #0",
        in("x0") 0x84000008u64,
        options(nostack, nomem)
        );
    }
}
