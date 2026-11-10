//! AArch64 EL1 exception-vector initialization and diagnostic reporting.

use core::arch::global_asm;
use core::fmt;

use crate::services::fault::{self, FaultClass, FaultReport, Hex};

global_asm!(include_str!("../asm/aarch64/aarch64_vectors.s"));

unsafe extern "C" {
    static vector_table_el1: u8;
}

#[repr(C)]
#[derive(Debug)]
/// Register state captured by the assembly exception vector before Rust runs.
///
/// Its `repr(C)` layout must stay synchronized with `vectors.s`.
pub struct ExceptionContext {
    /// General-purpose registers `x0` through `x29`.
    pub gpr: [u64; 30], // x0 - x29
    /// Link register (`x30`).
    pub lr: u64, // x30 (Link Register)
    /// Exception Link Register, the instruction address that faulted.
    pub elr: u64, // Exception Link Register (Instruction address that faulted)
    /// Saved processor state at exception entry.
    pub spsr: u64, // Saved Program Status Register
    /// Exception Syndrome Register.
    pub esr: u64, // Exception Syndrome Register
    /// Fault Address Register, where applicable.
    pub far: u64, // Fault Address Register (Memory address involved in fault)
    /// Stack pointer of the interrupted code (also keeps the struct 16-byte aligned).
    pub sp: u64,
}

const _: () = assert!(core::mem::size_of::<ExceptionContext>() == 288, "must match FRAME_SIZE in aarch64_vectors.s");

/// Called by the IRQ vector once the interrupted context is saved. Lets the
/// interrupt service ask the GIC what fired and run its handlers.
#[unsafe(no_mangle)]
extern "C" fn aarch64_irq_entry(_ctx: &mut ExceptionContext) {
    crate::services::interrupts::handle_interrupt(None);
}

/// Installs the assembly exception table in `VBAR_EL1`.
///
/// This must run at EL1 before exceptions are enabled.
pub fn init() {
    unsafe {
        let vector_addr = &vector_table_el1 as *const _ as u64;
        core::arch::asm!(
        "msr vbar_el1, {}",
        in(reg) vector_addr,
        options(nostack)
        );
    }
}

const SOURCES: [&str; 4] = [
    "Current EL (SP0)",
    "Current EL (SPx)",
    "Lower EL (AArch64)",
    "Lower EL (AArch32)",
];

const KINDS: [&str; 4] = ["Synchronous", "IRQ", "FIQ", "SError"];

/// Decoded cause: what the syndrome register says, plus which vector slot fired.
struct Cause<'a> {
    ctx: &'a ExceptionContext,
    source: usize,
    kind: usize,
}

impl fmt::Display for Cause<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let esr = self.ctx.esr;
        let ec = (esr >> 26) & 0x3F;
        let iss = esr & 0x01FF_FFFF;

        // The syndrome is only meaningful for synchronous exceptions and SError.
        if self.kind == 0 {
            match ec {
                0x20 | 0x21 => write!(f, "{}", Fsc(iss & 0x3F))?,
                0x24 | 0x25 => write!(f, "{} on {}", Fsc(iss & 0x3F), if iss & (1 << 6) != 0 { "write" } else { "read" })?,
                0x11 | 0x15 => write!(f, "svc #{}", iss & 0xFFFF)?,
                0x3C => write!(f, "brk #{}", iss & 0xFFFF)?,
                _ => write!(f, "ESR EC {:#04x}, ISS {:#09x}", ec, iss)?,
            }
        } else {
            write!(f, "ESR {:#x}", esr)?;
        }

        write!(
            f,
            "  [{} / {}]",
            SOURCES.get(self.source).unwrap_or(&"Unknown"),
            KINDS.get(self.kind).unwrap_or(&"Unknown"),
        )
    }
}

/// A fault status code (DFSC / IFSC) from the ISS of an abort.
struct Fsc(u64);

