//! Generic clock controller interface and driver discovery

use crate::DeviceTree;
use crate::services::sync::StaticCell;

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

/// A clock driver
pub struct ClockDriver {
    pub name: &'static str,
    pub compatible: &'static str,
    pub init: fn(base: usize) -> &'static dyn ClockController,
}

/// Registers a clock controller driver in the discovery table
#[macro_export]
macro_rules! register_clock_driver {
    ($ident:ident, $compat:expr, $init_fn:path) => {
        #[used]
        #[unsafe(link_section = ".drivers.clocks")]
        static $ident: $crate::driver_traits::clocks::ClockDriver = $crate::driver_traits::clocks::ClockDriver {
            name: stringify!($ident),
            compatible: $compat,
            init: $init_fn,
        };
    };
}

unsafe extern "C" {
    static __start_clock_drivers: u8;
    static __stop_clock_drivers: u8;
}

fn clock_drivers() -> &'static [ClockDriver] {
    unsafe {
        let start = core::ptr::addr_of!(__start_clock_drivers) as *const ClockDriver;
        let stop = core::ptr::addr_of!(__stop_clock_drivers) as *const ClockDriver;
        let count = (stop as usize - start as usize) / size_of::<ClockDriver>();
        core::slice::from_raw_parts(start, count)
    }
}

/// Failure to locate a clock controller driver for the supplied device tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockProbeError {
    NoCompatibleClockDriver,
    MappingFailed,
}

/// Finds and initializes the first registered clock controller present in
/// `tree`, mapping its registers as device memory.
pub fn probe_clocks(tree: &impl DeviceTree) -> Result<&'static dyn ClockController, ClockProbeError> {
    for driver in clock_drivers() {
        if let Some((base, size)) = tree.compatible_address(driver.compatible) {
            crate::services::mmu::map_device(base, size).map_err(|_| ClockProbeError::MappingFailed)?;
            return Ok(*ACTIVE_CLOCK.get_or_init(|| (driver.init)(base)));
        }
    }
    Err(ClockProbeError::NoCompatibleClockDriver)
}

pub fn active_clock() -> Option<&'static dyn ClockController> {
    ACTIVE_CLOCK.get().copied()
}

