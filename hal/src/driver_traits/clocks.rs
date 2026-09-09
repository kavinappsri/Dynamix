//! Generic clock controller interface and driver discovery

use crate::define_probe;
use crate::services::sync::StaticCell;
use crate::services::dtb::Dtb;

static ACTIVE_CLOCK: StaticCell<&'static dyn ClockController> = StaticCell::uninit();

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

pub fn active_clock() -> Option<&'static dyn ClockController> {
    ACTIVE_CLOCK.get().copied()
}

define_probe!("__start_clock_drivers", "__stop_clock_drivers", probe_cclocks, ClockController, |driver| { ACTIVE_CLOCK.get_or_init(|| driver) } );

