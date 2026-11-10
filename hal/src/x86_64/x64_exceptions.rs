//! x86_64 exception handling: the IDT, entry stubs and the fatal-fault reporter.

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::fmt;

use crate::services::fault::{self, FaultClass, FaultReport, Hex};
use crate::x86_64::gdt;

global_asm!(include_str!("../asm/x86_64/x64_vectors.s"));

#[allow(non_upper_case_globals)]
unsafe extern "C" {
    static isr_stub_table: [usize; 32];
    /// First of the 224 fixed-size entry stubs for vectors 32..=255.
    static irq_stub_first: u8;
}

/// Bytes between consecutive external-interrupt stubs (see `x64_vectors.s`).
const IRQ_STUB_SIZE: usize = 16;
/// First vector that is not a CPU exception.
pub const FIRST_EXTERNAL_VECTOR: u64 = 32;

const GATE_PRESENT_INTERRUPT: u8 = 0x8E; // present, DPL0, 64-bit interrupt gate
const VECTOR_BREAKPOINT: u64 = 3;

/// Register state at the time of the exception; layout must match
/// `x64_vectors.s` (GPRs pushed in reverse, then the hardware frame).
#[repr(C)]
pub struct ExceptionContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl Gate {
    const MISSING: Gate = Gate { offset_low: 0, selector: 0, ist: 0, attributes: 0, offset_mid: 0, offset_high: 0, reserved: 0 };

    fn new(handler: u64, ist: u8) -> Gate {
        Gate {
            offset_low: handler as u16,
            selector: gdt::KERNEL_CS,
            ist,
            attributes: GATE_PRESENT_INTERRUPT,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, align(16))]
struct Idt(UnsafeCell<[Gate; 256]>);

// SAFETY: written once by `init` during single-core boot, read by hardware after.
unsafe impl Sync for Idt {}

static IDT: Idt = Idt(UnsafeCell::new([Gate::MISSING; 256]));

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

const NAMES: [&str; 32] = [
    "#DE divide error",
    "#DB debug",
    "NMI",
    "#BP breakpoint",
    "#OF overflow",
    "#BR bound range exceeded",
    "#UD invalid opcode",
    "#NM device not available",
    "#DF double fault",
    "coprocessor segment overrun",
    "#TS invalid TSS",
    "#NP segment not present",
    "#SS stack-segment fault",
    "#GP general protection fault",
    "#PF page fault",
    "reserved (15)",
    "#MF x87 floating-point",
    "#AC alignment check",
    "#MC machine check",
    "#XM SIMD floating-point",
    "#VE virtualization",
    "#CP control protection",
    "reserved (22)",
    "reserved (23)",
    "reserved (24)",
    "reserved (25)",
    "reserved (26)",
    "reserved (27)",
    "#HV hypervisor injection",
    "#VC VMM communication",
    "#SX security",
    "reserved (31)",
];

/// Installs the kernel GDT/TSS and the IDT. Call once, panic, with interrupts masked.
pub fn init() {
    // SAFETY: single core, interrupts masked, higher-half code. The stub table
    // holds relocated (higher-half) addresses by the time we run.
    unsafe {
        gdt::init();

        let idt = &mut *IDT.0.get();
        for (vector, &stub) in isr_stub_table.iter().enumerate() {
            let ist = match vector {
                2 | 8 | 18 => gdt::IST_EMERGENCY,
                _ => 0,
            };
            idt[vector] = Gate::new(stub as u64, ist);
        }

        let first_irq_stub = core::ptr::addr_of!(irq_stub_first) as usize;
        for vector in FIRST_EXTERNAL_VECTOR as usize..256 {
            let stub = first_irq_stub + (vector - FIRST_EXTERNAL_VECTOR as usize) * IRQ_STUB_SIZE;
            idt[vector] = Gate::new(stub as u64, 0);
        }

        let pointer = DescriptorTablePointer {
            limit: (core::mem::size_of::<[Gate; 256]>() - 1) as u16,
            base: idt.as_ptr() as u64,
        };
        asm!("lidt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
    }
}

fn read_cr2() -> u64 {
    let value: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Decoded cause of a fault, from the vector and its error code.
struct Cause<'a>(&'a ExceptionContext);

impl fmt::Display for Cause<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let e = self.0.error_code;
        match self.0.vector {
            14 => write!(
                f,
                "{} on {} ({}{}{}), error code {:#x}",
                if e & 1 != 0 { "protection violation" } else { "page not present" },
                if e & 2 != 0 { "write" } else { "read" },
                if e & 4 != 0 { "user" } else { "supervisor" },
                if e & 8 != 0 { ", reserved bit set" } else { "" },
                if e & 16 != 0 { ", instruction fetch" } else { "" },
                e,
            ),
            // Faults that report a selector: bit 0 external, bits 1-2 which
            // table, the rest the index. Zero means "not segment related".
            10..=13 if e != 0 => write!(
                f,
                "error code {:#x}: {} index {}{}",
                e,
                match (e >> 1) & 3 {
                    0 => "GDT",
                    2 => "LDT",
                    _ => "IDT",
                },
                e >> 3,
                if e & 1 != 0 { " (external event)" } else { "" },
            ),
            8 => write!(f, "a fault occurred while delivering another one (often a stack overflow), error code {:#x}", e),
            // Only these vectors push an error code; for the rest the field is just 0.
            8 | 10..=14 | 17 | 21 | 29 | 30 => write!(f, "error code {:#x}", e),
            _ => write!(f, "no error code"),
        }
    }
}

