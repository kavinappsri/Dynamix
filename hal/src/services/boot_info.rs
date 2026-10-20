//! Boot Info struct passed into `rust_main` at start
//!
//! I'm making this to make the kernel completely independent of the fdt
//! thus eliminating the architecture issue

use crate::services::acpi::AcpiQuery;
use crate::services::addr::PhysRange;
use crate::services::device_query::{DeviceQuery, DeviceResource};
use crate::services::dtb::Dtb;
use crate::services::framebuffer::FirmwareFramebuffer;

/// Most usable-RAM regions a [`RegionList`] can hold.
pub const MAX_MEMORY_REGIONS: usize = 32;

#[derive(Clone, Copy)]
pub struct RegionList {
    regions: [PhysRange; MAX_MEMORY_REGIONS],
    count: usize,
}

impl RegionList {
    pub const fn empty() -> Self {
        Self {
            regions: [PhysRange::EMPTY; MAX_MEMORY_REGIONS],
            count: 0,
        }
    }

    pub fn from_fill(f: impl FnOnce(&mut [PhysRange]) -> usize) -> Self {
        let mut regions = [PhysRange::EMPTY; MAX_MEMORY_REGIONS];
        let written = f(&mut regions);
        let count = written.min(MAX_MEMORY_REGIONS);
        Self { regions, count }
    }

    pub fn as_slice(&self) -> &[PhysRange] {
        &self.regions[..self.count]
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// The firmware-provided hardware description, whichever form the platform
/// handed the kernel.
pub enum HardwareDescription<'a> {
    /// A Flattened Device Tree blob
    Fdt(Dtb<'a>),
    /// The ACPI tables
    Acpi(AcpiQuery),
}

impl<'a> DeviceQuery for HardwareDescription<'a> {
    fn find_compatible_resource(&self, compatible: &str) -> Option<DeviceResource> {
        match self {
            HardwareDescription::Fdt(dtb) => dtb.find_compatible_resource(compatible),
            HardwareDescription::Acpi(acpi) => acpi.find_compatible_resource(compatible),
        }
    }

    fn as_dtb(&self) -> Option<&Dtb<'_>> {
        match self {
            HardwareDescription::Fdt(dtb) => Some(dtb),
            HardwareDescription::Acpi(_) => None,
        }
    }

    fn as_acpi(&self) -> Option<&AcpiQuery> {
        match self {
            HardwareDescription::Fdt(_) => None,
            HardwareDescription::Acpi(acpi) => Some(acpi),
        }
    }
}

/// Basic services and hardware description handed to `rust_main` at kernel
/// entry, regardless of arch or bootloader.
#[repr(C)]
pub struct BootInfo<'a> {
    /// The firmware/bootloader's hardware description
    pub hardware: Option<HardwareDescription<'a>>,

    /// Usable RAM regions discovered during boot.
    pub memory_regions: RegionList,

    /// The physical range `hardware` itself occupies, if it needs protecting
    /// from the PMM handing it out as free RAM.
    pub hardware_reserved_range: Option<PhysRange>,

    /// A framebuffer set up by firmware before the kernel took over, if any.
    pub framebuffer: Option<FirmwareFramebuffer>,
}

impl<'a> BootInfo<'a> {
    /// Builds an empty `BootInfo` with every field absent.
    pub const fn empty() -> Self {
        Self {
            hardware: None,
            memory_regions: RegionList::empty(),
            hardware_reserved_range: None,
            framebuffer: None,
        }
    }
}
