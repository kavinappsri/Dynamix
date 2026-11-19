//! Time related functions for armv7

use core::arch::asm;

use crate::services::device_query::DeviceQuery;
use crate::services::interrupts::Irq;
use crate::services::timer::ns_to_ticks;

/// The non-secure physical timer's PPI on every board seen so far, used when
/// the device tree does not say.
const FALLBACK_TIMER_IRQ: Irq = Irq(30);

/// CNTP_CTL: timer enabled, interrupt not masked.
const CTL_ENABLE: u32 = 1;

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

pub fn delay_us(us: u64) {
    let frq = get_cntfrq();
    let ticks_needed = ((frq as u64).saturating_mul(us)) / 1_000_000;
    let start_ticks = get_cntpct();

    while (get_cntpct() - start_ticks) < ticks_needed {
        unsafe {
            asm!("yield", options(nomem, nostack, preserves_flags));
        }
    }
}

pub fn now_ns() -> u64 {
    crate::services::timer::ticks_to_ns(get_cntpct(), get_cntfrq() as u64)
}

#[inline(always)]
fn set_cntp_cval(value: u64) {
    // SAFETY: programs the physical timer's compare value, nothing else.
    unsafe {
        asm!(
        "mcrr p15, 2, {lo}, {hi}, c14",
        "isb",
        lo = in(reg) value as u32,
        hi = in(reg) (value >> 32) as u32,
        options(nomem, nostack, preserves_flags)
        );
    }
}

#[inline(always)]
fn set_cntp_ctl(value: u32) {
    // SAFETY: programs the physical timer's control register, nothing else.
    unsafe {
        asm!("mcr p15, 0, {}, c14, c2, 1", "isb", in(reg) value, options(nomem, nostack, preserves_flags));
    }
}

/// Leaves the event timer (the physical timer) disarmed and returns its interrupt.
pub fn event_init(hardware: &dyn DeviceQuery) -> Irq {
    set_cntp_ctl(0);
    crate::services::timer::arm_timer_irq_from_dt(hardware).unwrap_or(FALLBACK_TIMER_IRQ)
}

/// Fires the timer interrupt when `now_ns()` reaches `deadline_ns`.
pub fn event_arm(deadline_ns: u64) {
    // The comparator is absolute, on the same counter `now_ns` reads.
    set_cntp_cval(ns_to_ticks(deadline_ns, get_cntfrq() as u64));
    set_cntp_ctl(CTL_ENABLE);
}

/// Stops the timer; the interrupt line deasserts.
pub fn event_disarm() {
    set_cntp_ctl(0);
}
