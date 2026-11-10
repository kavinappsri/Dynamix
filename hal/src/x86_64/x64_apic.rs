//! x86_64 interrupt controller: the local APIC and the I/O APIC(s).
//!
//! # Numbering
//!
//! * `Irq` 0..256 are GSIs, routed through an I/O APIC to a dynamically
//!   assigned vector (0x30..=0xDF), handed out the first time a line is enabled.
//! * `Irq` 256..272 are software lines for self-IPIs (and, later, cross-core
//!   IPIs), on fixed vectors 0xE0..=0xEF.
//! * Vector 0xFF is the LAPIC spurious vector.
//!
//! # Not done yet
//!
//! * Routing is to the boot core only ([`set_affinity`] is unsupported): doing
//!   it properly needs the MADT processor list mapped to logical CPU ids.
//! * No MSI/MSI-X, no LAPIC timer, no NMI wiring.
//! * The IMCR (for machines that gate the APIC behind it) is not touched.
//!
//! [`set_affinity`]: InterruptController::set_affinity

use core::arch::asm;

use acpi::madt::{Madt, MadtEntry};

use crate::driver_traits::interrupt_controller::{
    Claim, InterruptController, Irq, IrqError, Polarity, Trigger,
};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::device_query::DeviceQuery;
use crate::services::interrupts::IrqConfig;
use crate::services::mmio::Mmio;
use crate::services::mmu;
use crate::services::sync::{IrqSpinLock, StaticCell};
use crate::x86_64::port::outb;

const MAX_IOAPICS: usize = 8;
const GSI_COUNT: u32 = 256;
const SOFTWARE_IRQ_BASE: u32 = GSI_COUNT;
const SOFTWARE_IRQ_COUNT: u32 = 16;
const IRQ_COUNT: u32 = SOFTWARE_IRQ_BASE + SOFTWARE_IRQ_COUNT;

const FIRST_DYNAMIC_VECTOR: u8 = 0x30;
const LAST_DYNAMIC_VECTOR: u8 = 0xDF;
const FIRST_SOFTWARE_VECTOR: u8 = 0xE0;
const SPURIOUS_VECTOR: u8 = 0xFF;
const NO_IRQ: u16 = u16::MAX;

const MSR_APIC_BASE: u32 = 0x1B;
const APIC_BASE_X2APIC: u64 = 1 << 10;
const APIC_BASE_ENABLE: u64 = 1 << 11;

// Local APIC registers (xAPIC MMIO offsets; x2APIC MSR = 0x800 + offset / 16).
const LAPIC_ID: usize = 0x20;
const LAPIC_TPR: usize = 0x80;
const LAPIC_EOI: usize = 0xB0;
const LAPIC_SVR: usize = 0xF0;
const LAPIC_ICR_LOW: usize = 0x300;
const LAPIC_SVR_ENABLE: u32 = 1 << 8;
const MSR_X2APIC_SELF_IPI: u32 = 0x83F;

// I/O APIC registers and redirection entry bits.
const IOREGSEL: usize = 0x00;
const IOWIN: usize = 0x10;
const IOAPIC_VER: u32 = 1;
const IOAPIC_REDIR_BASE: u32 = 0x10;
const REDIR_POLARITY_LOW: u32 = 1 << 13;
const REDIR_TRIGGER_LEVEL: u32 = 1 << 15;
const REDIR_MASKED: u32 = 1 << 16;

static APIC: StaticCell<Apic> = StaticCell::uninit();

fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: callers only read architectural APIC MSRs.
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    (hi as u64) << 32 | lo as u64
}

fn wrmsr(msr: u32, value: u64) {
    // SAFETY: callers only write architectural APIC MSRs.
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags))
    };
}

/// The local APIC, in whichever mode firmware left it.
#[derive(Clone, Copy)]
enum Lapic {
    Xapic(Mmio),
    X2apic,
}

impl Lapic {
    fn read(self, reg: usize) -> u32 {
        match self {
            Lapic::Xapic(mmio) => mmio.read32(reg),
            Lapic::X2apic => rdmsr(0x800 + (reg >> 4) as u32) as u32,
        }
    }

    fn write(self, reg: usize, value: u32) {
        match self {
            Lapic::Xapic(mmio) => mmio.write32(reg, value),
            Lapic::X2apic => wrmsr(0x800 + (reg >> 4) as u32, value as u64),
        }
    }

