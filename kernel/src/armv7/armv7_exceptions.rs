//! ARMv7-A exception vector initialization and diagnostic reporting.

use core::arch::global_asm;
use hal::{Serial, active_serial};

global_asm!(include_str!("../../asm/armv7/armv7_vectors.s"), options(raw));

unsafe extern "C" {
    static vector_table: u8;
}

#[repr(C)]
#[derive(Debug)]
/// Register state captured by the assembly exception vector before Rust runs.
///
/// Layout is 80 bytes (8-byte aligned) matching `vectors.s`.
pub struct ExceptionContext {
    /// General-purpose registers `r0` through `r12`.
    pub gpr: [u32; 13],
    /// Stack Pointer (`sp`).
    pub sp: u32,
    /// Link Register (`lr`).
    pub lr: u32,
    /// Program Counter (`pc` / saved execution address).
    pub pc: u32,
    /// Saved Program Status Register (`spsr`).
    pub spsr: u32,
    /// Data Fault Address Register (`dfar`).
    pub far: u32,
    /// Data Fault Status Register (`dfsr`).
    pub fsr: u32,
    _pad: u32,
}

/// Installs the vector table address into CP15 `VBAR`.
pub fn init() {
    unsafe {
        let vector_addr = &vector_table as *const _ as u32;
        core::arch::asm!(
        "mcr p15, 0, {}, c12, c0, 0",
        in(reg) vector_addr,
        options(nostack)
        );
    }
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

#[unsafe(no_mangle)]
/// Handles an exception forwarded by the assembly vector table.
pub extern "C" fn rust_exception_handler(ctx: &ExceptionContext, vector_id: usize) {
    let Some(uart) = active_serial() else { loop {} };

    uart.write_str("\n=== DYNAMIX STAGE FRIGHT ===\n");
    uart.write_str("Triggered by the exception vector table\n");
    uart.write_str("Vector: ");
    uart.write_str(VECTOR_NAMES.get(vector_id).unwrap_or(&"Unknown"));
    uart.write_str("\n\n");

    // Dump key system state
    print_hex(uart, "PC      ", ctx.pc as u64);
    print_hex(uart, "LR      ", ctx.lr as u64);
    print_hex(uart, "SP      ", ctx.sp as u64);
    print_hex(uart, "SPSR    ", ctx.spsr as u64);
    print_hex(uart, "DFAR    ", ctx.far as u64);
    print_hex(uart, "DFSR    ", ctx.fsr as u64);

    uart.write_str("\nSystem halted.\n");

    loop {}
}

fn print_hex(uart: &dyn Serial, label: &str, val: u64) {
    uart.write_str(label);
    uart.write_str(": 0x");

    let hex_chars = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        let byte = ((val >> (i * 4)) & 0xF) as usize;
        uart.write_byte(hex_chars[byte]);
    }
    uart.write_str("\n");
}