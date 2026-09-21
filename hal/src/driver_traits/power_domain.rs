//! On chip power controller interface
//!
//! This trait is for an SoC's internal power supply. the external power
//! should be handled by another trait, which communicates to the PMIC

use crate::define_probe;
use crate::services::sync::StaticCell;
use crate::services::dtb::Dtb;

static ACTIVE_POWER_DOMAIN_CONTROLLER: StaticCell<&'static dyn PowerDomainController> = StaticCell::uninit();

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

pub fn active_power_domain_controller() -> Option<&'static dyn PowerDomainController> {
    ACTIVE_POWER_DOMAIN_CONTROLLER.get().copied()
}

define_probe!("__start_power_domain_drivers", "__stop_power_domain_drivers", probe_power_domains, PowerDomainController, |driver| { ACTIVE_POWER_DOMAIN_CONTROLLER.get_or_init(|| driver) } );