//! ACPI for the kernel, powered by Intel ACPICA.


mod error;

pub use error::AcpiError;

#[cfg(target_arch = "x86_64")]
mod madt;
#[cfg(target_arch = "x86_64")]
mod namespace;
#[cfg(target_arch = "x86_64")]
mod raw;
#[cfg(target_arch = "x86_64")]
mod resource;
#[cfg(target_arch = "x86_64")]
mod sleep;
#[cfg(target_arch = "x86_64")]
mod subsystem;
#[cfg(target_arch = "x86_64")]
mod table;
#[cfg(target_arch = "x86_64")]
pub mod ec;
#[cfg(target_arch = "x86_64")]
pub mod events;
#[cfg(target_arch = "x86_64")]
pub mod object;

#[cfg(target_arch = "x86_64")]
pub use madt::{Madt, MadtEntries, MadtEntry};
#[cfg(target_arch = "x86_64")]
pub use namespace::{Device, DeviceInfo, Namespace};
#[cfg(target_arch = "x86_64")]
pub use resource::{AddressSpace, Resource};
#[cfg(target_arch = "x86_64")]
pub use sleep::SleepState;
#[cfg(target_arch = "x86_64")]
pub use subsystem::{Acpi, AcpiLevel};

use core::fmt;

use crate::services::addr::PhysAddr;

/// The ACPI description of a machine, as the bootloader found it: where the
/// RSDP is, and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcpiRoot {
    rsdp: PhysAddr,
}

impl AcpiRoot {
    /// Wraps the physical address of the RSDP.
    pub const fn new(rsdp: PhysAddr) -> Self {
        Self { rsdp }
    }

    /// Physical address of the RSDP.
    pub const fn rsdp(&self) -> PhysAddr {
        self.rsdp
    }

    /// Starts ACPICA: loads the static tables and the AML namespace.
    pub fn init(&self) -> Result<(), AcpiError> {
        #[cfg(target_arch = "x86_64")]
        {
            subsystem::Acpi::start(self.rsdp)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Err(AcpiError::Unsupported)
        }
    }

    /// Switches the hardware into ACPI mode: events, the SCI handler, and the
    /// device/region initialization methods (`_REG`, `_STA`, `_INI`).
    pub fn enable(&self) -> Result<(), AcpiError> {
        #[cfg(target_arch = "x86_64")]
        {
            self.acpi().ok_or(AcpiError::NotReady)?.enable()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Err(AcpiError::Unsupported)
        }
    }

    /// ACPICA itself, if [`init`](Self::init) worked.
    #[cfg(target_arch = "x86_64")]
    pub fn acpi(&self) -> Option<&'static Acpi> {
        Acpi::get()
    }
}

impl fmt::Display for AcpiRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(target_arch = "x86_64")]
        if let Some(acpi) = self.acpi() {
            return write!(f, "ACPI ({})", acpi.level());
        }
        write!(f, "ACPI")
    }
}
