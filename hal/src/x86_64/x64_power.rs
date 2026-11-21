//! ACPI power-off (sleep state S5) for x86_64, through ACPICA.

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::system_power::{self, SystemPower};
use crate::services::acpi::{Acpi, AcpiError, SleepState};
use crate::services::device::Device;
use crate::services::power::PowerError;

struct AcpiPower;

impl SystemPower for AcpiPower {
    fn power_off(&self) -> Result<(), PowerError> {
        let acpi = Acpi::get().ok_or(PowerError::Unsupported)?;
        // Success never returns, so anything that comes back is a failure.
        acpi.enter_sleep(SleepState::S5).map_err(|_| PowerError::Failed)
    }
}

/// The FADT is where firmware says how to power the machine off. It only
/// counts if the namespace also offers `_S5`.
///
/// Needs ACPICA's namespace, which is why this runs after `hardware.init()`.
fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let acpi = Acpi::get().ok_or(ProbeError::Firmware)?;
    match acpi.sleep_type_data(SleepState::S5) {
        Ok(_) => {}
        // No `_S5`: firmware does not describe soft-off.
        Err(AcpiError::NotReady | AcpiError::NotFound) => return Err(ProbeError::NotPresent),
        Err(_) => return Err(ProbeError::Firmware),
    }

    system_power::register(dev, AcpiPower);
    Ok(())
}

driver! {
    name: "acpi-power",
    matches: [Match::AcpiTable("FACP")],
    probe: probe,
}
