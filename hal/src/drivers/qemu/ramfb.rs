//! QEMU RAM framebuffer driver.

use core::mem::size_of;

use crate::driver_traits::framebuffer::FramebufferError::{DestinationOutOfBounds, DimensionsOverflow, SourceTooSmall};
use crate::drivers::qemu::fw_cfg::{FwCfg, RAMFB_FILE};
use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::framebuffer;
use crate::services::device::Device;
use crate::services::provider;
use crate::services::pmm;
use crate::{Framebuffer, FramebufferError};

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

/// Frames the pixel buffer takes, as a buddy-allocator order.
const FRAMEBUFFER_ORDER: usize = (PIXEL_COUNT * size_of::<u32>())
    .div_ceil(pmm::FRAME_SIZE)
    .next_power_of_two()
    .trailing_zeros() as usize;

/// QEMU RAM framebuffer implementation hidden behind [`Framebuffer`].
struct RamFb {
    pixels: *mut u32,
}

// Pixel writes use volatile stores through a shared framebuffer reference; the
// buffer belongs to this instance for the rest of the boot.
unsafe impl Sync for RamFb {}

impl RamFb {
    fn configure(fw_cfg: &FwCfg) -> Result<Self, ProbeError> {
        let selector = fw_cfg
            .find_file(RAMFB_FILE)
            .map_err(|_| ProbeError::InitFailed)?
            // A fw_cfg without a ramfb file is a machine with no such display.
            .ok_or(ProbeError::NotPresent)?;

        let frame = pmm::alloc_frames(FRAMEBUFFER_ORDER).map_err(|_| ProbeError::MappingFailed)?;
        // RAM from the physical allocator is identity mapped.
        let address = frame.addr().as_usize();
        let pixels = address as *mut u32;
        // SAFETY: the block is ours, mapped, and `PIXEL_COUNT` pixels fit in it.
        unsafe { core::ptr::write_bytes(pixels, 0, PIXEL_COUNT) };

        let config = RamFbConfig {
            address: (address as u64).to_be(),
            fourcc: XRGB8888.to_be(),
            flags: 0,
            width: (WIDTH as u32).to_be(),
            height: (HEIGHT as u32).to_be(),
            stride: ((WIDTH * size_of::<u32>()) as u32).to_be(),
        };
        if fw_cfg.write_object(selector, &config).is_err() {
            let _ = pmm::free_frames(frame, FRAMEBUFFER_ORDER);
            return Err(ProbeError::InitFailed);
        }

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

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    // Spawned by the `fw_cfg` driver, whose device is this one's parent.
    let fw_cfg: &FwCfg = provider::get(dev.parent().ok_or(ProbeError::Firmware)?)?;
    framebuffer::register(dev, RamFb::configure(fw_cfg)?);
    Ok(())
}

driver! {
    name: "ramfb",
    matches: [Match::Spawned("qemu,ramfb")],
    probe: probe,
}
