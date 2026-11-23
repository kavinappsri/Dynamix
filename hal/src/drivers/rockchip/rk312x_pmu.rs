//! RK3128, RK3126 Power Management Unit (PMU) driver
//!
//! Referenced from the corresponding driver for linux and the rk3128 TRM (ch14)

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::device::{Device, Resource};
use crate::services::mmio::Mmio;
use crate::services::provider;
use crate::services::timer::delay_ms;

/// The PMU's `compatible` string, which is also the id of the device that
/// [`obtain`] spawns when the board's tree does not describe one.
const COMPATIBLE: &str = "rockchip,rk3128-pmu";

/// Where the PMU's registers are on RK3126/RK3128, for boards whose device
/// tree leaves it out. A tree that has the node is believed over this.
const FALLBACK_BASE: usize = 0x2000_4000;
const FALLBACK_SIZE: usize = 0x100;

/// Why a power domain request failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerDomainError {
    /// The controller gave up waiting for a status/ack bit to reflect the
    /// requested change.
    Timeout,
}

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

pub struct Rk3126Pmu {
    regs: Mmio,
}

impl Rk3126Pmu {
    fn new(regs: Mmio) -> Self {
        Self { regs }
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

impl Rk3126Pmu {
    /// Powers a domain on, blocking until the controller confirms it.
    pub fn power_on(&self, domain: Rk3126PowerDomain) -> Result<(), PowerDomainError> {
        let bits = domain.bits();

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

    /// Powers a domain off, blocking until the controller confirms it.
    pub fn power_off(&self, domain: Rk3126PowerDomain) -> Result<(), PowerDomainError> {
        let bits = domain.bits();

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

    /// Whether `domain` is powered on now.
    pub fn is_powered_on(&self, domain: Rk3126PowerDomain) -> bool {
        let bits = domain.bits();
        let powered_down = (self.regs.read32(PMU_PWRDN_ST) >> bits.pwr_bit) & 1 != 0;
        !powered_down
    }
}

/// The PMU, for a driver that needs it.
pub fn obtain(consumer: &'static Device) -> Result<&'static Rk3126Pmu, ProbeError> {
    let registers = Resource::Mem(PhysRange::new(PhysAddr::new(FALLBACK_BASE), FALLBACK_SIZE));
    let device = consumer.spawn_once("rk3126-pmu", &[COMPATIBLE], &[registers]);
    provider::get(device)
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let regs = dev.mem(0)?.mmio();
    provider::register(dev, Rk3126Pmu::new(regs));
    Ok(())
}

driver! {
    name: "rk3126-pmu",
    matches: [Match::Fdt(COMPATIBLE), Match::Spawned(COMPATIBLE)],
    probe: probe,
}
