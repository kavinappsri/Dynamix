//! AArch64 EL1 exception-vector initialization and diagnostic reporting.

use core::arch::global_asm;
use crate::{Serial, active_serial};

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
    _pad: u64, // Padding to keep struct 16-byte aligned
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

#[unsafe(no_mangle)]
/// Handles an exception forwarded by the assembly vector table.
///
/// The function reports the captured state through the serial device selected
/// by HAL, then halts permanently. `source` and `kind` are vector-table
/// indices passed by `vectors.s`; unknown values are rendered as `Unknown`.
///
/// # Panics
///
/// This handler itself does not panic, but it never returns.
pub extern "C" fn rust_exception_handler(ctx: &ExceptionContext, source: usize, kind: usize) {
    let Some(uart) = active_serial() else { loop {} };

    uart.write_str("\n=== DYNAMIX STAGE FRIGHT ===\n");
    uart.write_str("Triggered by the exception vector table\n");
    uart.write_str("Source: ");
    uart.write_str(SOURCES.get(source).unwrap_or(&"Unknown"));
    uart.write_str("\nType:   ");
    uart.write_str(KINDS.get(kind).unwrap_or(&"Unknown"));
    uart.write_str("\n\n");

    let ec = ctx.esr >> 26;

    uart.write_str("Exception Class: ");
    uart.write_str(exception_class_name(ec));
    uart.write_str("\n\n");

    // Hex dump registers
    print_hex(uart, "ELR_EL1 (PC)", ctx.elr);
    print_hex(uart, "FAR_EL1 (Addr)", ctx.far);
    print_hex(uart, "ESR_EL1 (Syndrome)", ctx.esr);
    print_hex(uart, "SPSR_EL1", ctx.spsr);

    uart.write_str("\nSystem halted.\n");

    loop {}
}

fn print_hex(uart: &dyn Serial, label: &str, val: u64) {
    uart.write_str(label);
    uart.write_str(": 0x");

    let hex_chars = b"0123456789ABCDEF";
    for i in (0..16).rev() {
        let byte = ((val >> (i * 4)) & 0xF) as usize;
        uart.write_byte(hex_chars[byte]);
    }
    uart.write_str("\n");
}

fn exception_class_name(ec: u64) -> &'static str {
    match ec {
        0x00 => "Unknown",

        0x01 => "Trapped WFI/WFE",

        0x07 => "Floating Point / SIMD",

        0x15 => "SVC (Supervisor Call)",

        0x18 => "System Register Trap",

        0x20 => "Instruction Abort (Lower EL)",

        0x21 => "Instruction Abort (Current EL)",

        0x24 => "Data Abort (Lower EL)",

        0x25 => "Data Abort (Current EL)",

        0x2C => "Floating Point Exception",

        0x2F => "SError",

        _ => "Reserved / Unknown",
    }
}
