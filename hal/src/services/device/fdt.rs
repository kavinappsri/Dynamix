//! Devices from FDT

use alloc::string::String;
use alloc::vec::Vec;

use super::{Device, FdtOrigin, Origin, Resource, register};
use crate::driver_traits::driver::ProbeError;
use crate::driver_traits::interrupt_controller::{IrqSpec, interrupt_controllers};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::dtb::Dtb;

pub(super) fn enumerate(dtb: &Dtb<'static>) {
    dtb.walk(|walked| {
        if walked.depth() == 0 || !walked.is_enabled() {
            return;
        }

        let node = walked.node();
        let ids : Vec<String> = node.compatible().map(String::from).collect();

        let mut resources = Vec::new();
        let readable = walked.for_each_reg(|base, size| {
            if size > 0 {
                resources.push(Resource::Mem(PhysRange::new(PhysAddr::new(base as usize), size as usize)));
            }
        });
        if !readable {
            resources.clear();
        }
        register(Device::new(String::from(node.name()), ids, Origin::Fdt(FdtOrigin { node, interrupt_parent: walked.interrupt_parent() }), Some(resources)));
    });
}

/// The widest interrupt specifier read (`#interrupt-cells`).
const MAX_INTERRUPT_CELLS: usize = 8;

/// Resolves the `index`th entry of a node's `interrupts`: finds the interrupt
/// controller it names, which must be bound, and lets it decode the cells
pub(super) fn irq(origin: &FdtOrigin, index: usize) -> Result<IrqSpec, ProbeError> {
    let node = origin.node;
    let interrupts = node.property("interrupts").ok_or(ProbeError::MissingResource)?;
    let phandle = origin.interrupt_parent.ok_or(ProbeError::Firmware)?;
    let parent = node.dtb().find_phandle(phandle).ok_or(ProbeError::Firmware)?;

    let cells = parent
        .property("#interrupt-cells")
        .and_then(|prop| prop.as_u32())
        .ok_or(ProbeError::Firmware)? as usize;
    if cells == 0 || cells > MAX_INTERRUPT_CELLS {
        return Err(ProbeError::Firmware);
    }

    let entry = interrupts
        .value()
        .get(index * cells * 4..(index + 1) * cells * 4)
        .ok_or(ProbeError::MissingResource)?;
    let mut words = [0u32; MAX_INTERRUPT_CELLS];
    for (word, bytes) in words.iter_mut().zip(entry.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }

    let controller = interrupt_controllers()
        .find(|bound| bound.device.fdt().is_some_and(|candidate| candidate.offset() == parent.offset()))
        .ok_or(ProbeError::Defer)?;
    controller.handle.translate(&words[..cells]).map_err(|_| ProbeError::Firmware)
}
