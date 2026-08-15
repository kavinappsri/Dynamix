//! Minimal, identity-mapped MMIO access.

/// A base address for a memory-mapped device.
///
/// The kernel is currently responsible for ensuring that physical device
/// addresses are identity mapped before a driver uses them.
#[derive(Clone, Copy)]
pub struct Mmio {
    base: usize,
}

impl Mmio {
    /// Creates an MMIO block at `base`.
    ///
    /// # Safety
    /// `base` must identify a mapped MMIO region for the lifetime of this value.
    /// The caller must also ensure the region belongs to the intended device.
    pub const unsafe fn new(base: usize) -> Self {
        Self { base }
    }

    /// Reads an 8-bit device register at `offset`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `base + offset` overflows `usize`.
    #[inline(always)]
    pub fn read8(self, offset: usize) -> u8 {
        // SAFETY: `Mmio` is created only for a mapped device base; callers use
        // register offsets defined by that device's driver.
        unsafe { ((self.base + offset) as *const u8).read_volatile() }
    }

    /// Reads a 32-bit device register at `offset`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `base + offset` overflows `usize`.
    #[inline(always)]
    pub fn read32(self, offset: usize) -> u32 {
        // SAFETY: `Mmio` is created only for a mapped device base; callers use
        // register offsets defined by that device's driver.
        unsafe { ((self.base + offset) as *const u32).read_volatile() }
    }

    /// Writes a 32-bit device register at `offset`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `base + offset` overflows `usize`.
    #[inline(always)]
    pub fn write32(self, offset: usize, value: u32) {
        // SAFETY: see `read32`.
        unsafe { ((self.base + offset) as *mut u32).write_volatile(value) }
    }

    /// Writes a 16-bit device register at `offset`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `base + offset` overflows `usize`.
    #[inline(always)]
    pub fn write16(self, offset: usize, value: u16) {
        // SAFETY: see `read8`.
        unsafe { ((self.base + offset) as *mut u16).write_volatile(value) }
    }

    /// Writes a 64-bit device register at `offset`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `base + offset` overflows `usize`.
    #[inline(always)]
    pub fn write64(self, offset: usize, value: u64) {
        // SAFETY: see `read8`.
        unsafe { ((self.base + offset) as *mut u64).write_volatile(value) }
    }
}
