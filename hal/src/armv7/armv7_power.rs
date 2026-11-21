//! PSCI calls for armv7.

use core::arch::asm;

use crate::services::power::PowerError;

/// The conduit PSCI calls travel over, as the device tree's `method` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsciMethod {
    Hvc,
    Smc,
}

impl PsciMethod {
    pub fn from_property(method: &str) -> Option<Self> {
        match method {
            "hvc" => Some(PsciMethod::Hvc),
            "smc" => Some(PsciMethod::Smc),
            _ => None,
        }
    }
}

/// PSCI `SYSTEM_OFF`. Returns only if the call did not take effect.
pub fn system_off(method: PsciMethod) -> Result<(), PowerError> {
    // SAFETY: a PSCI call with function id SYSTEM_OFF, which takes no arguments.
    unsafe {
        match method {
            PsciMethod::Hvc => asm!(
            ".arch_extension virt",
            "hvc #0",
            in("r0") 0x8400_0008u32,
            options(nostack, nomem)
            ),
            PsciMethod::Smc => asm!(
            ".arch_extension sec",
            "smc #0",
            in("r0") 0x8400_0008u32,
            options(nostack, nomem)
            ),
        }
    }
    Err(PowerError::Failed)
}
