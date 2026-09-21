//! RK3128, RK3126 Power Management Unit (PMU) driver
//!
//! Referenced from the corresponding driver for linux and the rk3128 TRM (ch14)

use crate::driver_traits::driver::Driver;
use crate::driver_traits::driver::DriverProbeError;
use crate::driver_traits::power_domain::{PowerDomainController, PowerDomainError};
use crate::register_driver;
use crate::services::dtb::Dtb;
use crate::services::mmio::Mmio;
use crate::services::sync::StaticCell;
use crate::services::timer::delay_ms;

const PMU_PWRDN_CON: usize = 0x04;
const PMU_PWRDN_ST: usize = 0x08;
const PMU_IDLE_REQ: usize = 0x0c;
const PMU_IDLE_ST: usize = 0x10;

/// Per-poll sleep while waiting on a status/ack bit
const POLL_STEP_MS: u64 = 1;
/// Number of `POLL_STEP_MS` steps before giving up.
const POLL_MAX_ITERS: u32 = 100;

struct DomainBits {
    /// Bit in `PMU_PWRDN_CON` / `PMU_PWRDN_ST`
    pwr_bit: u32,
    /// Bit in `PMU_IDLE_REQ`, and the "finish" bit at the same position in
    /// `PMU_IDLE_ST`
    idle_bit: u32,
}

/// Power domain identifiers this driver currently recognizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Rk3126PowerDomain {
    /// Video I/O: VOP, VIP, IEP, RGA, EBC, MIPI-DSI, HDMI.
    Vio = 0,
    /// 3D GPU.
    Gpu = 1,
    /// Video encode/decode (VEPU/VDPU).
    Video = 2,
}

impl Rk3126PowerDomain {
    /// `pwr_bit` per `PMU_PWRDN_CON`/`PMU_PWRDN_ST`
    const fn bits(self) -> DomainBits {
        match self {
            Self::Vio => DomainBits { pwr_bit: 3, idle_bit: 2 },
            Self::Gpu => DomainBits { pwr_bit: 1, idle_bit: 3 },
            Self::Video => DomainBits { pwr_bit: 2, idle_bit: 1 },
        }
    }
}

impl TryFrom<u32> for Rk3126PowerDomain {
    type Error = PowerDomainError;

    fn try_from(value: u32) -> Result<Self, PowerDomainError> {
        match value {
            0 => Ok(Self::Vio),
            1 => Ok(Self::Gpu),
            2 => Ok(Self::Video),
            _ => Err(PowerDomainError::UnknownDomain),
        }
    }
}

pub struct Rk3126Pmu {
    regs: Mmio,
}

unsafe impl Sync for Rk3126Pmu {}

impl Rk3126Pmu {
    unsafe fn new(base: usize) -> Self {
        Self {
            regs: unsafe { Mmio::new(base) },
        }
    }

    /// Sets or clears a single bit at `offset`, read-modify-write
    fn set_bit(&self, offset: usize, bit: u32, set: bool) {
        let current = self.regs.read32(offset);
        let updated = if set {
            current | (1 << bit)
        } else {
            current & !(1 << bit)
        };
        self.regs.write32(offset, updated);
    }

    /// Polls `offset` until `(value >> bit) & 1 == want`, sleeping
    /// `POLL_STEP_MS` between attempts, up to `POLL_MAX_ITERS` times.
    fn poll_bit(&self, offset: usize, bit: u32, want: bool) -> Result<(), PowerDomainError> {
        for _ in 0..POLL_MAX_ITERS {
            let bit_set = (self.regs.read32(offset) >> bit) & 1 != 0;
            if bit_set == want {
                return Ok(());
            }
            delay_ms(POLL_STEP_MS);
        }
        Err(PowerDomainError::Timeout)
    }
}

impl PowerDomainController for Rk3126Pmu {
    fn power_on(&self, domain: u32) -> Result<(), PowerDomainError> {
        let bits = Rk3126PowerDomain::try_from(domain)?.bits();

        // Clear the power-down bit
        self.set_bit(PMU_PWRDN_CON, bits.pwr_bit, false);
        self.poll_bit(PMU_PWRDN_ST, bits.pwr_bit, false)?;

        // de-assert the idle request
        // and wait for both the finish and ack bits to clear.
        self.set_bit(PMU_IDLE_REQ, bits.idle_bit, false);
        self.poll_bit(PMU_IDLE_ST, bits.idle_bit, false)?;
        self.poll_bit(PMU_IDLE_ST, bits.idle_bit + 16, false)?;

        Ok(())
    }

    fn power_off(&self, domain: u32) -> Result<(), PowerDomainError> {
        let bits = Rk3126PowerDomain::try_from(domain)?.bits();

        // Ask the domain's NIU to flush in-flight transactions before
        // cutting power, and wait for it to confirm
        self.set_bit(PMU_IDLE_REQ, bits.idle_bit, true);
        self.poll_bit(PMU_IDLE_ST, bits.idle_bit, true)?;
        self.poll_bit(PMU_IDLE_ST, bits.idle_bit + 16, true)?;

        // Assert the power-down bit and wait for the status bit to follow.
        self.set_bit(PMU_PWRDN_CON, bits.pwr_bit, true);
        self.poll_bit(PMU_PWRDN_ST, bits.pwr_bit, true)?;

        Ok(())
    }

    fn is_powered_on(&self, domain: u32) -> Result<bool, PowerDomainError> {
        let bits = Rk3126PowerDomain::try_from(domain)?.bits();
        let powered_down = (self.regs.read32(PMU_PWRDN_ST) >> bits.pwr_bit) & 1 != 0;
        Ok(!powered_down)
    }
}

fn init_rk3126_pmu(base: usize, _tree: &Dtb) -> Result<&'static dyn PowerDomainController, DriverProbeError> {
    static INSTANCE: StaticCell<Rk3126Pmu> = StaticCell::uninit();
    let pmu = unsafe { Rk3126Pmu::new(base) };
    Ok(INSTANCE.get_or_init(|| pmu))
}

register_driver!(RK3126_PMU, "rockchip,rk3128-pmu", init_rk3126_pmu, PowerDomainController);
