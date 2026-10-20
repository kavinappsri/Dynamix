//! Time related functions for armv7

use core::arch::asm;

#[inline(always)]
fn get_cntfrq() -> u32 {
    let val: u32;
    unsafe {
        asm!(
        "mrc p15, 0, {}, c14, c0, 0",
        out(reg) val,
        options(nomem, nostack, preserves_flags)
        );
    }
    val
}

#[inline]
fn get_cntpct() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
        "mrrc p15, 0, {}, {}, c14",
        out(reg) low,
        out(reg) high,
        options(nomem, nostack, preserves_flags)
        );
    }
    ((high as u64) << 32) | (low as u64)
}

pub fn delay_ms(ms: u64) {
    let frq = get_cntfrq();
    let ticks_needed = ((frq as u64) * ms) / 1000;
    let start_ticks = get_cntpct();

    while (get_cntpct() - start_ticks) < ticks_needed {
        unsafe {
            asm!("yield", options(nomem, nostack, preserves_flags));
        }
    }
}