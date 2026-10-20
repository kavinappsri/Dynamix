//! Hardware discovery abstraction layer

use crate::services::acpi::AcpiQuery;
use crate::services::addr::PhysRange;
use crate::services::dtb::Dtb;

/// Where a device's registers live
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeviceResource {
    /// A memory-mapped register window.
    Mmio { range: PhysRange },
    /// A window of `count` consecutive x86 I/O ports starting at `base`.
    Io { base: u16, count: u16 },
}

impl DeviceResource {
    /// The base address, if this is a memory-mapped window.
    pub const fn mmio_base(self) -> Option<usize> {
        match self {
            DeviceResource::Mmio { range } => Some(range.start().as_usize()),
            DeviceResource::Io { .. } => None,
        }
    }

    /// The first port, if this is an I/O port window.
    pub const fn io_base(self) -> Option<u16> {
        match self {
            DeviceResource::Io { base, .. } => Some(base),
            DeviceResource::Mmio { .. } => None,
        }
    }
}

/// Finds devices described by the firmware, whatever form the description takes.
pub trait DeviceQuery {
    /// Finds the register resource of the first device matching `compatible`
    /// (an FDT `compatible` string, or an ACPI `_HID`).
    fn find_compatible_resource(&self, compatible: &str) -> Option<DeviceResource>;

    fn as_dtb(&self) -> Option<&Dtb<'_>> {
        None
    }

    fn as_acpi(&self) -> Option<&AcpiQuery> {
        None
    }
}
