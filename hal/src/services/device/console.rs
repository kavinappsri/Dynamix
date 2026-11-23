//! Choosing which serial port is the console.
//!
//! Firmware knows which port it used for its own console, and that is the
//! one a person has a cable on. A device tree says so in `/chosen/stdout-path`,
//! ACPI in the SPCR table. Without either, the first port that bound is used.

use crate::driver_traits::serial::{self, serials};
use crate::services::boot_info::HardwareDescription;

/// Makes the serial port firmware names the console, if it names one and that
/// port bound.
pub(super) fn select(hardware: &HardwareDescription<'static>) {
    let chosen = match hardware {
        HardwareDescription::Fdt(dtb) => dtb.stdout_node().and_then(|node| {
            serials().find(|entry| entry.device.fdt().is_some_and(|candidate| candidate.offset() == node.offset()))
        }),
        #[cfg(target_arch = "x86_64")]
        HardwareDescription::Acpi(root) => acpi::choice(root),
        #[cfg(not(target_arch = "x86_64"))]
        HardwareDescription::Acpi(_) => None,
    };

    if let Some(entry) = chosen {
        serial::prefer(entry);
    }
}

#[cfg(target_arch = "x86_64")]
mod acpi {
    use super::serials;
    use crate::driver_traits::serial::Serial;
    use crate::services::acpi::{AcpiRoot, SerialAddress};
    use crate::services::device::{Device, Resource};
    use crate::services::registry::Entry;

    /// The serial port at the address the SPCR gives.
    pub(super) fn choice(root: &AcpiRoot) -> Option<&'static Entry<dyn Serial>> {
        let address = root.acpi()?.console_serial().ok()?;
        serials().find(|entry| uses_address(entry.device, address))
    }

    fn uses_address(device: &Device, address: SerialAddress) -> bool {
        let Ok(resources) = device.resources() else { return false };
        resources.iter().any(|resource| match (resource, address) {
            (Resource::Io(ports), SerialAddress::Io(base)) => ports.base() == base,
            (Resource::Mem(range), SerialAddress::Memory(base)) => range.start().as_usize() as u64 == base,
            _ => false,
        })
    }
}
