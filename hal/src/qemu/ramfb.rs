//! QEMU RAM framebuffer configuration and pixel access.

use core::mem::size_of;

use super::FwCfg;

const RAMFB_FILE: &str = "etc/ramfb";
const XRGB8888: u32 = 0x3432_5258;

// This is the byte-level payload defined by QEMU's `etc/ramfb` fw_cfg file.
// It has no trailing padding: QEMU expects exactly 28 bytes.
#[repr(C, packed)]
struct RamFbConfig {
    address: u64,
    fourcc: u32,
    flags: u32,
    width: u32,
    height: u32,
    stride: u32,
}

const _: () = assert!(size_of::<RamFbConfig>() == 28);

/// A mutable XRGB8888 framebuffer supplied by the kernel.
pub struct Framebuffer {
    pixels: *mut u32,
    pixel_count: usize,
}

/// Errors returned while configuring or drawing to a RAM framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RamFbError {
    /// The supplied framebuffer does not contain `width * height` pixels.
    FramebufferTooSmall,
    /// A dimension calculation overflowed `usize`.
    DimensionsOverflow,
    /// A dimension or stride cannot be represented by QEMU's `u32` protocol.
    DimensionsTooLarge,
    /// QEMU did not expose the `etc/ramfb` configuration file.
    DeviceNotFound,
    /// The source slice is shorter than the declared source rectangle.
    SourceTooSmall,
    /// The requested destination rectangle lies outside the visible framebuffer.
    DestinationOutOfBounds,
    /// The underlying `fw_cfg` operation failed.
    FwCfg(crate::FwCfgError),
}

impl From<crate::FwCfgError> for RamFbError {
    fn from(error: crate::FwCfgError) -> Self {
        Self::FwCfg(error)
    }
}

impl Framebuffer {
    /// Creates a framebuffer view over `pixel_count` writable XRGB8888 pixels.
    ///
    /// # Safety
    /// The range must be valid, uniquely writable framebuffer memory for the
    /// lifetime of the returned value.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let framebuffer = unsafe { Framebuffer::new(pixels.as_mut_ptr(), pixels.len()) };
    /// ```
    pub const unsafe fn new(pixels: *mut u32, pixel_count: usize) -> Self {
        Self {
            pixels,
            pixel_count,
        }
    }

    fn write(&self, index: usize, color: u32) {
        // SAFETY: `RamFb` validates all indices against this buffer's capacity.
        unsafe { self.pixels.add(index).write_volatile(color) }
    }
}

/// A configured QEMU RAM framebuffer.
pub struct RamFb {
    framebuffer: Framebuffer,
    width: usize,
    height: usize,
}

impl RamFb {
    /// Configures QEMU's RAM framebuffer to scan out the supplied pixel buffer.
    ///
    /// Pixels use the XRGB8888 layout and are scanned out with no additional
    /// copies after configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when dimensions overflow, the buffer is too small, QEMU
    /// does not provide `etc/ramfb`, or the underlying `fw_cfg` transfer fails.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let display = RamFb::configure(&fw_cfg, framebuffer, 1024, 600)?;
    /// display.fill(0x00_20_20_20);
    /// # Ok::<(), hal::RamFbError>(())
    /// ```
    pub fn configure(
        fw_cfg: &FwCfg,
        framebuffer: Framebuffer,
        width: usize,
        height: usize,
    ) -> Result<Self, RamFbError> {
        let pixel_count = width
            .checked_mul(height)
            .ok_or(RamFbError::DimensionsOverflow)?;
        if framebuffer.pixel_count < pixel_count {
            return Err(RamFbError::FramebufferTooSmall);
        }

        let selector = fw_cfg
            .find_file(RAMFB_FILE)?
            .ok_or(RamFbError::DeviceNotFound)?;
        let stride = width
            .checked_mul(size_of::<u32>())
            .ok_or(RamFbError::DimensionsOverflow)?;

        let width = u32::try_from(width).map_err(|_| RamFbError::DimensionsTooLarge)?;
        let height = u32::try_from(height).map_err(|_| RamFbError::DimensionsTooLarge)?;
        let stride = u32::try_from(stride).map_err(|_| RamFbError::DimensionsTooLarge)?;
        let config = RamFbConfig {
            address: (framebuffer.pixels as usize as u64).to_be(),
            fourcc: XRGB8888.to_be(),
            flags: 0,
            width: width.to_be(),
            height: height.to_be(),
            stride: stride.to_be(),
        };
        fw_cfg.write_object(selector, &config)?;

        Ok(Self {
            framebuffer,
            width: width as usize,
            height: height as usize,
        })
    }

    /// Fills every visible pixel with `color` in XRGB8888 format.
    pub fn fill(&self, color: u32) {
        for index in 0..self.width * self.height {
            self.framebuffer.write(index, color);
        }
    }

    /// Copies XRGB8888 pixels to a rectangle in the framebuffer.
    ///
    /// # Errors
    ///
    /// Returns [`RamFbError::SourceTooSmall`] if `source` does not cover its
    /// declared dimensions, [`RamFbError::DestinationOutOfBounds`] if the
    /// rectangle does not fit, or [`RamFbError::DimensionsOverflow`] if the
    /// source pixel count overflows.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// display.blit_xrgb8888(0, 0, &logo_pixels, logo_width, logo_height)?;
    /// # Ok::<(), hal::RamFbError>(())
    /// ```
    pub fn blit_xrgb8888(
        &self,
        x: usize,
        y: usize,
        source: &[u32],
        source_width: usize,
        source_height: usize,
    ) -> Result<(), RamFbError> {
        let source_pixels = source_width
            .checked_mul(source_height)
            .ok_or(RamFbError::DimensionsOverflow)?;
        if source.len() < source_pixels {
            return Err(RamFbError::SourceTooSmall);
        }
        if x.checked_add(source_width)
            .is_none_or(|right| right > self.width)
            || y.checked_add(source_height)
                .is_none_or(|bottom| bottom > self.height)
        {
            return Err(RamFbError::DestinationOutOfBounds);
        }

        for row in 0..source_height {
            for column in 0..source_width {
                self.framebuffer.write(
                    (y + row) * self.width + x + column,
                    source[row * source_width + column],
                );
            }
        }

        Ok(())
    }
}
