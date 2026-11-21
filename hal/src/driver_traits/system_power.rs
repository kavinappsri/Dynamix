//! Whatever can turn the machine off: PSCI on ARM, ACPI sleep state S5 on x86.

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::power::PowerError;
use crate::services::registry::{Class, Entry, Iter};

static SYSTEM_POWER: Class<dyn SystemPower> = Class::new();

pub trait SystemPower: Sync {
    /// Turns the system off. Success never returns, so anything that comes
    /// back is a failure.
    fn power_off(&self) -> Result<(), PowerError>;
}

/// The first system power controller that was bound, if any.
pub fn primary_system_power() -> Option<&'static dyn SystemPower> {
    SYSTEM_POWER.primary()
}

/// Every system power controller that was bound, in the order they were bound.
pub fn system_power_controllers() -> Iter<Entry<dyn SystemPower>> {
    SYSTEM_POWER.iter()
}

/// Called by a driver's probe to hand over the controller it started on `device`.
pub fn register(device: &'static Device, power: impl SystemPower + 'static) -> &'static dyn SystemPower {
    let power: &'static dyn SystemPower = Box::leak(Box::new(power));
    SYSTEM_POWER.register(device, power);
    power
}
