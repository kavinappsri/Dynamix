//! Provides a API to control general power systems

use core::arch::asm;

/// Shuts down the device
///
/// This function shuts down the device its run on
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
