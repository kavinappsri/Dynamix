//! RK3128, RK3126 Clock and Reset Unit (CRU) driver
//!
//! Referenced from the corresponding driver for linux (`drivers/clk/rockchip/clk-rk3128.c`).
//! this does not use a usual "read-modify-write" cycle, instead using the upper 16 bits of a
//! 32 bit write as a write mask

use crate::driver_traits::clocks::{ClockController, ClockError};
use crate::register_clock_driver;
use crate::services::mmio::Mmio;
use crate::services::sync::StaticCell;

const OSC_HZ: u32 = 24_000_000;

const fn pll_con(index: usize) -> usize {
    index * 0x4
}

const fn clksel_con(index: usize) -> usize {
    index * 0x4 + 0x44
}

const fn clkgate_con(index: usize) -> usize {
    index * 0x4 + 0xd0
}

/// Base offsets of each PLL's four-word `CON` block. Only `CPLL`/`GPLL` are read today (the only PLLs `DCLK_VOP` can
/// mux from); `APLL`/`DPLL` are recorded for future drivers
#[allow(dead_code)]
mod pll_base {
    pub const APLL: usize = 0 * 0x10;
    pub const DPLL: usize = 1 * 0x10;
    pub const CPLL: usize = 2 * 0x10;
    pub const GPLL: usize = 3 * 0x10;
}

// PLLCON0: FBDIV[11:0], POSTDIV1[14:12]
const PLLCON0_FBDIV_MASK: u32 = 0xfff;
const PLLCON0_POSTDIV1_SHIFT: u32 = 12;
const PLLCON0_POSTDIV1_MASK: u32 = 0x7;

// PLLCON1: REFDIV[5:0], POSTDIV2[8:6], DSMPD[12]
const PLLCON1_REFDIV_MASK: u32 = 0x3f;
const PLLCON1_POSTDIV2_SHIFT: u32 = 6;
const PLLCON1_POSTDIV2_MASK: u32 = 0x7;
const PLLCON1_DSMPD_SHIFT: u32 = 12;

/// `DCLK_VOP`: `CLKSEL_CON(27)`, mux bits[1:0], 8-bit divider bits[15:8],
/// gated at `CLKGATE_CON(3)` bit 1.
mod dclk_vop {
    pub const SEL_CON: usize = super::clksel_con(27);
    pub const MUX_SHIFT: u32 = 0;
    pub const MUX_MASK: u32 = 0x3;
    pub const DIV_SHIFT: u32 = 8;
    pub const DIV_MASK: u32 = 0xff;
    pub const GATE_CON: usize = super::clkgate_con(3);
    pub const GATE_BIT: u32 = 1;
}

/// `mux_sclk_vop_src_p = { "cpll", "gpll", "gpll_div2", "gpll_div3" }` --
/// the parents selectable for `DCLK_VOP` (and `SCLK_VOP`) in mux order.
#[derive(Clone, Copy)]
enum VopClockParent {
    Cpll = 0,
    Gpll = 1,
    GpllDiv2 = 2,
    GpllDiv3 = 3,
}

/// `ACLK_LCDC0`: `CLKGATE_CON(6)` bit 0. Derived from `aclk_vio0`, which the
/// VIO power-domain bus divider (not this driver) controls; we only gate it.
const ACLK_LCDC0_GATE_CON: usize = clkgate_con(6);
const ACLK_LCDC0_GATE_BIT: u32 = 0;

/// `HCLK_LCDC0`: `CLKGATE_CON(6)` bit 1. Derived from `hclk_vio`.
const HCLK_LCDC0_GATE_CON: usize = clkgate_con(6);
const HCLK_LCDC0_GATE_BIT: u32 = 1;

/// Clock identifiers this driver recognizes, passed to [`ClockController`]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Rk3126Clock {
    /// VOP pixel (dot) clock.
    DclkVop = 0,
    /// VOP AXI bus clock.
    AclkLcdc0 = 1,
    /// VOP AHB bus clock.
    HclkLcdc0 = 2,
}

impl TryFrom<u32> for Rk3126Clock {
    type Error = ClockError;

    fn try_from(value: u32) -> Result<Self, ClockError> {
        match value {
            0 => Ok(Self::DclkVop),
            1 => Ok(Self::AclkLcdc0),
            2 => Ok(Self::HclkLcdc0),
            _ => Err(ClockError::UnknownClock),
        }
    }
}

pub struct Rk3126Cru {
    regs: Mmio,
}

unsafe impl Sync for Rk3126Cru {}

