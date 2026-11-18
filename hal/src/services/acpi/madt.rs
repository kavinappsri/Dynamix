//! The MADT

use core::mem::size_of;

use acpica_sys as sys;

use super::AcpiError;
use super::table::{Pod, Table, TableKind, read_pod};

// Every struct below is integers only
// SAFETY (all): plain-old-data C structs.
unsafe impl Pod for sys::ACPI_TABLE_MADT {}
unsafe impl Pod for sys::ACPI_SUBTABLE_HEADER {}
unsafe impl Pod for sys::ACPI_MADT_LOCAL_APIC {}
unsafe impl Pod for sys::ACPI_MADT_IO_APIC {}
unsafe impl Pod for sys::ACPI_MADT_INTERRUPT_OVERRIDE {}
unsafe impl Pod for sys::ACPI_MADT_NMI_SOURCE {}
unsafe impl Pod for sys::ACPI_MADT_LOCAL_APIC_NMI {}
unsafe impl Pod for sys::ACPI_MADT_LOCAL_APIC_OVERRIDE {}
unsafe impl Pod for sys::ACPI_MADT_LOCAL_X2APIC {}
unsafe impl Pod for sys::ACPI_MADT_LOCAL_X2APIC_NMI {}

struct MadtKind;

impl TableKind for MadtKind {
    const SIGNATURE: &'static [u8; 5] = sys::ACPI_SIG_MADT;
}

/// MADT flag: the machine also has the legacy 8259 PICs.
const MADT_PCAT_COMPAT: u32 = 1 << 0;

/// The MADT. Holds the table (and gives it back on drop), so keep it only as
/// long as you are reading it.
pub struct Madt {
    table: Table<MadtKind>,
    local_apic_address: u32,
    flags: u32,
}

impl Madt {
    /// Finds and verifies the MADT.
    pub fn get() -> Result<Self, AcpiError> {
        let table = Table::<MadtKind>::get(1)?;
        let fixed: sys::ACPI_TABLE_MADT = read_pod(table.bytes(), 0).ok_or(AcpiError::BadParameter)?;
        Ok(Self { local_apic_address: fixed.Address, flags: fixed.Flags, table })
    }

    /// Physical address of the local APICs, as firmware says (the
    /// [`MadtEntry::LocalApicAddressOverride`] entry can replace it).
    pub fn local_apic_address(&self) -> u32 {
        self.local_apic_address
    }

    /// Whether the machine also has dual 8259 PICs that must be masked off.
    pub fn has_legacy_pics(&self) -> bool {
        self.flags & MADT_PCAT_COMPAT != 0
    }

    /// Every entry after the fixed part, in table order. A damaged entry
    /// (zero length, running past the table) ends the walk.
    pub fn entries(&self) -> MadtEntries<'_> {
        MadtEntries { bytes: self.table.bytes(), offset: size_of::<sys::ACPI_TABLE_MADT>() }
    }
}

/// One MADT entry, copied out of the table.
///
/// More entries will be made in the enum as they are needed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MadtEntry {
    /// A CPU's local APIC (`flags` bit 0: enabled, bit 1: can be brought online).
    LocalApic { processor_id: u8, apic_id: u8, flags: u32 },
    /// An I/O APIC and the first interrupt (GSI) it serves.
    IoApic { id: u8, address: u32, gsi_base: u32 },
    /// "ISA line `source_irq` on `bus` really arrives on `gsi`, wired like `flags`."
    /// `flags` bits 0..2 are the polarity, 2..4 the trigger mode (MPS INTI flags).
    InterruptOverride { bus: u8, source_irq: u8, gsi: u32, flags: u16 },
    /// A non-maskable interrupt wired to an I/O APIC pin.
    NmiSource { flags: u16, gsi: u32 },
    /// A non-maskable interrupt wired to a local APIC's LINT pin
    /// (`processor_id` 0xFF means all of them).
    LocalApicNmi { processor_id: u8, flags: u16, lint: u8 },
    /// A 64-bit address for the local APICs.
    LocalApicAddressOverride { address: u64 },
    /// A CPU whose APIC id does not fit in a byte.
    LocalX2Apic { apic_id: u32, flags: u32, uid: u32 },
    /// Like [`LocalApicNmi`](MadtEntry::LocalApicNmi) for x2APIC ids (`uid` 0xFFFF_FFFF means all).
    LocalX2ApicNmi { flags: u16, uid: u32, lint: u8 },
    /// Anything else (or an entry too short for its kind).
    Unknown { kind: u8, length: u8 },
}

