//! RK3126/RK3128 Video Output Processor (VOP, historically "LCDC")
//! framebuffer driver.
//!
//! TODO for this driver:
//! - Window 1, the hardware cursor plane, and the scaler (left disabled)
//! - BCSH (post-processing), TV/HDMI output paths
//! - Panel timings are a fixed built-in const
//! - No VOP-internal IOMMU setup

use core::cell::UnsafeCell;
use crate::driver_traits::clocks::ClockController;
use crate::driver_traits::framebuffer::{FramebufferError, Framebuffer};
use crate::services::mmio::Mmio;
use crate::services::mmu::va_to_pa;
use crate::drivers::rockchip::rk312x_cru::Rk3126Clock;
use crate::register_framebuffer_driver;
use crate::services::sync::StaticCell;

// --- Register Offsets ---

const SYS_CTRL: usize = 0x00;
const DSP_CTRL0: usize = 0x04;
const DSP_CTRL1: usize = 0x08;
const ALPHA_CTRL: usize = 0x14;
const WIN0_YRGB_MST: usize = 0x20;
const WIN0_CBR_MST: usize = 0x24;
const WIN0_VIR: usize = 0x30;
const WIN0_ACT_INFO: usize = 0x34;
const WIN0_DSP_INFO: usize = 0x38;
const WIN0_DSP_ST: usize = 0x3c;
const WIN0_SCL_FACTOR_YRGB: usize = 0x40;
const WIN0_SCL_FACTOR_CBR: usize = 0x44;
const AXI_BUS_CTRL: usize = 0x2c;
const DSP_HTOTAL_HS_END: usize = 0x6c;
const DSP_HACT_ST_END: usize = 0x70;
const DSP_VTOTAL_VS_END: usize = 0x74;
const DSP_VACT_ST_END: usize = 0x78;
const REG_CFG_DONE: usize = 0x90;

// --- SYS_CTRL bits ---
const M_WIN0_EN: u32 = 1 << 0;
const M_WIN0_FORMAT: u32 = 0x7 << 3;
const M_AUTO_GATING_EN: u32 = 1 << 31;
// VOP_FORMAT_ARGB888 == 0, this is a no-op bit pattern, kept named and
// written explicitly so the format
// selection isn't dependent on POR defaults. Framebuffer::fill and
// blit_xrgb8888 write XRGB8888 (top byte ignored by convention); the VOP
// hardware format for that byte layout is the same ARGB888 mode, made
// effectively-X by disabling M_WIN0_ALPHA_EN in ALPHA_CTRL below.
const V_WIN0_FORMAT_ARGB888: u32 = 0 << 3;

// --- DSP_CTRL0 bits ---
const M_HSYNC_POL: u32 = 1 << 4;
const M_VSYNC_POL: u32 = 1 << 5;
const M_DEN_POL: u32 = 1 << 6;
const M_DCLK_POL: u32 = 1 << 7;
const M_DSP_OUT_FORMAT: u32 = 0xf << 0;
const V_DSP_OUT_FORMAT_P888: u32 = 0xb; // OUT_P888 (24bpp RGB, no dither)

// --- AXI_BUS_CTRL bits (RK312x RGB output clock enable) ---
const M_RGB_DCLK_EN: u32 = 1 << 24;
const M_RGB_DCLK_INVERT: u32 = 1 << 25;

// --- ALPHA_CTRL bits ---
const M_WIN0_ALPHA_EN: u32 = 1 << 0;
const M_WIN1_ALPHA_EN: u32 = 1 << 1;

/// A fixed-function panel timing. All fields are in pixel clocks
#[derive(Clone, Copy)]
struct Mode {
    xres: u32,
    yres: u32,
    pixel_clock_hz: u32,
    hsync_len: u32,
    left_margin: u32,
    right_margin: u32,
    vsync_len: u32,
    upper_margin: u32,
    lower_margin: u32,
}

/// Built-in default: a common 800x480 parallel-RGB panel timing, the size
/// of display most often paired with RK3126 boards. See the module docs
/// boards with a different panel need this changed until timings can be
/// read from the device tree.
const DEFAULT_MODE: Mode = Mode {
    xres: 800,
    yres: 480,
    pixel_clock_hz: 33_000_000,
    hsync_len: 1,
    left_margin: 46,
    right_margin: 210,
    vsync_len: 1,
    upper_margin: 23,
    lower_margin: 22,
};

