 //! The linear framebuffer the firmware set up before the kernel took over
//! (UEFI GOP on x86_64), usable as a regular [`Framebuffer`].

use crate::driver_traits::framebuffer::{Framebuffer, FramebufferError};

/// Byte order of a 32-bit framebuffer pixel, as laid out in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Bytes in memory are `R, G, B, unused`.
    Rgbx8,
    /// Bytes in memory are `B, G, R, unused`.
    Bgrx8,
}

impl PixelFormat {
    /// Encodes a `0x00RRGGBB` color into the 32-bit value the format expects.
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

/// A firmware-provided 32-bit linear framebuffer.
#[derive(Clone, Copy, Debug)]
pub struct FirmwareFramebuffer {
    base: usize,
    size: usize,
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
}

impl FirmwareFramebuffer {
    /// Builds a framebuffer from firmware-reported geometry. `None` if the
    /// geometry is empty, inconsistent, or does not fit in `size` bytes.
    pub fn new(base: usize, size: usize, width: u32, height: u32, pixels_per_scan_line: u32, format: PixelFormat) -> Option<Self> {
        let (width, height, stride) = (width as usize, height as usize, pixels_per_scan_line as usize);
        if base == 0 || width == 0 || height == 0 || stride < width {
            return None;
        }
        let last_pixel = (height - 1).checked_mul(stride)?.checked_add(width)?;
        if last_pixel.checked_mul(size_of::<u32>())? > size {
            return None;
        }
        Some(Self { base, size, width, height, stride, format })
    }

    /// Physical address of pixel (0, 0).
    pub const fn base(&self) -> usize {
        self.base
    }

    /// Size in bytes of the region starting at [`base`](Self::base).
    pub const fn size(&self) -> usize {
        self.size
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    /// Writes one already-encoded pixel. `x < width` and `y < height` must hold.
    #[inline]
    fn put(&self, x: usize, y: usize, encoded: u32) {
        debug_assert!(x < self.width && y < self.height);
        // SAFETY: `new` checked that every in-bounds pixel lies inside `size`
        // bytes from `base`, which its caller said is mapped and writable.
        unsafe { (self.base as *mut u32).add(y * self.stride + x).write_volatile(encoded) };
    }

    /// Fills every visible pixel with a `0x00RRGGBB` color.
    pub fn fill(&self, color: u32) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }

    /// Fills a rectangle with a `0x00RRGGBB` color, clipped to the visible area.
    pub fn fill_rect(&self, x: usize, y: usize, width: usize, height: usize, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let encoded = self.format.encode(color);

        for row in y..y + height {
            for col in x..x + width {
                self.put(col, row, encoded);
            }
        }
    }

    /// Copies `0x00RRGGBB` pixels into a destination rectangle.
    pub fn blit(&self, x: usize, y: usize, source: &[u32], source_width: usize, source_height: usize) -> Result<(), FramebufferError> {
        let source_pixels = source_width.checked_mul(source_height).ok_or(FramebufferError::DimensionsOverflow)?;
        if source.len() < source_pixels {
            return Err(FramebufferError::SourceTooSmall);
        }
        if x.checked_add(source_width).is_none_or(|right| right > self.width)
            || y.checked_add(source_height).is_none_or(|bottom| bottom > self.height)
        {
            return Err(FramebufferError::DestinationOutOfBounds);
        }

        for row in 0..source_height {
            for column in 0..source_width {
                let pixel = self.format.encode(source[row * source_width + column]);
                self.put(x + column, y + row, pixel);
            }
        }
        Ok(())
    }
}

impl Framebuffer for FirmwareFramebuffer {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn fill(&self, color: u32) {
        FirmwareFramebuffer::fill(self, color);
    }

    fn blit_xrgb8888(&self, x: usize, y: usize, source: &[u32], source_width: usize, source_height: usize) -> Result<(), FramebufferError> {
        self.blit(x, y, source, source_width, source_height)
    }
}
