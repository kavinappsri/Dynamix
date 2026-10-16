//! ACPI table access for x86_64, currently backed by `acpi` crate
//!
//! Currently, this is heap-less, so no AML parsing. im reserving that stage for future polish
//! once mmu and pmm are up and working. So, currently the `alloc` / `allocator_api` are disabled
//! in the `acpi` crate


use core::ptr::NonNull;

use crate::services::device_query::{DeviceQuery, MemoryRegion};

use acpi::{AcpiError, AcpiTable, AcpiTables};
use acpi::handler::{AcpiHandler, PhysicalMapping};
use acpi::sdt::SdtHeader;

#[derive(Clone, Copy)]
pub struct DynamixAcpiHandler;

impl AcpiHandler for DynamixAcpiHandler {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> PhysicalMapping<Self, T> {
        #[cfg(target_arch = "x86_64")]
        if crate::x86_64::x64_mmu::root().is_some() {
            crate::services::mmu::map_normal(physical_address, size).expect("failed to map an ACPI table");
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
    pub unsafe fn from_rsdp(rsdp_address: usize) -> Result<Self, AcpiQueryError> {
        // SAFETY: upheld by this function's own contract.
        let tables = unsafe { AcpiTables::from_rsdp(DynamixAcpiHandler, rsdp_address)? };
        Ok(Self { tables, rsdp_address })
    }

    /// Re-reads the root table (RSDT/XSDT) through the live page tables.
    ///
    /// `AcpiTables` keeps a pointer to the root table it mapped at creation.
    /// A query built before the kernel switched page tables points into
    /// firmware memory the kernel tables do not map, so call this once after
    /// the switch (and before any `find_table`).
    ///
    /// # Safety
    /// Same contract as [`AcpiQuery::from_rsdp`]; paging must be installed.
    pub unsafe fn remap(&mut self) -> Result<(), AcpiQueryError> {
        // SAFETY: upheld by this function's own contract.
        self.tables = unsafe { AcpiTables::from_rsdp(DynamixAcpiHandler, self.rsdp_address)? };
        Ok(())
    }

    /// Physical `(address, length)` of the whole DSDT, SDT header included.
    pub fn dsdt_aml(&self) -> Option<(usize, usize)> {
        self.tables.dsdt().ok().map(|table| (table.address, table.length as usize))
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

impl DeviceQuery for AcpiQuery {
    fn find_compatible_address(&self, _compatible: &str) -> Option<(usize, usize)> {
        // ACPI has no `compatible`-string equivalent for arbitrary platform
        // devices; that lookup goes through `_HID`/`_CID` in the AML
        // namespace, which this service CURRENTLY doesn't support
        None
    }

    fn memory_regions(&self, _out: &mut [MemoryRegion]) -> usize {
        // ACPI's static tables don't describe usable RAM
        0
    }

    fn as_acpi(&self) -> Option<&AcpiQuery> {
        Some(self)
    }
}