impl Rk3126Cru {
    unsafe fn new(base: usize) -> Self {
        Self {
            regs: unsafe { Mmio::new(base) },
        }
    }

    /// Performs a hiword-masked write: updates only the bits covered by
    /// `mask` (already positioned at `shift`) to `value`, leaving every
    /// other bit in the register untouched, in one atomic store.
    fn write_masked(&self, offset: usize, shift: u32, mask: u32, value: u32) {
        let field = (value & mask) << shift;
        let write_mask = mask << (shift + 16);
        self.regs.write32(offset, field | write_mask);
    }

    /// Reads back the rate of one of the four onboard PLLs by decoding its
    /// `FBDIV`/`REFDIV`/`POSTDIV1`/`POSTDIV2` fields.
    fn pll_rate_hz(&self, base: usize) -> Result<u32, ClockError> {
        let con0 = self.regs.read32(base + pll_con(0));
        let con1 = self.regs.read32(base + pll_con(1));

        let dsmpd = (con1 >> PLLCON1_DSMPD_SHIFT) & 0x01;

        if dsmpd == 0 {
            // Fractional mode: FRAC (PLLCON2) would need to factor in too.
            // No boot path on this platform is expected to leave a PLL in
            // fractional mode, so treat it as unsupported rather than
            // silently mis-reporting the rate.
            return Err(ClockError::UnreachableRate);
        };

        let fbdiv = con0 & PLLCON0_FBDIV_MASK;
        let postdiv1 = (con0 >> PLLCON0_POSTDIV1_SHIFT) & PLLCON0_POSTDIV1_MASK;
        let refdiv = con1 & PLLCON1_REFDIV_MASK;
        let postdiv2 = (con1 >> PLLCON1_POSTDIV2_SHIFT) & PLLCON1_POSTDIV2_MASK;

        if refdiv == 0 || postdiv1 == 0 || postdiv2 == 0 {
            return Err(ClockError::ParentUnavailable);
        }

        // vco = OSC_HZ * fbdiv / refdiv; output = vco / postdiv1 / postdiv2.
        // Widen to u64 for the multiply so a large fbdiv can't overflow.
        let vco_hz = (OSC_HZ as u64) * (fbdiv as u64) / (refdiv as u64);
        let output_hz = vco_hz / (postdiv1 as u64) / (postdiv2 as u64);

        u32::try_from(output_hz).map_err(|_| ClockError::UnreachableRate)
    }

    /// Returns the rate of the parent
    fn vop_parent_rate_hz(&self, parent: VopClockParent) -> Result<u32, ClockError> {
        match parent {
            VopClockParent::Cpll => self.pll_rate_hz(pll_base::CPLL),
            VopClockParent::Gpll => self.pll_rate_hz(pll_base::GPLL),
            VopClockParent::GpllDiv2 => self.pll_rate_hz(pll_base::GPLL).map(|hz| hz / 2),
            VopClockParent::GpllDiv3 => self.pll_rate_hz(pll_base::GPLL).map(|hz| hz / 3),
        }
    }

    /// Picks the parent/divider pair for `DCLK_VOP` that reaches closest to
    /// `hz` without exceeding it, preferring CPLL first (matching the
    /// reference driver's declared parent order and keeping GPLL free for
    /// other peripherals that depend on it).
    fn best_dclk_vop_divider(&self, hz: u32) -> Result<(VopClockParent, u32, u32), ClockError> {
        const CANDIDATES: [VopClockParent; 4] = [
            VopClockParent::Cpll,
            VopClockParent::Gpll,
            VopClockParent::GpllDiv2,
            VopClockParent::GpllDiv3,
        ];

        let mut best: Option<(VopClockParent, u32, u32)> = None; // (parent, divider, achieved_hz)

        for parent in CANDIDATES {
            let Ok(parent_hz) = self.vop_parent_rate_hz(parent) else {
                continue;
            };
            if parent_hz == 0 {
                continue;
            }

            // 8-bit divider field means the true divisor is div_field + 1,
            // in [1, 256].
            let divisor = (parent_hz / hz).max(1).min(256);
            let achieved_hz = parent_hz / divisor;

            let is_better = match best {
                None => true,
                Some((_, _, best_hz)) => {
                    // Prefer the candidate that doesn't exceed the target if
                    // one exists, then the one closest to it.
                    let candidate_over = achieved_hz > hz;
                    let best_over = best_hz > hz;
                    match (candidate_over, best_over) {
                        (false, true) => true,
                        (true, false) => false,
                        _ => achieved_hz.abs_diff(hz) < best_hz.abs_diff(hz),
                    }
                }
            };

            if is_better {
                best = Some((parent, divisor, achieved_hz));
            }
        }

        best.ok_or(ClockError::ParentUnavailable)
    }
}

