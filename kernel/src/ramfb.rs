// ramfb.rs
use crate::fw_cfg::FwCfg;

const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;

#[repr(C, packed)]
struct RamFbConfig {
    addr: u64,
    fourcc: u32,
    flags: u32,
    width: u32,
    height: u32,
    stride: u32,
}

pub struct RamFb {
    width: usize,
    height: usize,
    framebuffer: *mut u32,
}
const LOGO_WIDTH: usize = 729;
const LOGO_HEIGHT: usize = 228;

static LOGO_BYTES: &[u8] = include_bytes!("logo.bin");

impl RamFb {
    /// Initializes the Ramfb device by locating it in fw_cfg and passing the configuration via DMA.
    pub fn new(fw_cfg: &FwCfg, fb_addr: *mut u32, width: usize, height: usize) -> Result<Self, &'static str> {
        let selector = fw_cfg
            .find_file("etc/ramfb")
            .ok_or("etc/ramfb device not found in fw_cfg directory")?;

        let fb_addr_u64 = fb_addr as usize as u64;

        let config = RamFbConfig {
            addr: fb_addr_u64.to_be(),
            fourcc: DRM_FORMAT_XRGB8888.to_be(),
            flags: 0,
            width: (width as u32).to_be(),
            height: (height as u32).to_be(),
            stride: ((width * 4) as u32).to_be(),
        };

        unsafe {
            fw_cfg.do_dma_write(
                selector,
                &config as *const _ as u64,
                size_of::<RamFbConfig>() as u32,
            );
        }

        Ok(Self {
            width,
            height,
            framebuffer: fb_addr,
        })
    }

    /// Fills the entire screen directly in RAM with a specific XRGB8888 color.
    pub fn fill_screen(&self, color: u32) {
        unsafe {
            for i in 0..(self.width * self.height) {
                self.framebuffer.add(i).write_volatile(color);
            }
        }
    }
    pub fn draw_logo(&self) {
        let start_x = (self.width - LOGO_WIDTH) / 2;
        let start_y = (self.height - LOGO_HEIGHT) / 2;

        let logo_data: &[u32] = unsafe {
            core::slice::from_raw_parts(
                LOGO_BYTES.as_ptr() as *const u32,
                LOGO_WIDTH * LOGO_HEIGHT
            )
        };

        for y in 0..LOGO_HEIGHT {
            for x in 0..LOGO_WIDTH {
                let screen_x = start_x + x;
                let screen_y = start_y + y;

                if screen_x < self.width && screen_y < self.height {
                    let logo_pixel = logo_data[y * LOGO_WIDTH + x];
                    let fb_offset = (screen_y * self.width) + screen_x;
                    unsafe {
                        self.framebuffer.add(fb_offset).write_volatile(logo_pixel);
                    }
                }
            }
        }
    }
}