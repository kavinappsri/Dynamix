//! Kernel boot-logo presentation policy.

use core::mem::size_of;
use hal::RamFb;

const WIDTH: usize = 729;
const HEIGHT: usize = 228;

#[repr(align(4))]
struct AlignedPixels([u8; WIDTH * HEIGHT * size_of::<u32>()]);

static PIXELS: AlignedPixels = AlignedPixels(*include_bytes!("logo.bin"));

/// Draws the embedded logo at the center of the supplied display dimensions.
///
/// # Panics
///
/// Panics if the declared display dimensions are smaller than the embedded
/// logo, or if the framebuffer rejects the destination rectangle.
pub fn draw_centered(framebuffer: &RamFb, screen_width: usize, screen_height: usize) {
    let x = (screen_width - WIDTH) / 2;
    let y = (screen_height - HEIGHT) / 2;
    framebuffer
        .blit_xrgb8888(x, y, pixels(), WIDTH, HEIGHT)
        .expect("Embedded logo does not fit the framebuffer");
}

fn pixels() -> &'static [u32] {
    // SAFETY: `PIXELS` is explicitly aligned for `u32`, contains exactly the
    // required number of bytes, and the image is stored as native-endian pixels.
    unsafe { core::slice::from_raw_parts(PIXELS.0.as_ptr().cast::<u32>(), WIDTH * HEIGHT) }
}
