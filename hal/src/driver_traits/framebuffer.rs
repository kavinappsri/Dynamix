//! Generic framebuffer output and device-tree driver discovery.

use crate::DeviceTree;

/// A pixel-addressable XRGB8888 framebuffer.
///
/// Drivers own the backing storage and all device-specific setup. The kernel
/// can therefore draw without knowing whether the display is RAMFB, a SoC
/// controller, or another framebuffer implementation.
pub trait Framebuffer: Sync {
    /// Returns the visible `(width, height)` in pixels.
    fn dimensions(&self) -> (usize, usize);

    /// Fills every visible pixel with an XRGB8888 `color`.
    fn fill(&self, color: u32);

    /// Copies XRGB8888 pixels into a destination rectangle.
    fn blit_xrgb8888(
        &self,
        x: usize,
        y: usize,
        source: &[u32],
        source_width: usize,
        source_height: usize,
    ) -> Result<(), FramebufferError>;
}

/// HAL-level framebuffer failures.
///
/// Device-specific protocol failures intentionally map to
/// [`FramebufferError::InitializationFailed`], keeping the kernel independent
/// from any one display driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramebufferError {
    /// No compiled framebuffer driver matched the device tree.
    NoCompatibleFramebufferDriver,
    /// A matching driver's device-specific initialization failed.
    InitializationFailed,
    /// The source slice is shorter than its declared dimensions.
    SourceTooSmall,
    /// The requested destination rectangle does not fit the display.
    DestinationOutOfBounds,
    /// A framebuffer dimension calculation overflowed.
    DimensionsOverflow,
}

/// A framebuffer driver selected through a device-tree compatible string.
#[repr(C)]
pub struct FramebufferDriver {
    /// Human-readable driver name for inspection and debugging.
    pub name: &'static str,
    /// Device-tree `compatible` value handled by this driver.
    pub compatible: &'static str,
    /// Initializes the driver for the supplied mapped MMIO base address.
    pub init: fn(usize) -> Result<&'static dyn Framebuffer, FramebufferError>,
}

/// Registers a framebuffer driver in the linker-retained discovery table.
#[macro_export]
macro_rules! register_framebuffer_driver {
    ($ident:ident, $compat:expr, $init_fn:path) => {
        #[used]
        #[unsafe(link_section = ".drivers.framebuffer")]
        static $ident: $crate::driver_traits::framebuffer::FramebufferDriver =
            $crate::driver_traits::framebuffer::FramebufferDriver {
                name: stringify!($ident),
                compatible: $compat,
                init: $init_fn,
            };
    };
}

unsafe extern "C" {
    static __start_framebuffer_drivers: u8;
    static __stop_framebuffer_drivers: u8;
}

fn framebuffer_drivers() -> &'static [FramebufferDriver] {
    // SAFETY: the linker script places retained `FramebufferDriver` values
    // contiguously between these two symbols.
    unsafe {
        let start = core::ptr::addr_of!(__start_framebuffer_drivers) as *const FramebufferDriver;
        let stop = core::ptr::addr_of!(__stop_framebuffer_drivers) as *const FramebufferDriver;
        let count = (stop as usize - start as usize) / core::mem::size_of::<FramebufferDriver>();
        core::slice::from_raw_parts(start, count)
    }
}

/// Finds and initializes the first compiled framebuffer driver in `tree`.
pub fn probe_framebuffer(
    tree: &impl DeviceTree,
) -> Result<&'static dyn Framebuffer, FramebufferError> {
    for driver in framebuffer_drivers() {
        if let Some((base, size)) = tree.compatible_address(driver.compatible) {
            crate::services::mmu::map_device(base, size).map_err(|_| FramebufferError::InitializationFailed)?;
            return (driver.init)(base);
        }
    }

    Err(FramebufferError::NoCompatibleFramebufferDriver)
}
