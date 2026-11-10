//! ARM Generic Interrupt Controller v2 (GIC-400, Cortex-A15/A7 GIC).
//!
//! Interrupt ids: 0-15 are SGIs (software generated, used here for
//! self/cross-core IPIs), 16-31 PPIs (per-core: generic timer, ...), 32+ SPIs
//! (shared peripherals, i.e. most devices).

use crate::driver_traits::driver::{Driver, DriverProbeError};
use crate::driver_traits::interrupt_controller::{
    Claim, CpuId, InterruptController, Irq, IrqError, Polarity, Trigger,
};
use crate::register_driver;
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::device_query::{DeviceQuery, DeviceResource};
use crate::services::mmio::Mmio;
use crate::services::mmu;
use crate::services::sync::{IrqSpinLock, StaticCell};

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

/// `compatible` strings of GICv2 implementations.
const COMPATIBLES: [&str; 3] = ["arm,cortex-a15-gic", "arm,cortex-a7-gic", "arm,gic-400"];

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
        ((n as u32) < SGI_COUNT).then(|| Irq(n as u32))
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

/// Reads cell number `index` (each `cells` 32-bit words, big endian) of a
/// device tree property.
fn read_cells(bytes: &[u8], index: usize, cells: usize) -> Option<u64> {
    let start = index * cells * 4;
    let mut value = 0u64;
    for word in 0..cells {
        let at = start + word * 4;
        let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        value = value << 32 | u32::from_be_bytes(raw) as u64;
    }
    Some(value)
}

fn init_gic_v2(
    resource: DeviceResource,
    tree: &dyn DeviceQuery,
) -> Result<&'static dyn InterruptController, DriverProbeError> {
    static INSTANCE: StaticCell<GicV2> = StaticCell::uninit();

    let distributor_base = resource.mmio_base().ok_or(DriverProbeError::WrongResource)?;

    // The probe only knows the first `reg` entry (the distributor); the CPU
    // interface is the second. The parent's #address-cells / #size-cells are
    // not available here (2/2 on QEMU's virt even for 32-bit, 1/1 on most
    // 32-bit SoCs), so find which one makes the first entry the distributor
    // address the probe already resolved.
    let dtb = tree.as_dtb().ok_or(DriverProbeError::WrongResource)?;
    let node = COMPATIBLES
        .iter()
        .find_map(|compatible| dtb.find_compatible(compatible))
        .ok_or(DriverProbeError::NoCompatibleDriver)?;
    let reg = node.property("reg").ok_or(DriverProbeError::WrongResource)?;
    let reg = reg.value();

    let (cpu_base, cpu_size) = [2usize, 1]
        .into_iter()
        .find_map(|cells| {
            // Entries are (address, size) pairs, `cells` words each.
            if read_cells(reg, 0, cells)? != distributor_base as u64 {
                return None;
            }
            Some((read_cells(reg, 2, cells)?, read_cells(reg, 3, cells)?))
        })
        .ok_or(DriverProbeError::WrongResource)?;

    mmu::map_device(PhysRange::new(PhysAddr::new(cpu_base as usize), cpu_size as usize))
        .map_err(|_| DriverProbeError::MappingFailed)?;

    // SAFETY: both windows were mapped as device memory just above / by the probe.
    let (distributor, cpu_interface) = unsafe { (Mmio::new(distributor_base), Mmio::new(cpu_base as usize)) };

    let typer = distributor.read32(GICD_TYPER);
    let irq_count = (((typer & 0x1F) + 1) * 32).min(FIRST_SPECIAL_ID);

    let gic = INSTANCE.get_or_init(|| GicV2 {
        distributor,
        cpu_interface,
        irq_count,
        rmw: IrqSpinLock::new(()),
    });
    gic.init_distributor();
    Ok(gic)
}

register_driver!(GIC_V2_CORTEX_A15, "arm,cortex-a15-gic", init_gic_v2, InterruptController);
register_driver!(GIC_V2_CORTEX_A7, "arm,cortex-a7-gic", init_gic_v2, InterruptController);
register_driver!(GIC_V2_GIC_400, "arm,gic-400", init_gic_v2, InterruptController);
