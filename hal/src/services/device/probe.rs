//! Matching devices to drivers and running their probes.

use alloc::vec::Vec;

use super::{Device, Origin, State};
use crate::driver_traits::driver::{self, DriverInfo, Match, ProbeError};
use crate::services::klog::{self, Level, Source};

/// What [`super::init`] managed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Devices the firmware described.
    pub devices: usize,
    /// Devices a driver took.
    pub bound: usize,
    /// Devices a driver matched but could not start (including ones still
    /// waiting for a dependency at the end).
    pub failed: usize,
}

/// How well `driver` fits `device`: the position of the best match in the
/// device's id list, so the most specific id wins. `None` if it does not fit.
fn score(device: &Device, driver: &DriverInfo) -> Option<usize> {
    driver
        .matches
        .iter()
        .filter_map(|wanted| {
            let id = match (wanted, device.origin()) {
                (Match::Fdt(id), Origin::Fdt(_)) => *id,
                #[cfg(target_arch = "x86_64")]
                (Match::Acpi(id), Origin::Acpi(_)) => *id,
                #[cfg(target_arch = "x86_64")]
                (Match::AcpiTable(id), Origin::AcpiTable) => *id,
                (Match::Spawned(id), Origin::Spawned { .. }) => *id,
                _ => return None,
            };
            device.ids().iter().position(|have| have == id)
        })
        .min()
}

/// Tries the drivers that fit `device`, best fit first. Returns whether it
/// ended up bound.
fn attempt(device: &'static Device, table: &'static [DriverInfo]) -> bool {
    let mut candidates: Vec<(usize, &'static DriverInfo)> = table
        .iter()
        .filter_map(|driver| score(device, driver).map(|score| (score, driver)))
        .collect();
    // Stable, so equally good drivers are tried in table order.
    candidates.sort_by_key(|(score, _)| *score);

    if candidates.is_empty() {
        device.set_state(State::NoDriver);
        return false;
    }

    let mut last_error = None;
    for (_, driver) in candidates {
        match (driver.probe)(device) {
            Ok(()) => {
                device.set_state(State::Bound(driver.name));
                klog::log(Source::Kernel, Level::Info, format_args!("{}: bound to {}", device.name(), driver.name));
                return true;
            }
            Err(ProbeError::Defer) => {
                device.set_state(State::Deferred(driver.name));
                return false;
            }
            Err(error) => {
                klog::log(
                    Source::Kernel,
                    Level::Warn,
                    format_args!("{}: {} did not start: {}", device.name(), driver.name, error),
                );
                last_error = Some((driver.name, error));
            }
        }
    }

    if let Some((name, error)) = last_error {
        device.set_state(State::Failed(name, error));
    }
    false
}

/// Probes every device, repeating until a whole pass binds nothing new, so a
/// driver that deferred gets another go once its dependency has appeared.
pub(super) fn probe_all() -> Report {
    let table = driver::table();

    loop {
        let mut progress = false;
        for device in super::devices() {
            if matches!(device.state(), State::Pending | State::Deferred(_)) {
                progress |= attempt(device, table);
            }
        }
        if !progress {
            break;
        }
    }

    let mut report = Report::default();
    for device in super::devices() {
        report.devices += 1;
        match device.state() {
            State::Bound(_) => report.bound += 1,
            State::Failed(..) => report.failed += 1,
            State::Deferred(driver) => {
                report.failed += 1;
                klog::log(
                    Source::Kernel,
                    Level::Warn,
                    format_args!("{}: {} is still waiting for a dependency", device.name(), driver),
                );
            }
            State::Pending | State::NoDriver => {}
        }
    }
    report
}
