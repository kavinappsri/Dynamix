//! ARM Generic Interrupt Controller v2 (GIC-400, Cortex-A15/A7 GIC).
//!
//! Interrupt ids: 0-15 are SGIs (software generated, used here for
//! self/cross-core IPIs), 16-31 PPIs (per-core: generic timer, ...), 32+ SPIs
//! (shared peripherals, i.e. most devices).

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::interrupt_controller::{
    self, Claim, CpuId, InterruptController, Irq, IrqError, IrqSpec, Polarity, Trigger,
};
use crate::services::device::Device;
use crate::services::mmio::Mmio;
use crate::services::sync::IrqSpinLock;

// Distributor registers
const GICD_CTLR: usize = 0x000;
const GICD_TYPER: usize = 0x004;
const GICD_ISENABLER: usize = 0x100;
const GICD_ICENABLER: usize = 0x180;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;
const GICD_ICFGR: usize = 0xC00;
const GICD_SGIR: usize = 0xF00;

// CPU interface registers
const GICC_CTLR: usize = 0x00;
const GICC_PMR: usize = 0x04;
const GICC_BPR: usize = 0x08;
const GICC_IAR: usize = 0x0C;
const GICC_EOIR: usize = 0x10;

/// Interrupt ids at and above this are special (spurious).
const FIRST_SPECIAL_ID: u32 = 1020;
const SGI_COUNT: u32 = 16;
const FIRST_SPI: u32 = 32;
/// Default priority for everything: middle of the road, below the PMR mask.
const DEFAULT_PRIORITY: u32 = 0xA0;
const PRIORITY_MASK: u32 = 0xF0;

/// A GICv2: one distributor, plus a banked per-core CPU interface.
pub struct GicV2 {
    distributor: Mmio,
    cpu_interface: Mmio,
    irq_count: u32,
    /// Serialises the read-modify-write registers (config, targets).
    rmw: IrqSpinLock<()>,
}

impl GicV2 {
    fn check(&self, irq: Irq) -> Result<u32, IrqError> {
        if irq.0 >= self.irq_count {
            Err(IrqError::OutOfRange)
        } else {
            Ok(irq.0)
        }
    }

    /// Brings the distributor up with everything masked and routed to the
    /// calling core.
    fn init_distributor(&self) {
        let d = self.distributor;
        d.write32(GICD_CTLR, 0);

        // SPIs: default priority, targeted at the calling core, level
        // triggered, disabled. (ITARGETSR0 reads back the calling core's mask.)
        let target = d.read32(GICD_ITARGETSR) & 0xFF;
        let target_word = target * 0x0101_0101;
        let mut id = FIRST_SPI;
        while id < self.irq_count {
            d.write32(GICD_IPRIORITYR + id as usize, DEFAULT_PRIORITY * 0x0101_0101);
            d.write32(GICD_ITARGETSR + id as usize, target_word);
            id += 4;
        }
        let mut id = FIRST_SPI;
        while id < self.irq_count {
            d.write32(GICD_ICFGR + id as usize / 4, 0);
            id += 16;
        }
        let mut id = FIRST_SPI;
        while id < self.irq_count {
            d.write32(GICD_ICENABLER + (id as usize / 32) * 4, u32::MAX);
            id += 32;
        }

        d.write32(GICD_CTLR, 1);
    }
}

impl InterruptController for GicV2 {
    fn irq_count(&self) -> u32 {
        self.irq_count
    }

    fn init_this_cpu(&self) -> Result<(), IrqError> {
        let d = self.distributor;
        // The SGI/PPI registers are banked per core: mask them all and set
        // priorities for this core.
        d.write32(GICD_ICENABLER, u32::MAX);
        for word in 0..(FIRST_SPI as usize / 4) {
            d.write32(GICD_IPRIORITYR + word * 4, DEFAULT_PRIORITY * 0x0101_0101);
        }

        let c = self.cpu_interface;
        c.write32(GICC_PMR, PRIORITY_MASK);
        c.write32(GICC_BPR, 0);
        c.write32(GICC_CTLR, 1);
        Ok(())
    }

    fn enable(&self, irq: Irq) -> Result<(), IrqError> {
        let id = self.check(irq)?;
        // Write-1-to-set: no read-modify-write, no lock needed.
        self.distributor.write32(GICD_ISENABLER + (id as usize / 32) * 4, 1 << (id % 32));
        Ok(())
    }

    fn disable(&self, irq: Irq) -> Result<(), IrqError> {
        let id = self.check(irq)?;
        self.distributor.write32(GICD_ICENABLER + (id as usize / 32) * 4, 1 << (id % 32));
        Ok(())
    }

