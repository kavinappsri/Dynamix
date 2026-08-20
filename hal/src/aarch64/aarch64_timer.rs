//! Time related functions for Aarch64

use core::arch::asm;

#[inline(always)]
fn get_cntfrq() -> u64 {
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

#[inline(always)]
fn get_cntpct() -> u64 {
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

pub fn delay_ms(ms: u64) {
    let frq = get_cntfrq();
    let ticks_needed = (frq * ms) / 1000;
    let start_ticks = get_cntpct();

    while (get_cntpct() - start_ticks) < ticks_needed {
        unsafe {
            asm!("yield", options(nomem, nostack, preserves_flags));
        }
    }
}

