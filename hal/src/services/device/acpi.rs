//! Devices from the ACPI namespace.
//!
//! Every present device with a `_HID` or `_CID` becomes a device. Their
//! resources are not read here: `_CRS` is AML, and running it for every
//! device on a machine would be slow and would expose every firmware bug in
//! the namespace, so it only runs for devices a driver actually wants (see
//! [`Device::resources`](super::Device::resources)).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Device, IoPorts, IrqSpec, Origin, Resource, register};
use crate::driver_traits::interrupt_controller::Irq;
use crate::services::acpi::{
    AcpiError, AcpiRoot, AddressSpace, Device as AcpiDevice, MadtEntry, Resource as AcpiResource,
};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::boot_info::HardwareError;

pub(super) fn enumerate(root: &AcpiRoot) -> Result<(), HardwareError> {
    let acpi = root.acpi().ok_or(AcpiError::NotReady)?;
    let namespace = acpi.namespace()?;

    for device in namespace.all_devices()? {
        // A device whose description cannot be read is left out, not fatal.
        let Ok(info) = device.info() else { continue };

        let mut ids: Vec<String> = Vec::new();
        ids.extend(info.hardware_id);
        ids.extend(info.compatible_ids);
        let Some(first) = ids.first() else { continue };

        let name = match &info.unique_id {
            Some(unique) => format!("{first}:{unique}"),
            None => first.clone(),
        };
        register(Device::new(name, ids, Origin::Acpi(device), None));
    }

    // Hardware ACPI describes in tables, not in the namespace. The MADT only
    // counts when this machine has one; the FADT is mandatory.
    if let Ok(madt) = acpi.madt() {
        // The I/O APICs' register windows. The local APIC's is left out on
        // purpose: its base lives in an MSR the CPU can relocate, and that
        // is what the APIC driver goes by.
        let windows = madt
            .entries()
            .filter_map(|entry| match entry {
                MadtEntry::IoApic { address, .. } => {
                    Some(Resource::Mem(PhysRange::new(PhysAddr::new(address as usize), IOAPIC_WINDOW)))
                }
                _ => None,
            })
            .collect();
        register_table("APIC", windows);
    }
    register_table("FACP", Vec::new());
    Ok(())
}

/// Size of an I/O APIC's register window.
const IOAPIC_WINDOW: usize = 0x1000;

fn register_table(signature: &str, resources: Vec<Resource>) {
    register(Device::new(
        String::from(signature),
        alloc::vec![String::from(signature)],
        Origin::AcpiTable,
        Some(resources),
    ));
}

/// Evaluates the device's `_CRS` into resources.
pub(super) fn load_resources(device: &AcpiDevice) -> Result<Vec<Resource>, AcpiError> {
    let described = match device.resources() {
        Ok(list) => list,
        // No `_CRS`: the device uses nothing.
        Err(AcpiError::NotFound) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut resources = Vec::new();
    for resource in described {
        match resource {
            AcpiResource::Io { min, length, .. } if length > 0 => {
                resources.push(Resource::Io(IoPorts::new(min, u16::from(length))));
            }
            AcpiResource::FixedIo { base, length } if length > 0 => {
                resources.push(Resource::Io(IoPorts::new(base, u16::from(length))));
            }
            AcpiResource::Memory { min, length, .. } if length > 0 => {
                resources.push(Resource::Mem(memory(min, length)));
            }
            // Windows the device hands to its children (bridges) are not the
            // device's own registers.
            AcpiResource::Address { space: AddressSpace::Memory, produces: false, min, length, .. } if length > 0 => {
                resources.push(Resource::Mem(memory(min, length)));
            }
            AcpiResource::Address { space: AddressSpace::Io, produces: false, min, length, .. } if length > 0 => {
                if let (Ok(base), Ok(count)) = (u16::try_from(min), u16::try_from(length)) {
                    resources.push(Resource::Io(IoPorts::new(base, count)));
                }
            }
            AcpiResource::Irq { lines, trigger, polarity, .. } => {
                for line in lines {
                    resources.push(Resource::Irq(IrqSpec { irq: Irq(line), trigger, polarity }));
                }
            }
            _ => {}
        }
    }
    Ok(resources)
}

fn memory(base: u64, length: u64) -> PhysRange {
    PhysRange::new(PhysAddr::new(base as usize), length as usize)
}