    fn configure(&self, irq: Irq, trigger: Trigger, polarity: Polarity) -> Result<(), IrqError> {
        let id = self.check(irq)?;
        // SGIs are fixed edge. The GIC only does active-high / rising-edge:
        // an inverted line has to be fixed at its source.
        if id < SGI_COUNT || polarity == Polarity::ActiveLow {
            return Err(IrqError::Unsupported);
        }

        let _guard = self.rmw.lock();
        let reg = GICD_ICFGR + (id as usize / 16) * 4;
        let shift = (id % 16) * 2;
        let mut value = self.distributor.read32(reg) & !(0b11 << shift);
        if trigger == Trigger::Edge {
            value |= 0b10 << shift;
        }
        self.distributor.write32(reg, value);
        Ok(())
    }

    /// `<type number flags>`: type 0 is a shared peripheral interrupt (SPI,
    /// numbered from 32), type 1 a per-core one (PPI, from 16). The low four
    /// bits of `flags` give the signalling; the rest (a PPI's core mask) are
    /// not a property of the line.
    fn translate(&self, cells: &[u32]) -> Result<IrqSpec, IrqError> {
        let &[kind, number, flags] = cells else {
            return Err(IrqError::Unsupported);
        };
        let id = match kind {
            0 => number.checked_add(FIRST_SPI),
            1 => number.checked_add(SGI_COUNT),
            _ => None,
        }
            .ok_or(IrqError::Unsupported)?;
        let irq = Irq(id);
        self.check(irq)?;

        let (trigger, polarity) = match flags & 0xF {
            0b0001 => (Trigger::Edge, Polarity::ActiveHigh),
            0b0010 => (Trigger::Edge, Polarity::ActiveLow),
            0b0100 => (Trigger::Level, Polarity::ActiveHigh),
            0b1000 => (Trigger::Level, Polarity::ActiveLow),
            _ => return Err(IrqError::Unsupported),
        };
        Ok(IrqSpec { irq, trigger, polarity })
    }

    fn set_affinity(&self, irq: Irq, cpu: CpuId) -> Result<(), IrqError> {
        let id = self.check(irq)?;
        // Only SPIs are routable; the GICv2 target list is 8 cores wide.
        if id < FIRST_SPI || cpu.0 >= 8 {
            return Err(IrqError::Unsupported);
        }

        let _guard = self.rmw.lock();
        let reg = GICD_ITARGETSR + (id as usize & !3);
        let shift = (id & 3) * 8;
        let value = self.distributor.read32(reg) & !(0xFF << shift) | (1 << cpu.0) << shift;
        self.distributor.write32(reg, value);
        Ok(())
    }

    fn acknowledge(&self, _cpu_vector: Option<u32>) -> Option<Claim> {
        let iar = self.cpu_interface.read32(GICC_IAR);
        let id = iar & 0x3FF;
        // 1022/1023: nothing pending (spurious); no EOI for these.
        (id < FIRST_SPECIAL_ID).then_some(Claim { irq: Irq(id), raw: iar })
    }

    fn end_of_interrupt(&self, claim: Claim) {
        // Needs the whole IAR value: for an SGI it carries the sending core.
        self.cpu_interface.write32(GICC_EOIR, claim.raw);
    }

    fn software_irq(&self, n: u8) -> Option<Irq> {
        ((n as u32) < SGI_COUNT).then_some(Irq(n as u32))
    }

    fn raise_self(&self, irq: Irq) -> Result<(), IrqError> {
        if irq.0 >= SGI_COUNT {
            return Err(IrqError::Unsupported);
        }
        // TargetListFilter 0b10: only the requesting core.
        self.distributor.write32(GICD_SGIR, (0b10 << 24) | irq.0);
        Ok(())
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    // `reg` lists the distributor first and the CPU interface second.
    let distributor = dev.mem(0)?.mmio();
    let cpu_interface = dev.mem(1)?.mmio();

    let typer = distributor.read32(GICD_TYPER);
    let irq_count = (((typer & 0x1F) + 1) * 32).min(FIRST_SPECIAL_ID);

    let gic = GicV2 { distributor, cpu_interface, irq_count, rmw: IrqSpinLock::new(()) };
    gic.init_distributor();
    interrupt_controller::register(dev, gic);
    Ok(())
}

driver! {
    name: "gic-v2",
    matches: [
        Match::Fdt("arm,cortex-a15-gic"),
        Match::Fdt("arm,cortex-a7-gic"),
        Match::Fdt("arm,gic-400"),
    ],
    probe: probe,
}
