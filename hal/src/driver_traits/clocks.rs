//! Generic clock controller interface

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::registry::{Class, Entry, Iter};

static CLOCKS: Class<dyn ClockController> = Class::new();

/// Clock errors
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockError {
    UnknownClock,
    UnreachableRate,
    ParentUnavailable,
}

/// An SoC clock / reset controller
pub trait ClockController : Sync {
    fn enable(&self, clock: u32) -> Result<(), ClockError>;
    fn disable(&self, clock: u32) -> Result<(), ClockError>;
    /// Selects a divider so that `clock` runs as close to `hz` as
    /// the hardware allows it. Returns the rate actually reached
    /// on success
    fn set_rate(&self, clock: u32, hz: u32) -> Result<u32, ClockError>;
    fn get_rate(&self, clock: u32) -> Result<u32, ClockError>;

}

/// The first clock controller that was bound, if any. Drivers that need one
/// and find none return [`ProbeError::Defer`](crate::driver_traits::driver::ProbeError::Defer).
pub fn active_clock() -> Option<&'static dyn ClockController> {
    CLOCKS.primary()
}

/// Every clock controller that was bound, in the order they were bound.
pub fn clock_controllers() -> Iter<Entry<dyn ClockController>> {
    CLOCKS.iter()
}

/// Called by a driver's probe to hand over the clock controller it started on `device`.
pub fn register(device: &'static Device, controller: impl ClockController + 'static) -> &'static dyn ClockController {
    let controller: &'static dyn ClockController = Box::leak(Box::new(controller));
    CLOCKS.register(device, controller);
    controller
}
