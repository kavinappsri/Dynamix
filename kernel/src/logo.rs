//! Kernel boot-logo presentation policy.

use core::mem::size_of;
use hal::{Framebuffer, FramebufferError};

const WIDTH: usize = 729;
const HEIGHT: usize = 228;

#[repr(align(4))]
struct AlignedPixels([u8; WIDTH * HEIGHT * size_of::<u32>()]);

static PIXELS: AlignedPixels = AlignedPixels(*include_bytes!("logo.bin"));

/// Draws the embedded logo at the center of any HAL framebuffer.
pub fn draw_centered(framebuffer: &dyn Framebuffer) -> Result<(), FramebufferError> {
    let (screen_width, screen_height) = framebuffer.dimensions();
    let x = screen_width
        .checked_sub(WIDTH)
        .ok_or(FramebufferError::DestinationOutOfBounds)?
        / 2;
    let y = screen_height
        .checked_sub(HEIGHT)
        .ok_or(FramebufferError::DestinationOutOfBounds)?
        / 2;
    framebuffer.blit_xrgb8888(x, y, pixels(), WIDTH, HEIGHT)
}

fn pixels() -> &'static [u32] {
    // SAFETY: `PIXELS` is explicitly aligned for `u32`, contains exactly the
    // required number of bytes, and the image is stored as native-endian pixels.
    unsafe { core::slice::from_raw_parts(PIXELS.0.as_ptr().cast::<u32>(), WIDTH * HEIGHT) }
}
