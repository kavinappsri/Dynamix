//! ACPI table access for x86_64, currently backed by `acpi` crate
//!
//! Currently, this is heap-less, so no AML parsing. im reserving that stage for future polish
//! once mmu and pmm are up and working. So, currently the `alloc` / `allocator_api` are disabled
//! in the `acpi` crate
//!
//! TODO: AML Interpreter via `aml` crate


use core::ptr::NonNull;

use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::device_query::{DeviceQuery, DeviceResource};
use crate::services::mmu;

use acpi::{AcpiError, AcpiTable, AcpiTables};
use acpi::handler::{AcpiHandler, PhysicalMapping};
use acpi::sdt::SdtHeader;

#[derive(Clone, Copy)]
pub struct DynamixAcpiHandler;

impl AcpiHandler for DynamixAcpiHandler {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> PhysicalMapping<Self, T> {
        // Before the kernel's own page tables exist, firmware's identity map
        // still covers the tables, so there is nothing to map yet.
        if mmu::is_ready() {
            mmu::map_normal(PhysRange::new(PhysAddr::new(physical_address), size)).expect("failed to map an ACPI table");
        }

        unsafe {
            PhysicalMapping::new(
                physical_address,
                NonNull::new(physical_address as *mut T).expect("ACPI handed a null physical address"),
                size,
                size,
                *self,
            )
        }
    }

    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {
        // Identity mapping owns nothing that needs releasing: there is no
        // separate virtual window allocated per `map_physical_region` call
        // to give back.
    }
}

/// Errors that can occur while locating or reading the ACPI static tables.
#[derive(Debug)]
pub enum AcpiQueryError {
    /// The `acpi` crate rejected the RSDP or a table
    Acpi(AcpiError),
}

impl From<AcpiError> for AcpiQueryError {
    fn from(err: AcpiError) -> Self {
        AcpiQueryError::Acpi(err)
    }
}

/// Root handle over the system's ACPI static tables.
pub struct AcpiQuery {
    tables: AcpiTables<DynamixAcpiHandler>,
    rsdp_address: usize,
}

impl AcpiQuery {
    /// Builds an `AcpiQuery` from the RSDP's physical address.
    ///
    /// # Safety
    /// `rdsp_address` must point to a valid mapped adress where rdsp is present
    pub unsafe fn from_rsdp(rsdp_address: usize) -> Result<Self, AcpiQueryError> {
        // SAFETY: upheld by this function's own contract.
        let tables = unsafe { AcpiTables::from_rsdp(DynamixAcpiHandler, rsdp_address)? };
        Ok(Self { tables, rsdp_address })
    }

    /// Re-reads the root table (RSDT/XSDT) through the live page tables.
    ///
    /// # Safety
    /// The new MMU page tables must be present, and `rdsp_adress`
    /// must still be a valid address holding the `rdsp`
    pub unsafe fn remap(&mut self) -> Result<(), AcpiQueryError> {
        // SAFETY: upheld by this function's own contract.
        self.tables = unsafe { AcpiTables::from_rsdp(DynamixAcpiHandler, self.rsdp_address)? };
        Ok(())
    }

    /// Physical range of the whole DSDT, SDT header included.
    pub fn dsdt_aml(&self) -> Option<PhysRange> {
        self.tables.dsdt().ok().map(|table| PhysRange::new(PhysAddr::new(table.address), table.length as usize))
    }

    /// The ACPI revision reported by the RSDP (`0` for ACPI 1.0, using the
    /// 32-bit RSDT; nonzero for ACPI 2.0+, using the 64-bit XSDT).
    pub fn revision(&self) -> u8 {
        self.tables.revision()
    }

    /// Finds and maps the single static table with signature `T::SIGNATURE`
    /// (e.g. `acpi::fadt::Fadt`, `acpi::madt::Madt`).
    pub fn find_table<T: AcpiTable>(&self) -> Option<PhysicalMapping<DynamixAcpiHandler, T>> {
        self.tables.find_table::<T>().ok()
    }

    /// Iterates the header of every static table firmware published
    pub fn headers(&self) -> impl Iterator<Item = SdtHeader> + '_ {
        self.tables.headers()
    }
}

/// ACPI `_HID` of a 16550-compatible UART.
const HID_NS16550: &str = "PNP0501";
/// `PNP0501` as an AML `DWordConst`: opcode, then the compressed EISA id.
const AML_EISA_PNP0501: [u8; 5] = [0x0C, 0x41, 0xD0, 0x05, 0x01];
/// Where the PC's first 16550 UART decodes its registers.
const LEGACY_COM1: DeviceResource = DeviceResource::Io { base: 0x3F8, count: 8 };

impl AcpiQuery {
    /// Whether the DSDT mentions `hid` as a device `_HID` (string or EISA id form).
    ///
    /// This is a byte scan, not AML evaluation: it can say a device of that
    /// type is declared, not where it is. TODO: replace with real
    /// `_HID`/`_CRS` walk once the `aml` interpreter exists.
    fn dsdt_declares(&self, hid: &str, eisa: &[u8]) -> bool {
        let Some(range) = self.dsdt_aml() else { return false };
        if mmu::map_normal(range).is_err() {
            return false;
        }
        // SAFETY: the DSDT was just mapped and firmware keeps it alive.
        let aml = unsafe { core::slice::from_raw_parts(range.start().as_usize() as *const u8, range.len()) };
        aml.windows(eisa.len()).any(|w| w == eisa)
            || aml.windows(hid.len()).any(|w| w == hid.as_bytes())
    }
}

impl DeviceQuery for AcpiQuery {
    fn find_compatible_resource(&self, compatible: &str) -> Option<DeviceResource> {
        // Stopgap until AML is interpreted: a declared 16550 UART is assumed to
        // sit at the legacy COM1 ports, which is where QEMU and PCs put it.
        match compatible {
            HID_NS16550 if self.dsdt_declares(HID_NS16550, &AML_EISA_PNP0501) => Some(LEGACY_COM1),
            _ => None,
        }
    }

    fn as_acpi(&self) -> Option<&AcpiQuery> {
        Some(self)
    }
}
