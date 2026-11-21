//! Generic framebuffer output.

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::registry::{Class, Entry, Iter};

static FRAMEBUFFERS: Class<dyn Framebuffer> = Class::new();

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

/// The first framebuffer that was bound, if any.
pub fn primary_framebuffer() -> Option<&'static dyn Framebuffer> {
    FRAMEBUFFERS.primary()
}

/// Every framebuffer that was bound, in the order they were bound.
pub fn framebuffers() -> Iter<Entry<dyn Framebuffer>> {
    FRAMEBUFFERS.iter()
}

/// Called by a driver's probe to hand over the framebuffer it started on `device`.
pub fn register(device: &'static Device, framebuffer: impl Framebuffer + 'static) -> &'static dyn Framebuffer {
    let framebuffer: &'static dyn Framebuffer = Box::leak(Box::new(framebuffer));
    FRAMEBUFFERS.register(device, framebuffer);
    framebuffer
}
