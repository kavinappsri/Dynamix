//! PSCI, the firmware interface ARM systems use to power off.
//!
//! The node's `method` says whether calls go through `hvc` or `smc`.

#[cfg(target_arch = "aarch64")]
use crate::aarch64::aarch64_power::{self as arch, PsciMethod};
#[cfg(target_arch = "arm")]
use crate::armv7::armv7_power::{self as arch, PsciMethod};

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::system_power::{self, SystemPower};
use crate::services::device::Device;
use crate::services::power::PowerError;

struct Psci {
    method: PsciMethod,
}

impl SystemPower for Psci {
    fn power_off(&self) -> Result<(), PowerError> {
        arch::system_off(self.method)
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let method = dev
        .fdt()
        .and_then(|node| node.property("method")?.as_str())
        .and_then(PsciMethod::from_property)
        .ok_or(ProbeError::Firmware)?;

    system_power::register(dev, Psci { method });
    Ok(())
}

driver! {
    name: "psci",
    matches: [
        Match::Fdt("arm,psci-1.0"),
        Match::Fdt("arm,psci-0.2"),
        Match::Fdt("arm,psci"),
    ],
    probe: probe,
}
