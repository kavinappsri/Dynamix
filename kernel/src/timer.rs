//! AArch64 architectural timer access for early boot delays.

use core::arch::asm;

/// Returns the architectural counter frequency in ticks per second.
#[inline(always)]
pub fn get_cntfrq() -> u64 {
    let val: u64;
    unsafe {
        asm!(
        "mrs {}, cntfrq_el0",
        out(reg) val,
        options(nomem, nostack, preserves_flags)
        );
    }
    val
}

/// Returns the current value of the architectural physical count register.
#[inline(always)]
pub fn get_cntpct() -> u64 {
    let val: u64;
    unsafe {
        asm!(
        "mrs {}, cntpct_el0",
        out(reg) val,
        options(nomem, nostack, preserves_flags)
        );
    }
    val
}

/// Busy-waits for `ms` milliseconds using an architectural timer frequency.
///
/// Pass the value returned by [`get_cntfrq`] as `frq`. The wait consumes a CPU
/// and should only be used during early boot or when no scheduler exists.
///
/// # Examples
///
/// ```
/// let frq = timer::get_cntfrq();
/// timer::delay_ms(1000, frq); // Blocks for 1 second
/// ```
///
/// # Panics
///
/// Panics in debug builds if `frq * ms` overflows `u64`.
pub fn delay_ms(ms: u64, frq: u64) {
    let ticks_needed = (frq * ms) / 1000;
    let start_ticks = get_cntpct();

    while (get_cntpct() - start_ticks) < ticks_needed {
        unsafe {
            asm!("yield", options(nomem, nostack, preserves_flags));
        }
    }
}