    fn id(self) -> u32 {
        match self {
            Lapic::Xapic(_) => self.read(LAPIC_ID) >> 24,
            Lapic::X2apic => self.read(LAPIC_ID),
        }
    }

    fn eoi(self) {
        self.write(LAPIC_EOI, 0);
    }

    fn self_ipi(self, vector: u8) {
        match self {
            Lapic::X2apic => wrmsr(MSR_X2APIC_SELF_IPI, vector as u64),
            Lapic::Xapic(_) => {
                // Fixed delivery, level assert, destination shorthand "self".
                self.write(LAPIC_ICR_LOW, vector as u32 | (1 << 14) | (0b01 << 18));
                // Delivery status: wait for the APIC to take it.
                while self.read(LAPIC_ICR_LOW) & (1 << 12) != 0 {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct IoApic {
    mmio: Mmio,
    gsi_base: u32,
    entries: u32,
}

impl IoApic {
    // Index/data are a two-step access: callers hold the APIC lock.
    fn read(&self, reg: u32) -> u32 {
        self.mmio.write32(IOREGSEL, reg);
        self.mmio.read32(IOWIN)
    }

    fn write(&self, reg: u32, value: u32) {
        self.mmio.write32(IOREGSEL, reg);
        self.mmio.write32(IOWIN, value);
    }

    fn covers(&self, gsi: u32) -> bool {
        gsi >= self.gsi_base && gsi < self.gsi_base + self.entries
    }
}

/// How firmware wired one legacy ISA line.
#[derive(Clone, Copy)]
struct IsaLine {
    gsi: u32,
    trigger: Trigger,
    polarity: Polarity,
}

struct Dynamic {
    irq_to_vector: [u8; IRQ_COUNT as usize],
    vector_to_irq: [u16; 256],
    next_vector: u8,
}

impl Dynamic {
    fn vector_for(&mut self, irq: u32) -> Result<u8, IrqError> {
        let existing = self.irq_to_vector[irq as usize];
        if existing != 0 {
            return Ok(existing);
        }
        if self.next_vector > LAST_DYNAMIC_VECTOR {
            return Err(IrqError::NoFreeVectors);
        }
        let vector = self.next_vector;
        self.next_vector += 1;
        self.irq_to_vector[irq as usize] = vector;
        self.vector_to_irq[vector as usize] = irq as u16;
        Ok(vector)
    }
}

/// The LAPIC + I/O APIC controller.
pub struct Apic {
    lapic: Lapic,
    ioapics: [Option<IoApic>; MAX_IOAPICS],
    isa: [IsaLine; 16],
    boot_apic_id: u32,
    /// Vector bookkeeping, and the lock that serialises I/O APIC register access.
    dynamic: IrqSpinLock<Dynamic>,
}

impl Apic {
    fn ioapic_for(&self, gsi: u32) -> Option<(IoApic, u32)> {
        self.ioapics
            .iter()
            .flatten()
            .find(|io| io.covers(gsi))
            .map(|io| (*io, gsi - io.gsi_base))
    }
}

fn redir_bits(trigger: Trigger, polarity: Polarity) -> u32 {
    let trig = if trigger == Trigger::Level { REDIR_TRIGGER_LEVEL } else { 0 };
    let pol = if polarity == Polarity::ActiveLow { REDIR_POLARITY_LOW } else { 0 };
    trig | pol
}

impl InterruptController for Apic {
    fn irq_count(&self) -> u32 {
        IRQ_COUNT
    }

    fn init_this_cpu(&self) -> Result<(), IrqError> {
        let base = rdmsr(MSR_APIC_BASE);
        wrmsr(MSR_APIC_BASE, base | APIC_BASE_ENABLE);
        self.lapic.write(LAPIC_TPR, 0);
        self.lapic.write(LAPIC_SVR, LAPIC_SVR_ENABLE | SPURIOUS_VECTOR as u32);
        Ok(())
    }

    fn enable(&self, irq: Irq) -> Result<(), IrqError> {
        if irq.0 >= SOFTWARE_IRQ_BASE {
            // Software lines have no mask.
            return if irq.0 < IRQ_COUNT { Ok(()) } else { Err(IrqError::OutOfRange) };
        }
        let (io, pin) = self.ioapic_for(irq.0).ok_or(IrqError::OutOfRange)?;
        let mut dynamic = self.dynamic.lock();
        let vector = dynamic.vector_for(irq.0)?;

        let reg = IOAPIC_REDIR_BASE + 2 * pin;
        let keep = io.read(reg) & (REDIR_POLARITY_LOW | REDIR_TRIGGER_LEVEL);
        // Fixed delivery, physical destination: the boot core.
        io.write(reg + 1, self.boot_apic_id << 24);
        io.write(reg, keep | vector as u32);
        Ok(())
    }

    fn disable(&self, irq: Irq) -> Result<(), IrqError> {
        if irq.0 >= SOFTWARE_IRQ_BASE {
            return Err(IrqError::Unsupported);
        }
        let (io, pin) = self.ioapic_for(irq.0).ok_or(IrqError::OutOfRange)?;
        let _dynamic = self.dynamic.lock();
        let reg = IOAPIC_REDIR_BASE + 2 * pin;
        io.write(reg, io.read(reg) | REDIR_MASKED);
        Ok(())
    }

    fn configure(&self, irq: Irq, trigger: Trigger, polarity: Polarity) -> Result<(), IrqError> {
        if irq.0 >= SOFTWARE_IRQ_BASE {
            return Err(IrqError::Unsupported);
        }
        let (io, pin) = self.ioapic_for(irq.0).ok_or(IrqError::OutOfRange)?;
        let _dynamic = self.dynamic.lock();
        let reg = IOAPIC_REDIR_BASE + 2 * pin;
        let lo = io.read(reg) & !(REDIR_POLARITY_LOW | REDIR_TRIGGER_LEVEL);
        io.write(reg, lo | redir_bits(trigger, polarity));
        Ok(())
    }

    fn acknowledge(&self, cpu_vector: Option<u32>) -> Option<Claim> {
        let vector = cpu_vector? as u8;
        if vector == SPURIOUS_VECTOR {
            // Spurious interrupts must not be EOI'd.
            return None;
        }
        let irq = if vector >= FIRST_SOFTWARE_VECTOR && vector < FIRST_SOFTWARE_VECTOR + SOFTWARE_IRQ_COUNT as u8 {
            SOFTWARE_IRQ_BASE + (vector - FIRST_SOFTWARE_VECTOR) as u32
        } else {
            match self.dynamic.lock().vector_to_irq[vector as usize] {
                // A vector nobody assigned: still reported, so it gets its EOI.
                NO_IRQ => NO_IRQ as u32,
                irq => irq as u32,
            }
        };
        Some(Claim { irq: Irq(irq), raw: vector as u32 })
    }

    fn end_of_interrupt(&self, _claim: Claim) {
        // For level-triggered lines the LAPIC EOI also reaches the I/O APIC.
        self.lapic.eoi();
    }

    fn software_irq(&self, n: u8) -> Option<Irq> {
        ((n as u32) < SOFTWARE_IRQ_COUNT).then(|| Irq(SOFTWARE_IRQ_BASE + n as u32))
    }

    fn raise_self(&self, irq: Irq) -> Result<(), IrqError> {
        if irq.0 < SOFTWARE_IRQ_BASE || irq.0 >= IRQ_COUNT {
            return Err(IrqError::Unsupported);
        }
        self.lapic.self_ipi(FIRST_SOFTWARE_VECTOR + (irq.0 - SOFTWARE_IRQ_BASE) as u8);
        Ok(())
    }
}

/// How legacy ISA line `line` is wired, after MADT overrides.
pub fn isa_irq(line: u8) -> Option<IrqConfig> {
    let apic = APIC.get()?;
    let isa = apic.isa.get(line as usize)?;
    Some(IrqConfig { irq: Irq(isa.gsi), trigger: isa.trigger, polarity: isa.polarity })
}

/// Parks the legacy 8259 PICs: remapped clear of the CPU exception vectors,
/// then fully masked, so a stray legacy interrupt cannot look like a fault.
fn disable_legacy_pic() {
    // SAFETY: standard 8259 initialisation sequence on the PC's fixed ports.
    unsafe {
        let wait = || outb(0x80, 0);
        outb(0x20, 0x11);
        wait();
        outb(0xA0, 0x11);
        wait();
        outb(0x21, 0x20);
        wait();
        outb(0xA1, 0x28);
        wait();
        outb(0x21, 0x04);
        wait();
        outb(0xA1, 0x02);
        wait();
        outb(0x21, 0x01);
        wait();
        outb(0xA1, 0x01);
        wait();
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
    }
}

fn cpu_has_apic() -> bool {
    // SAFETY: CPUID leaf 1 exists on every x86_64 CPU.
    let leaf = unsafe { core::arch::x86_64::__cpuid(1) };
    leaf.edx & (1 << 9) != 0
}

/// Finds the APICs through the ACPI MADT, parks the legacy PIC, maps and
/// masks every I/O APIC pin, and returns the controller.
pub fn init(hardware: &dyn DeviceQuery) -> Result<&'static dyn InterruptController, IrqError> {
    if !cpu_has_apic() {
        return Err(IrqError::NoController);
    }
    let acpi = hardware.as_acpi().ok_or(IrqError::NoController)?;
    let mapping = acpi.find_table::<Madt>().ok_or(IrqError::NoController)?;
    let madt = mapping.get();

    let mut io_found = [None::<(u32, u32)>; MAX_IOAPICS]; // (address, gsi base)
    let mut io_count = 0;
    let mut isa = core::array::from_fn::<IsaLine, 16, _>(|line| IsaLine {
        gsi: line as u32,
        trigger: Trigger::Edge,
        polarity: Polarity::ActiveHigh,
    });

    for entry in madt.entries() {
        match entry {
            MadtEntry::IoApic(io) if io_count < MAX_IOAPICS => {
                io_found[io_count] = Some((io.io_apic_address, io.global_system_interrupt_base));
                io_count += 1;
            }
            MadtEntry::InterruptSourceOverride(over) if over.bus == 0 && (over.irq as usize) < 16 => {
                let flags = over.flags;
                isa[over.irq as usize] = IsaLine {
                    gsi: over.global_system_interrupt,
                    // 0b00 means "conforms to the bus": ISA is edge, active high.
                    polarity: if flags & 0b11 == 0b11 { Polarity::ActiveLow } else { Polarity::ActiveHigh },
                    trigger: if (flags >> 2) & 0b11 == 0b11 { Trigger::Level } else { Trigger::Edge },
                };
            }
            _ => {}
        }
    }
    if io_count == 0 {
        return Err(IrqError::NoController);
    }

    disable_legacy_pic();

    // The local APIC, in the mode firmware left it in.
    let base_msr = rdmsr(MSR_APIC_BASE);
    let lapic = if base_msr & APIC_BASE_X2APIC != 0 {
        Lapic::X2apic
    } else {
        let base = (base_msr & 0xFFFF_F000) as usize;
        mmu::map_device(PhysRange::new(PhysAddr::new(base), 0x1000)).map_err(|_| IrqError::InitFailed)?;
        // SAFETY: just mapped as device memory.
        Lapic::Xapic(unsafe { Mmio::new(base) })
    };

    let mut ioapics = [None::<IoApic>; MAX_IOAPICS];
    for (slot, found) in ioapics.iter_mut().zip(io_found.iter()) {
        let Some((address, gsi_base)) = *found else { continue };
        mmu::map_device(PhysRange::new(PhysAddr::new(address as usize), 0x1000)).map_err(|_| IrqError::InitFailed)?;
        // SAFETY: just mapped as device memory.
        let mmio = unsafe { Mmio::new(address as usize) };
        let mut io = IoApic { mmio, gsi_base, entries: 0 };
        io.entries = ((io.read(IOAPIC_VER) >> 16) & 0xFF) + 1;

        // Every pin masked, with the trigger mode firmware says it needs:
        // ISA lines as wired (after overrides), everything else PCI-style.
        for pin in 0..io.entries {
            let gsi = gsi_base + pin;
            let (trigger, polarity) = isa
                .iter()
                .find(|line| line.gsi == gsi)
                .map(|line| (line.trigger, line.polarity))
                .unwrap_or((Trigger::Level, Polarity::ActiveLow));
            io.write(IOAPIC_REDIR_BASE + 2 * pin, REDIR_MASKED | redir_bits(trigger, polarity));
            io.write(IOAPIC_REDIR_BASE + 2 * pin + 1, 0);
        }
        *slot = Some(io);
    }

    let boot_apic_id = lapic.id();
    let apic = APIC.get_or_init(|| Apic {
        lapic,
        ioapics,
        isa,
        boot_apic_id,
        dynamic: IrqSpinLock::new(Dynamic {
            irq_to_vector: [0; IRQ_COUNT as usize],
            vector_to_irq: [NO_IRQ; 256],
            next_vector: FIRST_DYNAMIC_VECTOR,
        }),
    });
    Ok(apic)
}