impl ClockController for Rk3126Cru {
    fn enable(&self, clock: u32) -> Result<(), ClockError> {
        let clock = Rk3126Clock::try_from(clock)?;
        match clock {
            Rk3126Clock::DclkVop => {
                // GFLAGS on this family is CLK_GATE_SET_TO_DISABLE: 0 ungates.
                self.write_masked(dclk_vop::GATE_CON, dclk_vop::GATE_BIT, 0x1, 0);
            }
            Rk3126Clock::AclkLcdc0 => {
                self.write_masked(ACLK_LCDC0_GATE_CON, ACLK_LCDC0_GATE_BIT, 0x1, 0);
            }
            Rk3126Clock::HclkLcdc0 => {
                self.write_masked(HCLK_LCDC0_GATE_CON, HCLK_LCDC0_GATE_BIT, 0x1, 0);
            }
        }
        Ok(())
    }

    fn disable(&self, clock: u32) -> Result<(), ClockError> {
        let clock = Rk3126Clock::try_from(clock)?;
        match clock {
            Rk3126Clock::DclkVop => {
                self.write_masked(dclk_vop::GATE_CON, dclk_vop::GATE_BIT, 0x1, 1);
            }
            Rk3126Clock::AclkLcdc0 => {
                self.write_masked(ACLK_LCDC0_GATE_CON, ACLK_LCDC0_GATE_BIT, 0x1, 1);
            }
            Rk3126Clock::HclkLcdc0 => {
                self.write_masked(HCLK_LCDC0_GATE_CON, HCLK_LCDC0_GATE_BIT, 0x1, 1);
            }
        }
        Ok(())
    }

    fn set_rate(&self, clock: u32, hz: u32) -> Result<u32, ClockError> {
        let clock = Rk3126Clock::try_from(clock)?;
        match clock {
            Rk3126Clock::DclkVop => {
                if hz == 0 {
                    return Err(ClockError::UnreachableRate);
                }
                let (parent, divisor, achieved_hz) = self.best_dclk_vop_divider(hz)?;

                self.write_masked(
                    dclk_vop::SEL_CON,
                    dclk_vop::MUX_SHIFT,
                    dclk_vop::MUX_MASK,
                    parent as u32,
                );
                // Register field stores divisor - 1 (divide-by-(n+1)).
                self.write_masked(
                    dclk_vop::SEL_CON,
                    dclk_vop::DIV_SHIFT,
                    dclk_vop::DIV_MASK,
                    divisor - 1,
                );

                Ok(achieved_hz)
            }
            // ACLK_LCDC0/HCLK_LCDC0 derive from shared bus clocks
            // (aclk_vio0/hclk_vio) this driver doesn't independently divide;
            // their rate is whatever that bus is already running at.
            Rk3126Clock::AclkLcdc0 | Rk3126Clock::HclkLcdc0 => Err(ClockError::UnreachableRate),
        }
    }

    fn get_rate(&self, clock: u32) -> Result<u32, ClockError> {
        let clock = Rk3126Clock::try_from(clock)?;
        match clock {
            Rk3126Clock::DclkVop => {
                let con = self.regs.read32(dclk_vop::SEL_CON);
                let mux = (con >> dclk_vop::MUX_SHIFT) & dclk_vop::MUX_MASK;
                let divisor = ((con >> dclk_vop::DIV_SHIFT) & dclk_vop::DIV_MASK) + 1;

                let parent = match mux {
                    0 => VopClockParent::Cpll,
                    1 => VopClockParent::Gpll,
                    2 => VopClockParent::GpllDiv2,
                    _ => VopClockParent::GpllDiv3,
                };

                self.vop_parent_rate_hz(parent).map(|hz| hz / divisor)
            }
            Rk3126Clock::AclkLcdc0 | Rk3126Clock::HclkLcdc0 => Err(ClockError::UnreachableRate),
        }
    }
}

fn init_rk3126_cru(base: usize) -> &'static dyn ClockController {
    static INSTANCE: StaticCell<Rk3126Cru> = StaticCell::uninit();
    // SAFETY: `base` came from the validated, mapped device tree via
    // `probe_clocks`.
    let cru = unsafe { Rk3126Cru::new(base) };
    INSTANCE.get_or_init(|| cru)
}

register_clock_driver!(RK3126_CRU, "rockchip,rk3126-cru", init_rk3126_cru);