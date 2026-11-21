//! Time related functions for Aarch64

use core::arch::asm;

use crate::services::interrupts::Irq;
use crate::services::timer::ns_to_ticks;

/// The non-secure physical timer's PPI on every board seen so far, used when
/// the device tree does not say.
/// The non-secure physical timer's PPI, 30 on every GIC platform we know,
/// used if the device tree does not say.
pub const FALLBACK_TIMER_IRQ: Irq = Irq(30);

/// CNTP_CTL_EL0: timer enabled, interrupt not masked.
const CTL_ENABLE: u64 = 1;

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

pub fn delay_us(us: u64) {
    let frq = get_cntfrq();
    let ticks_needed = frq.saturating_mul(us) / 1_000_000;
    let start_ticks = get_cntpct();

    while (get_cntpct() - start_ticks) < ticks_needed {
        unsafe {
            asm!("yield", options(nomem, nostack, preserves_flags));
        }
    }
}

pub fn now_ns() -> u64 {
    crate::services::timer::ticks_to_ns(get_cntpct(), get_cntfrq())
}

#[inline(always)]
fn set_cntp_cval(value: u64) {
    // SAFETY: programs the physical timer's compare value, nothing else.
    unsafe {
        asm!("msr cntp_cval_el0, {}", "isb", in(reg) value, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
fn set_cntp_ctl(value: u64) {
    // SAFETY: programs the physical timer's control register, nothing else.
    unsafe {
        asm!("msr cntp_ctl_el0, {}", "isb", in(reg) value, options(nomem, nostack, preserves_flags));
    }
}

/// Leaves the event timer (the physical timer) disarmed.
pub fn event_init() {
    set_cntp_ctl(0);
}

/// Fires the timer interrupt when `now_ns()` reaches `deadline_ns`.
pub fn event_arm(deadline_ns: u64) {
    // The comparator is absolute, on the same counter `now_ns` reads.
    set_cntp_cval(ns_to_ticks(deadline_ns, get_cntfrq()));
    set_cntp_ctl(CTL_ENABLE);
}

/// Stops the timer; the interrupt line deasserts.
pub fn event_disarm() {
    set_cntp_ctl(0);
}
