//! Devices from ACPI namespace

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::driver_traits::interrupt_controller::{Irq, IrqSpec};
use crate::services::acpi::{AcpiError, AcpiRoot, AddressSpace, Device as AcpiDevice, Resource as AcpiResource};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::boot_info::HardwareError;
use crate::services::device::{register, Device, Origin, Resource, IoPorts};

pub(super) fn enumerate(root: &AcpiRoot) -> Result<(), HardwareError> {
    let acpi = root.acpi().ok_or(AcpiError::NotReady)?;
    let namespace = acpi.namespace()?;

    for device in namespace.all_devices()? {
        // leave if device info error
        let Ok(info) = device.info() else { continue };

        let mut ids: Vec<String> = Vec::new();
        ids.extend(info.hardware_id);
        ids.extend(info.compatible_ids);
        let Some(first) = ids.first() else { continue };

        let name = match &info.unique_id {
            Some(unique) => format!("{first}:{unique}"),
            None => first.clone()
        };
        register(Device::new(name, ids, Origin::Acpi(device), None));
    }
    if acpi.madt().is_ok() {
        register_table("APIC")
    }
    register_table("FACP");

    Ok(())
}

fn register_table(signature: &str) {
    register(Device::new(String::from(signature), alloc::vec![String::from(signature)], Origin::AcpiTable, Some(Vec::new())));
}

pub(super) fn load_resources(device: &AcpiDevice) -> Result<Vec<Resource>, AcpiError> {
    let described = match device.resources() {
        Ok(list) => list,
        Err(AcpiError::NotFound) => Vec::new(),
        Err(error) => return Err(error),
    };

    let mut resources = Vec::new();
    for resource in described {
        match resource {
            AcpiResource::Io { min, length, .. } if length > 0 => {
                resources.push(Resource::Io(IoPorts::new(min, u16::from(length))))
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