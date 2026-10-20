//! busy waits for x86_64.


use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::x86_64::port::{inb, outb};

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
        outb(PIT_COMMAND, 0b10_11_000_0);
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
            let hz = calibrate().filter(|&hz| hz != 0).unwrap_or(FALLBACK_TSC_HZ);
            TSC_HZ.store(hz, Ordering::Relaxed);
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