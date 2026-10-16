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

pub trait DeviceQuery {
    /// Finds the `(base, size)` MMIO window of the first device matching
    /// `compatible`.
    fn find_compatible_address(&self, compatible: &str) -> Option<(usize, usize)>;

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