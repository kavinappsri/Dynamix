//! provides basic time-related functions during boot

use core::arch::asm;

/// Returns the tick frequency of the clock
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

/// Returns the current ticks elapsed as counted by the clock
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

/// Blocks for a certain amount of time in milliseconds
///
/// Accepts the amount of milliseconds as a `u64` and the clock frequency as `u64`
///
/// # Examples
///
/// ```
/// let frq = power::get_cntfrq();
/// power::delay_ms(1000, frq); // Blocks for 1 second
/// ```
pub fn delay_ms(ms: u64, frq: u64) {
    let ticks_needed = (frq * ms)  / 1000;
    let start_ticks = get_cntpct();

    while (get_cntpct() - start_ticks) < ticks_needed {
        unsafe { asm!("yield", options(nomem, nostack, preserves_flags)); }
    }
}
