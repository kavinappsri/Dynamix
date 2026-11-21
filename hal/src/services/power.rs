//! Powering the machine off.
//!
//! What can do it is a driver: PSCI on ARM, ACPI sleep state S5 on x86. This
//! is the kernel's way to ask the one that bound.

use crate::driver_traits::system_power::primary_system_power;

/// Why a power request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerError {
    /// The controller does not support the request.
    Unsupported,
    /// The request was made and did not take effect.
    Failed,
}

/// Whether anything that can turn the machine off was found.
pub fn is_available() -> bool {
    primary_system_power().is_some()
}

/// Turns off the system. Returns if power-off is unavailable or failed.
pub fn system_off() {
    if let Some(power) = primary_system_power() {
        let _ = power.power_off();
    }
}