impl fmt::Display for Fsc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = self.0 & 3;
        match self.0 {
            0x00..=0x03 => write!(f, "address size fault, level {level}"),
            0x04..=0x07 => write!(f, "translation fault, level {level}"),
            0x08..=0x0B => write!(f, "access flag fault, level {level}"),
            0x0C..=0x0F => write!(f, "permission fault, level {level}"),
            0x10 => write!(f, "synchronous external abort"),
            0x14..=0x17 => write!(f, "external abort on translation table walk, level {level}"),
            0x21 => write!(f, "alignment fault"),
            0x30 => write!(f, "TLB conflict abort"),
            other => write!(f, "unknown fault status {other:#04x}"),
        }
    }
}

/// The register dump, laid out like the other architectures' (three per row).
struct Registers<'a>(&'a ExceptionContext);

impl fmt::Display for Registers<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.0;
        for (row, chunk) in c.gpr.chunks(3).enumerate() {
            for (i, value) in chunk.iter().enumerate() {
                write!(f, "x{:<2} {}  ", row * 3 + i, Hex(*value as usize))?;
            }
            writeln!(f)?;
        }
        writeln!(f, "lr  {}  sp  {}", Hex(c.lr as usize), Hex(c.sp as usize))?;
        writeln!(f, "esr {}  far {}  spsr {}", Hex(c.esr as usize), Hex(c.far as usize), Hex(c.spsr as usize))
    }
}

#[unsafe(no_mangle)]
/// Handles an exception forwarded by the assembly vector table.
///
/// The exception is reported through [`crate::services::fault`], which halts
/// permanently. `source` and `kind` are vector-table indices passed by
/// `vectors.s`; unknown values are rendered as `Unknown`.
pub extern "C" fn rust_exception_handler(ctx: &ExceptionContext, source: usize, kind: usize) -> ! {
    let ec = (ctx.esr >> 26) & 0x3F;

    let name = match kind {
        0 => exception_class_name(ec),
        1 => "Unexpected IRQ",
        2 => "Unexpected FIQ",
        3 => "SError interrupt",
        _ => "Unknown exception",
    };
    // FAR is only defined for aborts, PC alignment faults and watchpoints.
    let far_valid = kind == 0 && matches!(ec, 0x20 | 0x21 | 0x22 | 0x24 | 0x25 | 0x34 | 0x35);

    let cause = Cause { ctx, source, kind };
    let registers = Registers(ctx);
    let report = FaultReport {
        class: FaultClass::Exception,
        kind: name,
        cause: &cause,
        pc: Some(ctx.elr as usize),
        fault_addr: far_valid.then_some(ctx.far as usize),
        registers: Some(&registers),
        frame_pointer: ctx.gpr[29] as usize,
        link_register: Some(ctx.lr as usize),
        stack_hint: ctx.sp as usize,
        code: ec as u8,
    };
    fault::fatal(&report)
}

fn exception_class_name(ec: u64) -> &'static str {
    match ec {
        0x00 => "Undefined instruction (EC 0x00, unknown reason)",
        0x01 => "Trapped WFI/WFE",
        0x03 => "Trapped AArch32 CP15 access",
        0x07 => "Trapped FP/SIMD access",
        0x0E => "Illegal execution state",
        0x11 => "SVC (AArch32)",
        0x15 => "SVC (Supervisor Call)",
        0x16 => "HVC",
        0x17 => "SMC",
        0x18 => "System register trap",
        0x20 => "Instruction Abort (Lower EL)",
        0x21 => "Instruction Abort (Current EL)",
        0x22 => "PC alignment fault",
        0x24 => "Data Abort (Lower EL)",
        0x25 => "Data Abort (Current EL)",
        0x26 => "SP alignment fault",
        0x28 | 0x2C => "Floating-point exception",
        0x2F => "SError",
        0x30 | 0x31 => "Breakpoint",
        0x32 | 0x33 => "Software step",
        0x34 | 0x35 => "Watchpoint",
        0x3C => "BRK instruction",
        _ => "Reserved / unknown exception class",
    }
}
