//! ACPI power-off (sleep state S5) for x86_64.

use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};

use acpi::address::AddressSpace;
use acpi::fadt::Fadt;

use crate::services::acpi::AcpiQuery;
use crate::services::power::PowerError;
use crate::x86_64::port::{inw, outb, outw};

const SCI_EN: u16 = 1 << 0;
const SLP_TYP_SHIFT: u16 = 10;
const SLP_TYP_MASK: u16 = 0b111 << SLP_TYP_SHIFT;
const SLP_EN: u16 = 1 << 13;

const AML_NAME_OP: u8 = 0x08;
const AML_ROOT_PREFIX: u8 = b'\\';
const AML_PACKAGE_OP: u8 = 0x12;
const AML_ZERO_OP: u8 = 0x00;
const AML_ONE_OP: u8 = 0x01;
const AML_BYTE_PREFIX: u8 = 0x0A;

static READY: AtomicBool = AtomicBool::new(false);
static PM1A_CONTROL: AtomicU16 = AtomicU16::new(0);
static PM1B_CONTROL: AtomicU16 = AtomicU16::new(0);
static SLP_TYP_A: AtomicU8 = AtomicU8::new(0);
static SLP_TYP_B: AtomicU8 = AtomicU8::new(0);

/// Returns the I/O port of a PM1 control register, if it is a usable one.
fn io_port(address_space: AddressSpace, address: u64) -> Option<u16> {
    if address_space == AddressSpace::SystemIo && address != 0 {
        u16::try_from(address).ok()
    } else {
        None
    }
}

/// Reads one AML integer constant (`ZeroOp`, `OneOp` or `BytePrefix N`).
fn aml_small_int(aml: &[u8], at: &mut usize) -> Option<u8> {
    match *aml.get(*at)? {
        AML_ZERO_OP => {
            *at += 1;
            Some(0)
        }
        AML_ONE_OP => {
            *at += 1;
            Some(1)
        }
        AML_BYTE_PREFIX => {
            let value = *aml.get(*at + 1)?;
            *at += 2;
            Some(value)
        }
        _ => None,
    }
}

/// Finds `Name(_S5, Package(){ SLP_TYPa, SLP_TYPb, ... })` and returns the two values.
fn find_s5(aml: &[u8]) -> Option<(u8, u8)> {
    let mut search = 0;
    while let Some(offset) = aml.get(search..)?.windows(4).position(|w| w == b"_S5_") {
        let name_at = search + offset;
        search = name_at + 1;

        // Must be the definition `NameOp [\] _S5_`, not a mere reference.
        let after_name_op = name_at.checked_sub(1).is_some_and(|i| aml[i] == AML_NAME_OP)
            || (name_at >= 2 && aml[name_at - 1] == AML_ROOT_PREFIX && aml[name_at - 2] == AML_NAME_OP);
        if !after_name_op {
            continue;
        }

        let mut at = name_at + 4;
        if *aml.get(at)? != AML_PACKAGE_OP {
            continue;
        }
        at += 1;
        // PkgLength: the top two bits of the lead byte count the extra bytes.
        let lead = *aml.get(at)?;
        at += 1 + (lead >> 6) as usize;
        at += 1; // NumElements

        let typ_a = aml_small_int(aml, &mut at)?;
        let typ_b = aml_small_int(aml, &mut at)?;
        return Some((typ_a, typ_b));
    }
    None
}

/// Reads the FADT and DSDT and prepares `system_off`.
pub fn init(acpi: &AcpiQuery) -> Result<(), PowerError> {
    if READY.load(Ordering::Acquire) {
        return Err(PowerError::Unsupported);
    }

    let (pm1a, pm1b, smi_cmd, acpi_enable) = {
        let fadt = acpi.find_table::<Fadt>().ok_or(PowerError::NotDescribed)?;
        let pm1a = fadt.pm1a_control_block().map_err(|_| PowerError::Unsupported)?;
        let pm1a = io_port(pm1a.address_space, pm1a.address).ok_or(PowerError::Unsupported)?;
        let pm1b = match fadt.pm1b_control_block().map_err(|_| PowerError::Unsupported)? {
            Some(gas) => io_port(gas.address_space, gas.address),
            None => None,
        };
        (pm1a, pm1b, fadt.smi_cmd_port, fadt.acpi_enable)
    };

    let (aml_phys, aml_len) = acpi.dsdt_aml().ok_or(PowerError::NotDescribed)?;
    crate::services::mmu::map_normal(aml_phys, aml_len).map_err(|_| PowerError::Unsupported)?;
    // SAFETY: the DSDT's AML stream was just mapped and firmware keeps it alive.
    let aml = unsafe { core::slice::from_raw_parts(aml_phys as *const u8, aml_len) };
    let (typ_a, typ_b) = find_s5(aml).ok_or(PowerError::NotDescribed)?;

    // Firmware normally enables ACPI mode itself; if it did not, ask the chipset to.
    // SAFETY: ports come from the FADT.
    unsafe {
        if inw(pm1a) & SCI_EN == 0 && smi_cmd != 0 && acpi_enable != 0 {
            outb(smi_cmd as u16, acpi_enable);
            for _ in 0..1_000_000 {
                if inw(pm1a) & SCI_EN != 0 {
                    break;
                }
            }
        }
    }

    PM1A_CONTROL.store(pm1a, Ordering::Relaxed);
    PM1B_CONTROL.store(pm1b.unwrap_or(0), Ordering::Relaxed);
    SLP_TYP_A.store(typ_a, Ordering::Relaxed);
    SLP_TYP_B.store(typ_b, Ordering::Relaxed);
    READY.store(true, Ordering::Release);
    Ok(())
}

/// Requests S5 (soft off). Returns if power-off was not set up or did not take effect.
pub fn system_off() {
    if !READY.load(Ordering::Acquire) {
        return;
    }
    let pm1a = PM1A_CONTROL.load(Ordering::Relaxed);
    let pm1b = PM1B_CONTROL.load(Ordering::Relaxed);
    let typ_a = SLP_TYP_A.load(Ordering::Relaxed) as u16;
    let typ_b = SLP_TYP_B.load(Ordering::Relaxed) as u16;

    // SAFETY: registers described by the FADT; interrupts must not run while powering off.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let value = (inw(pm1a) & !SLP_TYP_MASK) | (typ_a << SLP_TYP_SHIFT) | SLP_EN;
        outw(pm1a, value);
        if pm1b != 0 {
            let value = (inw(pm1b) & !SLP_TYP_MASK) | (typ_b << SLP_TYP_SHIFT) | SLP_EN;
            outw(pm1b, value);
        }
    }
}