impl Mode {
    const fn htotal(&self) -> u32 {
        self.hsync_len + self.left_margin + self.xres + self.right_margin
    }

    const fn vtotal(&self) -> u32 {
        self.vsync_len + self.upper_margin + self.yres + self.lower_margin
    }

    const fn hact_start(&self) -> u32 {
        self.hsync_len + self.left_margin
    }

    const fn vact_start(&self) -> u32 {
        self.vsync_len + self.upper_margin
    }
}

/// Backing pixel storage for the framebuffer. Sized for the built-in
/// default mode; a board using a different (larger) [`DEFAULT_MODE`] must
/// grow this to match.
const PIXEL_COUNT: usize = (DEFAULT_MODE.xres * DEFAULT_MODE.yres) as usize;

#[repr(align(4096))]
struct DisplayMemory(UnsafeCell<[u32; PIXEL_COUNT]>);

unsafe impl Sync for DisplayMemory {}

static DISPLAY_MEMORY: DisplayMemory = DisplayMemory(UnsafeCell::new([0; PIXEL_COUNT]));

struct Rk312xVop {
    regs: Mmio,
    pixels: *mut u32,
    mode: Mode,
}

unsafe impl Sync for Rk312xVop {}

impl Rk312xVop {
    fn write(&self, index: usize, color: u32) {
        // SAFETY: all callers calculate indices inside the fixed display.
        unsafe { self.pixels.add(index).write_volatile(color) };
    }

    /// Un-gates and rate-sets every clock the VOP output path needs
    fn bring_up_clocks(clocks: &dyn ClockController, mode: &Mode) -> Result<(), FramebufferError> {
        clocks
            .enable(Rk3126Clock::AclkLcdc0 as u32)
            .map_err(|_| FramebufferError::InitializationFailed)?;
        clocks
            .enable(Rk3126Clock::HclkLcdc0 as u32)
            .map_err(|_| FramebufferError::InitializationFailed)?;

        // The divider must be programmed before the pixel clock is ungated
        clocks
            .set_rate(Rk3126Clock::DclkVop as u32, mode.pixel_clock_hz)
            .map_err(|_| FramebufferError::InitializationFailed)?;
        clocks
            .enable(Rk3126Clock::DclkVop as u32)
            .map_err(|_| FramebufferError::InitializationFailed)?;

        Ok(())
    }

    fn configure(vop_base: usize, clocks: &dyn ClockController) -> Result<Self, FramebufferError> {
        let mode = DEFAULT_MODE;
        Self::bring_up_clocks(clocks, &mode)?;

        // SAFETY: `vop_base` came from the validated, mapped device tree
        // via `probe_framebuffer`.
        let regs = unsafe { Mmio::new(vop_base) };
        let pixels = DISPLAY_MEMORY.0.get().cast::<u32>();

        let vop = Self { regs, pixels, mode };
        vop.program_registers();
        Ok(vop)
    }

