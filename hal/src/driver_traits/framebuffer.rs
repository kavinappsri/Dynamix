//! Generic framebuffer output and device-tree driver discovery.

use crate::define_probe;

/// A pixel-addressable framebuffer.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramebufferError {
    /// The source slice is shorter than its declared dimensions.
    SourceTooSmall,
    /// The requested destination rectangle does not fit the display.
    DestinationOutOfBounds,
    /// A framebuffer dimension calculation overflowed.
    DimensionsOverflow,
}

define_probe!(Framebuffer, probe_framebuffer, |_d| {});