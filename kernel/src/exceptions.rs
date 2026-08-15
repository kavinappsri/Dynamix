use core::arch::global_asm;
use crate::uart::Uart;

global_asm!(include_str!("vectors.s"));

unsafe extern "C" {
    static vector_table_el1: u8;
}

#[repr(C)]
#[derive(Debug)]
pub struct ExceptionContext {
    pub gpr: [u64; 30], // x0 - x29
    pub lr: u64,        // x30 (Link Register)
    pub elr: u64,       // Exception Link Register (Instruction address that faulted)
    pub spsr: u64,      // Saved Program Status Register
    pub esr: u64,       // Exception Syndrome Register
    pub far: u64,       // Fault Address Register (Memory address involved in fault)
    _pad: u64,          // Padding to keep struct 16-byte aligned
}

/// Sets VBAR_EL1 to point to assembly vector table.
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

const KINDS: [&str; 4] = [
    "Synchronous",
    "IRQ",
    "FIQ",
    "SError",
];

#[unsafe(no_mangle)]
pub extern "C" fn rust_exception_handler(
    ctx: &ExceptionContext,
    source: usize,
    kind: usize,
) {
    let uart = Uart::new(0x09000000);

    uart.write_string("\n=== EXCEPTION TRIGGERED ===\n");
    uart.write_string("Source: ");
    uart.write_string(SOURCES.get(source).unwrap_or(&"Unknown"));
    uart.write_string("\nType:   ");
    uart.write_string(KINDS.get(kind).unwrap_or(&"Unknown"));
    uart.write_string("\n\n");

    let ec = ctx.esr >> 26;

    uart.write_string("Exception Class: ");
    uart.write_string(exception_class_name(ec));
    uart.write_string("\n\n");

    // Hex dump registers
    print_hex(&uart, "ELR_EL1 (PC)", ctx.elr);
    print_hex(&uart, "FAR_EL1 (Addr)", ctx.far);
    print_hex(&uart, "ESR_EL1 (Syndrome)", ctx.esr);
    print_hex(&uart, "SPSR_EL1", ctx.spsr);

    uart.write_string("\nSystem halted.\n");

    loop {}
}

fn print_hex(uart: &Uart, label: &str, val: u64) {
    uart.write_string(label);
    uart.write_string(": 0x");

    let hex_chars = b"0123456789ABCDEF";
    for i in (0..16).rev() {
        let byte = ((val >> (i * 4)) & 0xF) as usize;
        uart.write_byte(hex_chars[byte]);
    }
    uart.write_string("\n");
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