impl MadtEntry {
    fn parse(kind: u8, length: u8, body: &[u8]) -> Self {
        let parsed = match u32::from(kind) {
            sys::ACPI_MADT_TYPE_LOCAL_APIC => read_pod::<sys::ACPI_MADT_LOCAL_APIC>(body, 0).map(|e| {
                MadtEntry::LocalApic { processor_id: e.ProcessorId, apic_id: e.Id, flags: e.LapicFlags }
            }),
            sys::ACPI_MADT_TYPE_IO_APIC => read_pod::<sys::ACPI_MADT_IO_APIC>(body, 0).map(|e| {
                MadtEntry::IoApic { id: e.Id, address: e.Address, gsi_base: e.GlobalIrqBase }
            }),
            sys::ACPI_MADT_TYPE_INTERRUPT_OVERRIDE => read_pod::<sys::ACPI_MADT_INTERRUPT_OVERRIDE>(body, 0).map(|e| {
                MadtEntry::InterruptOverride { bus: e.Bus, source_irq: e.SourceIrq, gsi: e.GlobalIrq, flags: e.IntiFlags }
            }),
            sys::ACPI_MADT_TYPE_NMI_SOURCE => read_pod::<sys::ACPI_MADT_NMI_SOURCE>(body, 0)
                .map(|e| MadtEntry::NmiSource { flags: e.IntiFlags, gsi: e.GlobalIrq }),
            sys::ACPI_MADT_TYPE_LOCAL_APIC_NMI => read_pod::<sys::ACPI_MADT_LOCAL_APIC_NMI>(body, 0).map(|e| {
                MadtEntry::LocalApicNmi { processor_id: e.ProcessorId, flags: e.IntiFlags, lint: e.Lint }
            }),
            sys::ACPI_MADT_TYPE_LOCAL_APIC_OVERRIDE => read_pod::<sys::ACPI_MADT_LOCAL_APIC_OVERRIDE>(body, 0)
                .map(|e| MadtEntry::LocalApicAddressOverride { address: e.Address }),
            sys::ACPI_MADT_TYPE_LOCAL_X2APIC => read_pod::<sys::ACPI_MADT_LOCAL_X2APIC>(body, 0).map(|e| {
                MadtEntry::LocalX2Apic { apic_id: e.LocalApicId, flags: e.LapicFlags, uid: e.Uid }
            }),
            sys::ACPI_MADT_TYPE_LOCAL_X2APIC_NMI => read_pod::<sys::ACPI_MADT_LOCAL_X2APIC_NMI>(body, 0).map(|e| {
                MadtEntry::LocalX2ApicNmi { flags: e.IntiFlags, uid: e.Uid, lint: e.Lint }
            }),
            _ => None,
        };
        parsed.unwrap_or(MadtEntry::Unknown { kind, length })
    }
}

/// Iterator over a [`Madt`]'s entries; see [`Madt::entries`].
pub struct MadtEntries<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Iterator for MadtEntries<'_> {
    type Item = MadtEntry;

    fn next(&mut self) -> Option<MadtEntry> {
        let header: sys::ACPI_SUBTABLE_HEADER = read_pod(self.bytes, self.offset)?;
        let length = usize::from(header.Length);
        let end = self.offset.checked_add(length);
        match end {
            Some(end) if length >= size_of::<sys::ACPI_SUBTABLE_HEADER>() && end <= self.bytes.len() => {
                // Hand the parser only this entry's bytes, so a short entry
                // cannot read into its neighbour.
                let body = &self.bytes[self.offset..end];
                self.offset = end;
                Some(MadtEntry::parse(header.Type, header.Length, body))
            }
            _ => {
                // Damaged entry: nothing after it can be trusted.
                self.offset = self.bytes.len();
                None
            }
        }
    }
}
