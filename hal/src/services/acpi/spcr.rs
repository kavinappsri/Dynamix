//! The SPCR (Serial Port Console Redirection table): which serial port the
//! firmware itself uses as its console, and so which one we should too.

use core::mem::size_of;

use acpica_sys as sys;

use super::AcpiError;
use super::table::{Table, TableKind, read_pod};

struct SpcrKind;

impl TableKind for SpcrKind {
    const SIGNATURE: &'static [u8; 5] = sys::ACPI_SIG_SPCR;
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialAddress {
    /// Memory mapped registers at this physical address.
    Memory(u64),
    /// x86 I/O ports starting here.
    Io(u16),
}

/// ACPI generic address space ids (ACPICA does not bind these macros).
const SPACE_SYSTEM_MEMORY: u8 = 0;
const SPACE_SYSTEM_IO: u8 = 1;

/// Fixed part of the table ahead of the port's address: the header, the
/// interface type and three reserved bytes.
const PORT_OFFSET: usize = size_of::<sys::ACPI_TABLE_HEADER>() + 4;

/// Reads where the SPCR says the console serial port is.
pub(super) fn console_serial() -> Result<SerialAddress, AcpiError> {
    let table = Table::<SpcrKind>::get(1)?;
    let port: sys::ACPI_GENERIC_ADDRESS = read_pod(table.bytes(), PORT_OFFSET).ok_or(AcpiError::BadParameter)?;

    let space = port.SpaceId;
    let address = port.Address;
    match space {
        SPACE_SYSTEM_MEMORY => Ok(SerialAddress::Memory(address)),
        SPACE_SYSTEM_IO => u16::try_from(address).map(SerialAddress::Io).map_err(|_| AcpiError::BadParameter),
        // PCI configuration space and the like name a device, not registers.
        _ => Err(AcpiError::NotImplemented),
    }
}
