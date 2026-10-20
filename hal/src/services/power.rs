//! Provides Power related functions

use crate::services::boot_info::HardwareDescription;

/// Why the power system could not be set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerError {
    /// The platform description (device tree / ACPI tables) has no usable power interface.
    NotDescribed,
    /// The description exists but names something this kernel cannot drive.
    Unsupported,
    /// The platform description is present but malformed.
    BadHardwareDescription,
    /// Power control was used before `init` succeeded.
    NotInitialized,
}

/// Sets up the power system from the platform description:
/// PSCI node on aarch64/armv7, FADT + DSDT `\_S5` on x86_64.
pub fn init(tree: &HardwareDescription<'_>) -> Result<(), PowerError> {
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    {
        use crate::services::device_query::DeviceQuery;

        let dtb = tree.as_dtb().ok_or(PowerError::NotDescribed)?;
        let psci = dtb.find_compatible("arm,psci").ok_or(PowerError::NotDescribed)?;
        let method = psci.property("method").and_then(|p| p.as_str()).ok_or(PowerError::Unsupported)?;

        #[cfg(target_arch = "aarch64")]
        { crate::aarch64::aarch64_power::init(method) }
        #[cfg(target_arch = "arm")]
        { crate::armv7::armv7_power::init(method) }
    }

    #[cfg(target_arch = "x86_64")]
    {
        use crate::services::device_query::DeviceQuery;

        let acpi = tree.as_acpi().ok_or(PowerError::NotDescribed)?;
        crate::x86_64::x64_power::init(acpi)
    }
}

/// Turns off the system. Returns if power-off is unavailable.
pub fn system_off() {
    #[cfg(target_arch = "aarch64")]
    let _ = crate::aarch64::aarch64_power::system_off();

    #[cfg(target_arch = "arm")]
    let _ = crate::armv7::armv7_power::system_off();

    #[cfg(target_arch = "x86_64")]
    crate::x86_64::x64_power::system_off();
}
