//! Devices from a device tree.
//!
//! Every enabled node with a `compatible` property becomes a device, whether
//! or not a driver exists for it. Its `reg` entries become memory windows,
//! translated through the `ranges` of every bus above it. Nodes on buses that
//! do not map their children into CPU memory (I2C, SPI) get no windows, since
//! their `reg` is a bus address, not a memory one.

use alloc::string::String;
use alloc::vec::Vec;

use super::{Device, FdtOrigin, Origin, Resource, devices, register};
use crate::driver_traits::driver::ProbeError;
use crate::driver_traits::interrupt_controller::{IrqSpec, interrupt_controllers};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::dtb::Dtb;

pub(super) fn enumerate(dtb: &Dtb<'static>) {
    dtb.walk(|walked| {
        // The root's own `compatible` names the board, not a device.
        if walked.depth() == 0 || !walked.is_enabled() {
            return;
        }

        let node = walked.node();
        let ids: Vec<String> = node.compatible().map(String::from).collect();
        if ids.is_empty() {
            return;
        }

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
/// controller it names, which must be bound, and lets it decode the cells.
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
    for (index, word) in words.iter_mut().enumerate().take(cells) {
        let at = index * 4;
        *word = u32::from_be_bytes([entry[at], entry[at + 1], entry[at + 2], entry[at + 3]]);
    }

    let controller = interrupt_controllers()
        .find(|bound| bound.device.fdt().is_some_and(|candidate| candidate.offset() == parent.offset()))
        .ok_or(ProbeError::Defer)?;
    controller.handle.translate(&words[..cells]).map_err(|_| ProbeError::Firmware)
}


pub struct FdtLink {
    property: &'static str,
    cells: &'static str,
    names: &'static str,
}

impl FdtLink {
    /// `clocks = <&cru 5>` with `clock-names`.
    pub const CLOCKS: FdtLink = FdtLink { property: "clocks", cells: "#clock-cells", names: "clock-names" };
    /// `power-domains = <&pd 2>` with `power-domain-names`.
    pub const POWER_DOMAINS: FdtLink =
        FdtLink { property: "power-domains", cells: "#power-domain-cells", names: "power-domain-names" };

    /// Any other `<phandle cells...>` list a binding defines.
    pub const fn new(property: &'static str, cells: &'static str, names: &'static str) -> FdtLink {
        FdtLink { property, cells, names }
    }
}

/// Finds the device the `name`d entry of `link` points at.
pub(super) fn link(origin: &FdtOrigin, link: &FdtLink, name: &str) -> Result<&'static Device, ProbeError> {
    let node = origin.node;
    let names = node.property(link.names).ok_or(ProbeError::MissingResource)?;
    let wanted = names
        .value()
        .split(|byte| *byte == 0)
        .filter(|string| !string.is_empty())
        .position(|string| string == name.as_bytes())
        .ok_or(ProbeError::MissingResource)?;

    let entries = node.property(link.property).ok_or(ProbeError::MissingResource)?.value();
    let word = |index: usize| -> Option<u32> {
        let raw = entries.get(index * 4..index * 4 + 4)?;
        Some(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
    };

    let mut at = 0;
    for entry in 0.. {
        let phandle = word(at).ok_or(ProbeError::MissingResource)?;
        let provider = node.dtb().find_phandle(phandle).ok_or(ProbeError::Firmware)?;
        if entry == wanted {
            return devices()
                .find(|device| device.fdt().is_some_and(|candidate| candidate.offset() == provider.offset()))
                .ok_or(ProbeError::NotPresent);
        }
        let cells = provider
            .property(link.cells)
            .and_then(|prop| prop.as_u32())
            .ok_or(ProbeError::Firmware)? as usize;
        at += 1 + cells;
    }
    unreachable!("the loop only ends by returning")
}
