//! On chip power controller interface
//!
//! This trait is for internal power supply. the external power
//! should be handled by another trait, which communicates to the PMIC

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::registry::{Class, Entry, Iter};

static POWER_DOMAINS: Class<dyn PowerDomainController> = Class::new();

/// Power domain errors
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerDomainError {
    /// The requested domain id isn't recognized by this controller
    UnknownDomain,
    /// The controller gave up waiting for a status/ack bit to reflect the
    /// requested change
    Timeout,
}

/// SoC internal power controller
pub trait PowerDomainController: Sync {
    /// Powers a domain on, blocking until the controller confirms it
    fn power_on(&self, domain: u32) -> Result<(), PowerDomainError>;
    /// Powers a domain off, blocking until the controller confirms it
    fn power_off(&self, domain: u32) -> Result<(), PowerDomainError>;
    /// Returns whether `domain` is currently powered on
    fn is_powered_on(&self, domain: u32) -> Result<bool, PowerDomainError>;
}

/// The first power domain controller that was bound, if any.
pub fn active_power_domain_controller() -> Option<&'static dyn PowerDomainController> {
    POWER_DOMAINS.primary()
}

/// Every power domain controller that was bound, in the order they were bound.
pub fn power_domain_controllers() -> Iter<Entry<dyn PowerDomainController>> {
    POWER_DOMAINS.iter()
}

/// Called by a driver's probe to hand over the controller it started on `device`.
pub fn register(
    device: &'static Device,
    controller: impl PowerDomainController + 'static,
) -> &'static dyn PowerDomainController {
    let controller: &'static dyn PowerDomainController = Box::leak(Box::new(controller));
    POWER_DOMAINS.register(device, controller);
    controller
}