/// The register dump, laid out like the other architectures' (three per row).
struct Registers<'a>(&'a ExceptionContext);

impl fmt::Display for Registers<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.0;
        let h = |v: u64| Hex(v as usize);
        writeln!(f, "rax {}  rbx {}  rcx {}", h(c.rax), h(c.rbx), h(c.rcx))?;
        writeln!(f, "rdx {}  rsi {}  rdi {}", h(c.rdx), h(c.rsi), h(c.rdi))?;
        writeln!(f, "rbp {}  r8  {}  r9  {}", h(c.rbp), h(c.r8), h(c.r9))?;
        writeln!(f, "r10 {}  r11 {}  r12 {}", h(c.r10), h(c.r11), h(c.r12))?;
        writeln!(f, "r13 {}  r14 {}  r15 {}", h(c.r13), h(c.r14), h(c.r15))?;
        writeln!(f, "rsp {}  rflags {}", h(c.rsp), h(c.rflags))?;
        writeln!(f, "cs  {:#06x}  ss {:#06x}", c.cs, c.ss)
    }
}

/// Called by `isr_common` for every vector. Returns for #BP and for external interrupts.
#[unsafe(no_mangle)]
pub extern "efiapi" fn x86_exception_handler(ctx: &mut ExceptionContext) {
    if ctx.vector >= FIRST_EXTERNAL_VECTOR {
        // External interrupt or IPI, not a fault: hand it to the interrupt service.
        crate::services::interrupts::handle_interrupt(Some(ctx.vector as u32));
        return;
    }

    let cause = Cause(ctx);
    let registers = Registers(ctx);
    let report = FaultReport {
        class: FaultClass::Exception,
        kind: NAMES.get(ctx.vector as usize).copied().unwrap_or("unknown"),
        cause: &cause,
        pc: Some(ctx.rip as usize),
        fault_addr: (ctx.vector == 14).then(|| read_cr2() as usize),
        registers: Some(&registers),
        frame_pointer: ctx.rbp as usize,
        link_register: None,
        stack_hint: ctx.rsp as usize,
        code: ctx.vector as u8,
    };

    if ctx.vector == VECTOR_BREAKPOINT {
        fault::print(&report);
        return;
    }
    fault::fatal(&report)
}