    fn program_registers(&self) {
        // Disable auto-clock-gating during setup
        self.mask_write(SYS_CTRL, M_AUTO_GATING_EN, 0);

        // RK312x-specific: route the RGB parallel output's pixel clock.
        self.mask_write(
            AXI_BUS_CTRL,
            M_RGB_DCLK_EN | M_RGB_DCLK_INVERT,
            M_RGB_DCLK_EN,
        );

        // Output polarity (active-high sync, active-high data-enable,
        // rising-edge pixel clock) and 24bpp RGB888 output format.
        self.mask_write(
            DSP_CTRL0,
            M_HSYNC_POL | M_VSYNC_POL | M_DEN_POL | M_DCLK_POL | M_DSP_OUT_FORMAT,
            V_DSP_OUT_FORMAT_P888,
        );

        // Background color black; no swaps.
        self.regs.write32(DSP_CTRL1, 0);

        // Horizontal timing.
        let htotal = self.mode.htotal();
        self.regs.write32(
            DSP_HTOTAL_HS_END,
            (self.mode.hsync_len & 0xfff) | ((htotal & 0xfff) << 16),
        );
        let hact_start = self.mode.hact_start();
        let hact_end = hact_start + self.mode.xres;
        self.regs.write32(
            DSP_HACT_ST_END,
            (hact_end & 0xfff) | ((hact_start & 0xfff) << 16),
        );

        // Vertical timing (progressive only -- no interlaced field support).
        let vtotal = self.mode.vtotal();
        self.regs.write32(
            DSP_VTOTAL_VS_END,
            (self.mode.vsync_len & 0xfff) | ((vtotal & 0xfff) << 16),
        );
        let vact_start = self.mode.vact_start();
        let vact_end = vact_start + self.mode.yres;
        self.regs.write32(
            DSP_VACT_ST_END,
            (vact_end & 0xfff) | ((vact_start & 0xfff) << 16),
        );

        // Window 0: full-screen, no scaling (unity scale factor), XRGB8888.
        // WIN0_VIR's stride field is in pixels for 32bpp formats
        let stride_words = self.mode.xres;
        self.regs.write32(WIN0_VIR, stride_words & 0x1fff);
        self.regs.write32(
            WIN0_ACT_INFO,
            ((self.mode.xres - 1) & 0x1fff) | (((self.mode.yres - 1) & 0x1fff) << 16),
        );
        self.regs.write32(
            WIN0_DSP_INFO,
            ((self.mode.xres - 1) & 0x7ff) | (((self.mode.yres - 1) & 0x7ff) << 16),
        );
        self.regs.write32(
            WIN0_DSP_ST,
            (hact_start & 0xfff) | ((vact_start & 0xfff) << 16),
        );
        // Unity scale (CalScale(x, x) == 0x1000 for any x)
        self.regs.write32(WIN0_SCL_FACTOR_YRGB, 0x1000 | (0x1000 << 16));
        self.regs.write32(WIN0_SCL_FACTOR_CBR, 0x1000 | (0x1000 << 16));


        let framebuffer_pa = va_to_pa(self.pixels as usize);
        self.regs.write32(WIN0_YRGB_MST, framebuffer_pa as u32);
        self.regs.write32(WIN0_CBR_MST, 0);

        self.mask_write(SYS_CTRL, M_WIN0_FORMAT, V_WIN0_FORMAT_ARGB888);
        self.mask_write(SYS_CTRL, M_WIN0_EN, M_WIN0_EN);

        // No alpha blending: only one opaque window is active.
        self.mask_write(ALPHA_CTRL, M_WIN0_ALPHA_EN | M_WIN1_ALPHA_EN, 0);

        self.cfg_done();
    }

    fn mask_write(&self, offset: usize, mask: u32, value: u32) {
        let current = self.regs.read32(offset);
        self.regs.write32(offset, (current & !mask) | (value & mask));
    }

    /// Latches every register write since the last `cfg_done` into the
    /// VOP's active (shadow-swapped) configuration, applied at the next
    /// frame boundary.
    fn cfg_done(&self) {
        self.regs.write32(REG_CFG_DONE, 0x1);
    }
}

impl Framebuffer for Rk312xVop {
    fn dimensions(&self) -> (usize, usize) {
        (self.mode.xres as usize, self.mode.yres as usize)
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
            .ok_or(FramebufferError::DimensionsOverflow)?;
        if source.len() < source_pixels {
            return Err(FramebufferError::SourceTooSmall);
        }

        let width = self.mode.xres as usize;
        let height = self.mode.yres as usize;
        if x.checked_add(source_width).is_none_or(|right| right > width)
            || y.checked_add(source_height).is_none_or(|bottom| bottom > height)
        {
            return Err(FramebufferError::DestinationOutOfBounds);
        }

        for row in 0..source_height {
            for column in 0..source_width {
                self.write(
                    (y + row) * width + x + column,
                    source[row * source_width + column],
                );
            }
        }
        Ok(())
    }
}

fn init_rk312x_vop(vop_base: usize) -> Result<&'static dyn Framebuffer, FramebufferError> {
    static INSTANCE: StaticCell<Rk312xVop> = StaticCell::uninit();
    let clocks = crate::driver_traits::clocks::active_clock().ok_or(FramebufferError::InitializationFailed)?;

    let vop = Rk312xVop::configure(vop_base, clocks)?;
    let vop: &'static Rk312xVop = INSTANCE.get_or_init(|| vop);
    Ok(vop)
}

register_framebuffer_driver!(RK312X_VOP, "rockchip,rk312x-lcdc", init_rk312x_vop);