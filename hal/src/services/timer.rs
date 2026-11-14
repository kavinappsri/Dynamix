//! Provides time related functions

pub fn ticks_to_ns(ticks: u64, hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    let secs = ticks / hz;
    let rem = ticks % hz;
    secs.wrapping_mul(1_000_000_000)
        .wrapping_add(rem * 1_000_000_000 / hz)
}

pub fn delay_ms(ms: u64) {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_timer::delay_ms(ms);

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_timer::delay_ms(ms);

    #[cfg(target_arch = "x86_64")]
    {
        crate::x86_64::x64_timer::delay_ms(ms);
    }
}

/// Busy-waits for at least `us` microseconds.
pub fn delay_us(us: u64) {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_timer::delay_us(us);

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_timer::delay_us(us);

    #[cfg(target_arch = "x86_64")]
    crate::x86_64::x64_timer::delay_us(us);
}

/// Nanoseconds since the platform's free-running counter started.
pub fn now_ns() -> u64 {
    #[cfg(target_arch = "aarch64")]
    return crate::aarch64::aarch64_timer::now_ns();

    #[cfg(target_arch = "arm")]
    return crate::armv7::armv7_timer::now_ns();

    #[cfg(target_arch = "x86_64")]
    return crate::x86_64::x64_timer::now_ns();
}
