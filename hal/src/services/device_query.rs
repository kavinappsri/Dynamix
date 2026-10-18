//! Hardware discovery abstraction layer

use crate::services::dtb::Dtb;
use crate::services::acpi::{AcpiQuery};

/// A single contiguous region of usable RAM, as reported by firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    pub base: usize,
    pub size: usize,
}

pub const MAX_MEMORY_REGIONS: usize = 32;

/// Where a device's registers live
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeviceResource {
    /// A memory-mapped register window: physical `base` and `size` in bytes.
    Mmio { base: usize, size: usize },
    /// A window of `count` consecutive x86 I/O ports starting at `base`.
    Io { base: u16, count: u16 },
}

impl DeviceResource {
    /// The base address, if this is a memory-mapped window.
    pub const fn mmio_base(self) -> Option<usize> {
        match self {
            DeviceResource::Mmio { base, .. } => Some(base),
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

pub trait DeviceQuery {
    /// Finds the `(base, size)` MMIO window of the first device matching
    /// `compatible`.
    fn find_compatible_address(&self, compatible: &str) -> Option<(usize, usize)>;

    /// Finds the register resource of the first device matching `compatible`.
    fn find_compatible_resource(&self, compatible: &str) -> Option<DeviceResource> {
        self.find_compatible_address(compatible)
            .map(|(base, size)| DeviceResource::Mmio { base, size })
    }

    /// Returns the usable RAM region(s) reported by firmware, most useful
    /// region first.
    fn memory_regions(&self, out: &mut [MemoryRegion]) -> usize;

    fn as_dtb(&self) -> Option<&Dtb<'_>> {
        None
    }

    fn as_acpi(&self) -> Option<&AcpiQuery> {
        None
    }
}