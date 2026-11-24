//! Time for x86_64: busy-waits and the clock on the TSC, and the one-shot
//! event timer (the local APIC timer) behind the interrupt-driven timers.


use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::services::interrupts::Irq;
use crate::services::timer::ns_to_ticks;
use crate::x86_64::port::{inb, outb};
use crate::x86_64::x64_apic;

const PIT_HZ: u64 = 1_193_182;
/// PIT reload value for the calibration window: 23864 / 1193182 Hz ~= 20 ms.
const CALIBRATION_COUNT: u16 = 23_864;
/// Polls of port 0x61 before giving up on the PIT.
const POLL_LIMIT: u32 = 5_000_000;
/// Used when the PIT never answers; deliberately low so delays err long.
const FALLBACK_TSC_HZ: u64 = 1_000_000_000;

const PIT_COMMAND: u16 = 0x43;
const PIT_CHANNEL2: u16 = 0x42;
const PORT_B: u16 = 0x61;
const PORT_B_GATE2: u8 = 1 << 0;
const PORT_B_SPEAKER: u8 = 1 << 1;
const PORT_B_OUT2: u8 = 1 << 5;

static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// Ticks per second of the local APIC timer (after its divider); 0 until calibrated.
static LAPIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);

/// How long the local APIC timer is measured against the TSC.
const LAPIC_CALIBRATION_US: u64 = 10_000;

#[inline]
fn rdtsc() -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: `lfence` keeps the read from floating above earlier instructions;
    // `rdtsc` only reads the counter.
    unsafe {
        asm!("lfence", "rdtsc", out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    ((high as u64) << 32) | low as u64
}

/// Measures TSC ticks across one PIT one-shot. `None` if the PIT never fires.
fn calibrate() -> Option<u64> {
    // SAFETY: legacy PIT / port-B registers, owned by this module.
    unsafe {
        let saved = inb(PORT_B);

        // Gate off + speaker off, program channel 2: binary, mode 0, lo/hi byte.
        outb(PORT_B, saved & !(PORT_B_GATE2 | PORT_B_SPEAKER));
        outb(PIT_COMMAND, 0b10110000);
        outb(PIT_CHANNEL2, (CALIBRATION_COUNT & 0xFF) as u8);
        outb(PIT_CHANNEL2, (CALIBRATION_COUNT >> 8) as u8);

        // Raising the gate starts the countdown.
        outb(PORT_B, (saved & !PORT_B_SPEAKER) | PORT_B_GATE2);
        let start = rdtsc();

        let mut polls = 0u32;
        while inb(PORT_B) & PORT_B_OUT2 == 0 {
            polls += 1;
            if polls > POLL_LIMIT {
                outb(PORT_B, saved);
                return None;
            }
        }
        let end = rdtsc();
        outb(PORT_B, saved);

        let ticks = end.wrapping_sub(start);
        // ticks / (COUNT / PIT_HZ) seconds, kept in integers.
        Some(ticks * PIT_HZ / CALIBRATION_COUNT as u64)
    }
}

fn tsc_hz() -> u64 {
    match TSC_HZ.load(Ordering::Relaxed) {
        0 => {
            // Calibration owns the PIT for ~20 ms. Keep interrupts masked so a
            // handler that reads the clock cannot start a second calibration
            // on top of this one.
            let state = crate::services::cpu::irq_save();
            let hz = match TSC_HZ.load(Ordering::Relaxed) {
                0 => {
                    let hz = calibrate().filter(|&hz| hz != 0).unwrap_or(FALLBACK_TSC_HZ);
                    TSC_HZ.store(hz, Ordering::Relaxed);
                    hz
                }
                hz => hz,
            };
            crate::services::cpu::irq_restore(state);
            hz
        }
        hz => hz,
    }
}

/// Busy-waits for at least `ms` milliseconds.
pub fn delay_ms(ms: u64) {
    let ticks = tsc_hz().saturating_mul(ms) / 1000;
    let start = rdtsc();
    while rdtsc().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// Busy-waits for at least `us` microseconds.
pub fn delay_us(us: u64) {
    let ticks = tsc_hz().saturating_mul(us) / 1_000_000;
    let start = rdtsc();
    while rdtsc().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// Nanoseconds of TSC time since the TSC started counting.
pub fn now_ns() -> u64 {
    crate::services::timer::ticks_to_ns(rdtsc(), tsc_hz())
}

/// Calibrates the local APIC timer against the TSC and leaves it stopped.
/// Returns the interrupt it raises, or `None` if the APIC is not running or
/// its timer does not count.
pub fn event_init() -> Option<Irq> {
    let state = crate::services::cpu::irq_save();

    // Counting but masked, so the measurement cannot raise an interrupt.
    x64_apic::lapic_timer_program(u32::MAX, true);
    delay_us(LAPIC_CALIBRATION_US);
    let counted = (u32::MAX - x64_apic::lapic_timer_remaining()) as u64;
    x64_apic::lapic_timer_stop();

    crate::services::cpu::irq_restore(state);

    if counted == 0 {
        return None;
    }
    LAPIC_TIMER_HZ.store(counted * 1_000_000 / LAPIC_CALIBRATION_US, Ordering::Relaxed);
    Some(x64_apic::LOCAL_TIMER_IRQ)
}

/// Fires the timer interrupt when `now_ns()` reaches `deadline_ns`.
///
/// The counter is 32 bits wide, so a far-away deadline is reached in steps:
/// the interrupt arrives early, finds nothing expired, and the timer service
/// arms it again for the remainder.
pub fn event_arm(deadline_ns: u64) {
    let hz = LAPIC_TIMER_HZ.load(Ordering::Relaxed);
    let remaining_ns = deadline_ns.saturating_sub(now_ns());
    // At least one tick: zero would mean "stopped".
    let ticks = ns_to_ticks(remaining_ns, hz).clamp(1, u32::MAX as u64) as u32;
    x64_apic::lapic_timer_program(ticks, false);
}

/// Stops the timer.
pub fn event_disarm() {
    x64_apic::lapic_timer_stop();
}

/// The local APIC's timer as the event timer.
pub struct LapicTimer;

impl crate::driver_traits::event_timer::EventTimer for LapicTimer {
    fn init(&self) -> Option<Irq> {
        event_init()
    }

    fn arm(&self, deadline_ns: u64) {
        event_arm(deadline_ns);
    }

    fn disarm(&self) {
        event_disarm();
    }
}
