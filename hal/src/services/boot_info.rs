//! Boot Info struct passed into `rust_main` at start
//!
//! I'm making this to make the kernel completely independent of the fdt
//! thus eliminating the architecture issue

use crate::services::device_query::{DeviceQuery, MemoryRegion, MAX_MEMORY_REGIONS};
use crate::services::dtb::Dtb;
#[cfg(target_arch = "x86_64")]
use crate::services::acpi::AcpiQuery;

#[derive(Clone, Copy)]
pub struct RegionList {
    regions: [MemoryRegion; MAX_MEMORY_REGIONS],
    count: usize,
}

impl RegionList {
    pub const fn empty() -> Self {
        Self {
            regions: [MemoryRegion { base: 0, size: 0 }; MAX_MEMORY_REGIONS],
            count: 0,
        }
    }

    pub fn from_fill(f: impl FnOnce(&mut [MemoryRegion]) -> usize) -> Self {
        let mut regions = [MemoryRegion { base: 0, size: 0 }; MAX_MEMORY_REGIONS];
        let written = f(&mut regions);
        let count = written.min(MAX_MEMORY_REGIONS);
        Self { regions, count }
    }

    pub fn as_slice(&self) -> &[MemoryRegion] {
        &self.regions[..self.count]
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// The firmware-provided device description, whichever form the platform
/// handed the bootloader/kernel.
pub enum DeviceTree<'a> {
    /// A Flattened Device Tree blob
    Fdt(Dtb<'a>),
    /// The ACPI tables
    #[cfg(target_arch = "x86_64")]
    Acpi(AcpiQuery),
}

impl<'a> DeviceQuery for DeviceTree<'a> {
    fn find_compatible_address(&self, compatible: &str) -> Option<(usize, usize)> {
        match self {
            DeviceTree::Fdt(dtb) => dtb.find_compatible_address(compatible),
            #[cfg(target_arch = "x86_64")]
            DeviceTree::Acpi(acpi) => acpi.find_compatible_address(compatible),
        }
    }

    fn memory_regions(&self, out: &mut [MemoryRegion]) -> usize {
        match self {
            DeviceTree::Fdt(dtb) => dtb.memory_regions(out),
            #[cfg(target_arch = "x86_64")]
            DeviceTree::Acpi(acpi) => acpi.memory_regions(out),
        }
    }

    fn as_dtb(&self) -> Option<&Dtb<'_>> {
        match self {
            DeviceTree::Fdt(dtb) => Some(dtb),
            #[cfg(target_arch = "x86_64")]
            DeviceTree::Acpi(_) => None,
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn as_acpi(&self) -> Option<&AcpiQuery> {
        match self {
            DeviceTree::Fdt(_) => None,
            DeviceTree::Acpi(acpi) => Some(acpi),
        }
    }
}

/// Byte order of a 32-bit framebuffer pixel, as laid out in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Bytes in memory are `R, G, B, unused`.
    Rgbx8,
    /// Bytes in memory are `B, G, R, unused`.
    Bgrx8,
}

impl PixelFormat {
    /// Encodes a `0x00RRGGBB` color into the 32-bit value this format expects.
    pub const fn encode(self, color: u32) -> u32 {
        let r = (color >> 16) & 0xFF;
        let g = (color >> 8) & 0xFF;
        let b = color & 0xFF;

        match self {
            PixelFormat::Rgbx8 => r | (g << 8) | (b << 16),
            PixelFormat::Bgrx8 => b | (g << 8) | (r << 16),
        }
    }
}

/// Raw UEFI GOP framebuffer geometry, carried as-is until Phase 4 gives
/// x86_64 a `Framebuffer` trait impl to wrap it in.
#[derive(Clone, Copy, Debug)]
pub struct RawFramebufferInfo {
    /// Physical address of pixel (0, 0).
    pub base: u64,
    /// Size in bytes of the region `base` points at.
    pub size: usize,
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Pixels per scanline - may exceed `width` if hardware pads rows.
    pub pixels_per_scan_line: u32,
    pub pixel_format: PixelFormat,
}

impl RawFramebufferInfo {
    /// Fills every visible pixel with a `0x00RRGGBB` color.
    pub fn fill(&self, color: u32) {
        self.fill_rect(0, 0, self.width as usize, self.height as usize, color);
    }

    /// Fills a rectangle with a `0x00RRGGBB` color. The rectangle is clipped
    /// to the visible area.
    pub fn fill_rect(&self, x: usize, y: usize, width: usize, height: usize, color: u32) {
        let (screen_w, screen_h) = (self.width as usize, self.height as usize);
        if x >= screen_w || y >= screen_h {
            return;
        }
        let width = width.min(screen_w - x);
        let height = height.min(screen_h - y);

        let encoded = self.pixel_format.encode(color);
        let stride = self.pixels_per_scan_line as usize;
        let capacity = self.size / size_of::<u32>();
        let base = self.base as usize as *mut u32;

        for row in y..y + height {
            for col in x..x + width {
                let index = row * stride + col;
                if index >= capacity {
                    return;
                }
                // SAFETY: `index` is inside `size` bytes from `base`, which the
                // struct's invariant says is mapped and writable.
                unsafe { base.add(index).write_volatile(encoded) };
            }
        }
    }
}

/// Basic services and hardware description handed to `rust_main` at kernel
/// entry, regardless of arch or bootloader.
#[repr(C)]
pub struct BootInfo<'a> {
    /// The firmware/bootloader's device description
    pub device_tree: Option<DeviceTree<'a>>,

    /// Usable RAM regions discovered during boot.
    pub memory_regions: RegionList,

    /// The `(base, size)` physical range `device_tree` itself occupies, if
    /// it needs protecting from the PMM handing it out as free RAM.
    pub device_tree_reserved_range: Option<(usize, usize)>,

    /// A framebuffer set up by firmware before the kernel took over, if any.
    pub framebuffer: Option<RawFramebufferInfo>,
}

impl<'a> BootInfo<'a> {
    /// Builds an empty `BootInfo` with every field absent.
    pub const fn empty() -> Self {
        Self {
            device_tree: None,
            memory_regions: RegionList::empty(),
            device_tree_reserved_range: None,
            framebuffer: None,
        }
    }
}
