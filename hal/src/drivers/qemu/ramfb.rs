//! QEMU RAM framebuffer driver.

use crate::services::mmu::va_to_pa;
use core::{cell::UnsafeCell, mem::size_of};

use crate::driver_traits::framebuffer::FramebufferError::{DestinationOutOfBounds, DimensionsOverflow, SourceTooSmall};
use crate::drivers::qemu::fw_cfg::FwCfg;
use crate::services::sync::StaticCell;
use crate::{
    Framebuffer, FramebufferError,
    register_framebuffer_driver,
};

const RAMFB_FILE: &str = "etc/ramfb";
const XRGB8888: u32 = 0x3432_5258;
const WIDTH: usize = 1024;
const HEIGHT: usize = 600;
const PIXEL_COUNT: usize = WIDTH * HEIGHT;

// Byte-level payload defined by QEMU's `etc/ramfb` fw_cfg file.
// Has no trailing padding - QEMU expects only 28 bytes.
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

#[repr(align(16))]
struct DisplayMemory(UnsafeCell<[u32; PIXEL_COUNT]>);

// RAMFB is the only writer to this memory after initialization. Early boot is
// single-core; future multi core display support must add external locking.
unsafe impl Sync for DisplayMemory {}

static DISPLAY_MEMORY: DisplayMemory = DisplayMemory(UnsafeCell::new([0; PIXEL_COUNT]));

/// QEMU RAM framebuffer implementation hidden behind [`Framebuffer`].
struct RamFb {
    pixels: *mut u32,
}

// Pixel writes use volatile stores through a shared framebuffer reference. The
// driver is selected once during single-core early boot.
unsafe impl Sync for RamFb {}

impl RamFb {
    fn configure(fw_cfg_base: usize) -> Result<Self, FramebufferError> {
        // SAFETY: this address came from the validated, identity-mapped DTB.
        let fw_cfg = unsafe { FwCfg::new(fw_cfg_base) };
        let selector = fw_cfg
            .find_file(RAMFB_FILE)
            .map_err(|_| FramebufferError::InitializationFailed)?
            .ok_or(FramebufferError::InitializationFailed)?;
        let pixels = DISPLAY_MEMORY.0.get().cast::<u32>();
        let config = RamFbConfig {
            address: (va_to_pa(pixels as usize) as u64).to_be(),
            fourcc: XRGB8888.to_be(),
            flags: 0,
            width: (WIDTH as u32).to_be(),
            height: (HEIGHT as u32).to_be(),
            stride: ((WIDTH * size_of::<u32>()) as u32).to_be(),
        };
        fw_cfg
            .write_object(selector, &config)
            .map_err(|_| FramebufferError::InitializationFailed)?;

        Ok(Self { pixels })
    }

    fn write(&self, index: usize, color: u32) {
        // SAFETY: all callers calculate indices inside the fixed display.
        unsafe { self.pixels.add(index).write_volatile(color) }
    }
}

impl Framebuffer for RamFb {
    fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    fn fill(&self, color: u32) {
        for index in 0..PIXEL_COUNT {
            self.write(index, color);
        }
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
            .ok_or(DimensionsOverflow)?;
        if source.len() < source_pixels {
            return Err(SourceTooSmall);
        }
        if x.checked_add(source_width)
            .is_none_or(|right| right > WIDTH)
            || y.checked_add(source_height)
                .is_none_or(|bottom| bottom > HEIGHT)
        {
            return Err(DestinationOutOfBounds);
        }

        for row in 0..source_height {
            for column in 0..source_width {
                self.write(
                    (y + row) * WIDTH + x + column,
                    source[row * source_width + column],
                );
            }
        }
        Ok(())
    }
}

fn init_ramfb(fw_cfg_base: usize) -> Result<&'static dyn Framebuffer, FramebufferError> {
    static INSTANCE: StaticCell<RamFb> = StaticCell::uninit();
    let ramfb = RamFb::configure(fw_cfg_base)?;
    let ramfb: &'static RamFb = INSTANCE.get_or_init(|| ramfb);
    Ok(ramfb)
}

register_framebuffer_driver!(QEMU_RAMFB, "qemu,fw-cfg-mmio", init_ramfb);
