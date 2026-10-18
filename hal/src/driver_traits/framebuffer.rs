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

/// The framebuffer the firmware set up (UEFI GOP on x86_64) is usable as a
/// regular [`Framebuffer`].
impl Framebuffer for crate::services::boot_info::RawFramebufferInfo {
    fn dimensions(&self) -> (usize, usize) {
        (self.width as usize, self.height as usize)
    }

    fn fill(&self, color: u32) {
        crate::services::boot_info::RawFramebufferInfo::fill(self, color);
    }

    fn blit_xrgb8888(
        &self,
        x: usize,
        y: usize,
        source: &[u32],
        source_width: usize,
        source_height: usize,
    ) -> Result<(), FramebufferError> {
        let source_pixels = source_width
            .checked_mul(source_height)
            .ok_or(FramebufferError::DimensionsOverflow)?;
        if source.len() < source_pixels {
            return Err(FramebufferError::SourceTooSmall);
        }
        let (width, height) = self.dimensions();
        if x.checked_add(source_width).is_none_or(|right| right > width)
            || y.checked_add(source_height).is_none_or(|bottom| bottom > height)
        {
            return Err(FramebufferError::DestinationOutOfBounds);
        }

        let stride = self.pixels_per_scan_line as usize;
        let capacity = self.size / size_of::<u32>();
        let base = self.base as usize as *mut u32;
        for row in 0..source_height {
            for column in 0..source_width {
                let index = (y + row) * stride + x + column;
                if index >= capacity {
                    return Err(FramebufferError::DestinationOutOfBounds);
                }
                let pixel = self.pixel_format.encode(source[row * source_width + column]);
                // SAFETY: `index` is inside the `size` bytes at `base`, which
                // the struct's invariant says are mapped and writable.
                unsafe { base.add(index).write_volatile(pixel) };
            }
        }
        Ok(())
    }
}

define_probe!(Framebuffer, probe_framebuffer, |_d| {});