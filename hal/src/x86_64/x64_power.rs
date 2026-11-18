//! ACPI power-off (sleep state S5) for x86_64, through ACPICA.


use core::sync::atomic::{AtomicBool, Ordering};

use crate::services::acpi::{Acpi, AcpiError, AcpiRoot, SleepState};
use crate::services::power::PowerError;

static READY: AtomicBool = AtomicBool::new(false);

/// Checks that firmware offers S5 and arms `system_off`.
///
/// Needs ACPICA's namespace, so it runs after `hardware.init()`.
pub fn init(root: &AcpiRoot) -> Result<(), PowerError> {
    if READY.load(Ordering::Acquire) {
        return Err(PowerError::Unsupported);
    }

    let acpi = root.acpi().ok_or(PowerError::NotDescribed)?;
    match acpi.sleep_type_data(SleepState::S5) {
        Ok(_) => {}
        // No namespace, or no `_S5`: firmware does not describe soft-off.
        Err(AcpiError::NotReady | AcpiError::NotFound) => return Err(PowerError::NotDescribed),
        Err(_) => return Err(PowerError::Unsupported),
    }

    READY.store(true, Ordering::Release);
    Ok(())
}

/// Requests S5 (soft off). Returns if power-off was not set up or did not take effect.
pub fn system_off() {
    if !READY.load(Ordering::Acquire) {
        return;
    }
    let Some(acpi) = Acpi::get() else { return };
    // Success never returns, so anything that comes back is a failure.
    let _ = acpi.enter_sleep(SleepState::S5);
}
