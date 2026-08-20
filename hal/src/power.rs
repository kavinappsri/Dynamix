//! Provides Power related functions

/// Sets up the power system
pub fn init(method: &str) {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_power::init(method);

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_power::init(method);
}

/// Turns off the system
pub fn system_off() {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_power::system_off();

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_power::system_off();
}