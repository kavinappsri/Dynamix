//! Generic framebuffer output and device-tree driver discovery.

use crate::define_probe;
use crate::services::dtb::Dtb;

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
    /// The source slice is shorter than its declared dimensions.
    SourceTooSmall,
    /// The requested destination rectangle does not fit the display.
    DestinationOutOfBounds,
    /// A framebuffer dimension calculation overflowed.
    DimensionsOverflow,
}

define_probe!("__start_framebuffer_drivers", "__stop_framebuffer_drivers", probe_framebuffer, Framebuffer ,|_d| {});