//! ARMv7-A exception vector initialization and diagnostic reporting.

use core::arch::global_asm;
use core::fmt;

use crate::services::fault::{self, FaultClass, FaultReport, Hex};

global_asm!(include_str!("../asm/armv7/armv7_vectors.s"), options(raw));

unsafe extern "C" {
    static vector_table: u8;
    /// Gives the abort, undefined and FIQ modes their own stacks.
    fn armv7_init_fault_stacks();
}

#[repr(C)]
#[derive(Debug)]
/// Register state captured by the assembly exception vector before Rust runs.
///
/// Layout is 80 bytes (8-byte aligned) matching `vectors.s`.
pub struct ExceptionContext {
    /// General-purpose registers `r0` through `r12`.
    pub gpr: [u32; 13],
    /// Stack Pointer (`sp`) of the interrupted mode.
    pub sp: u32,
    /// Link Register (`lr`) of the interrupted mode.
    pub lr: u32,
    /// Address of the instruction that faulted.
    pub pc: u32,
    /// Saved Program Status Register (`spsr`).
    pub spsr: u32,
    /// Fault address: DFAR for a data abort, IFAR for a prefetch abort.
    pub far: u32,
    /// Fault status: DFSR for a data abort, IFSR for a prefetch abort.
    pub fsr: u32,
    _pad: u32,
}

const _: () = assert!(core::mem::size_of::<ExceptionContext>() == 80, "must match the frame in armv7_vectors.s");

/// Installs the fault stacks and the vector table address into CP15 `VBAR`.
pub fn init() {
    // SAFETY: called once, in SVC mode with interrupts masked, as the
    // routine requires.
    unsafe { armv7_init_fault_stacks() };

    unsafe {
        let vector_addr = &vector_table as *const _ as u32;
        core::arch::asm!(
        "mcr p15, 0, {}, c12, c0, 0",
        in(reg) vector_addr,
        options(nostack)
        );
    }
}

/// Called by the IRQ vector once the interrupted context is saved. Lets the
/// interrupt service ask the GIC what fired and run its handlers.
#[unsafe(no_mangle)]
extern "C" fn armv7_irq_entry() {
    crate::services::interrupts::handle_interrupt(None);
}

const VECTOR_NAMES: [&str; 8] = [
    "Reset",
    "Undefined Instruction",
    "Software Interrupt (SVC)",
    "Prefetch Abort",
    "Data Abort",
    "Reserved",
    "IRQ Interrupt",
    "FIQ Interrupt",
];

const VECTOR_UNDEFINED: usize = 1;
const VECTOR_PREFETCH_ABORT: usize = 3;
const VECTOR_DATA_ABORT: usize = 4;

/// Decoded cause of an abort, from the short-descriptor fault status.
struct Cause<'a> {
    ctx: &'a ExceptionContext,
    vector: usize,
}

impl fmt::Display for Cause<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fsr = self.ctx.fsr;
        match self.vector {
            VECTOR_DATA_ABORT => write!(
                f,
                "{} on {}  [FSR {:#x}]",
                fault_status_name(fsr),
                if fsr & (1 << 11) != 0 { "write" } else { "read" },
                fsr,
            ),
            VECTOR_PREFETCH_ABORT => write!(f, "{}  [FSR {:#x}]", fault_status_name(fsr), fsr),
            VECTOR_UNDEFINED => write!(f, "undefined instruction"),
            _ => write!(f, "taken through the {} vector", VECTOR_NAMES.get(self.vector).unwrap_or(&"unknown")),
        }
    }
}

/// FS[4:0] = FSR[10] : FSR[3:0].
fn fault_status_name(fsr: u32) -> &'static str {
    match (fsr & 0xF) | (((fsr >> 10) & 1) << 4) {
        0b00001 => "alignment fault",
        0b00010 => "debug event",
        0b00011 => "access flag fault (section)",
        0b00100 => "instruction cache maintenance fault",
        0b00101 => "translation fault (section)",
        0b00110 => "access flag fault (page)",
        0b00111 => "translation fault (page)",
        0b01000 => "synchronous external abort",
        0b01001 => "domain fault (section)",
        0b01011 => "domain fault (page)",
        0b01100 => "external abort on translation table walk (level 1)",
        0b01101 => "permission fault (section)",
        0b01110 => "external abort on translation table walk (level 2)",
        0b01111 => "permission fault (page)",
        0b10110 => "asynchronous external abort",
        _ => "unknown fault status",
    }
}

/// The register dump, laid out like the other architectures' (three per row).
struct Registers<'a>(&'a ExceptionContext);

impl fmt::Display for Registers<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.0;
        for (row, chunk) in c.gpr.chunks(3).enumerate() {
            for (i, value) in chunk.iter().enumerate() {
                write!(f, "r{:<2} {}  ", row * 3 + i, Hex(*value as usize))?;
            }
            writeln!(f)?;
        }
        writeln!(f, "sp  {}  lr  {}", Hex(c.sp as usize), Hex(c.lr as usize))?;
        writeln!(f, "spsr {}  fsr {}  far {}", Hex(c.spsr as usize), Hex(c.fsr as usize), Hex(c.far as usize))
    }
}

#[unsafe(no_mangle)]
/// Handles an exception forwarded by the assembly vector table.
///
/// The exception is reported through [`crate::services::fault`], which halts
/// permanently.
pub extern "C" fn rust_exception_handler(ctx: &ExceptionContext, vector_id: usize) -> ! {
    let far_valid = matches!(vector_id, VECTOR_PREFETCH_ABORT | VECTOR_DATA_ABORT);

    let cause = Cause { ctx, vector: vector_id };
    let registers = Registers(ctx);
    let report = FaultReport {
        class: FaultClass::Exception,
        kind: VECTOR_NAMES.get(vector_id).copied().unwrap_or("Unknown exception"),
        cause: &cause,
        pc: Some(ctx.pc as usize),
        fault_addr: far_valid.then_some(ctx.far as usize),
        registers: Some(&registers),
        // r11 is the frame pointer in ARM state.
        frame_pointer: ctx.gpr[11] as usize,
        link_register: Some(ctx.lr as usize),
        stack_hint: ctx.sp as usize,
        code: vector_id as u8,
    };
    fault::fatal(&report)
}
