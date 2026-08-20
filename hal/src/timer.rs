//! Provides time related functions
pub fn delay_ms(ms: u64) {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_timer::delay_ms(ms);

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_timer::delay_ms(ms);
}