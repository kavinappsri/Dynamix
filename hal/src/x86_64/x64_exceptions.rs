//! x86_64 exception handling: the IDT, entry stubs and the fatal-fault reporter.

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::fmt::Write;

use crate::services::framebuffer::FirmwareFramebuffer;
use crate::driver_traits::serial::{SerialWriter, active_serial};
use crate::x86_64::gdt;

global_asm!(include_str!("../asm/x86_64/x64_vectors.s"));

#[allow(non_upper_case_globals)]
unsafe extern "C" {
    static isr_stub_table: [usize; 32];
}

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

fn report(ctx: &ExceptionContext) {
    // Serial may not be probed yet (or at all); the framebuffer paint still happens.
    let Some(uart) = active_serial() else { return };
    let mut out = SerialWriter(uart);
    let name = NAMES.get(ctx.vector as usize).copied().unwrap_or("unknown");
    let _ = writeln!(out, "\n=== CPU EXCEPTION: {} (vector {}) ===", name, ctx.vector);
    let _ = writeln!(out, "error code {:#x}", ctx.error_code);
    if ctx.vector == 14 {
        let e = ctx.error_code;
        let _ = writeln!(
            out,
            "cr2 {:#018x}  [{} {} {}{}]",
            read_cr2(),
            if e & 1 != 0 { "protection" } else { "not-present" },
            if e & 2 != 0 { "write" } else { "read" },
            if e & 4 != 0 { "user" } else { "supervisor" },
            if e & 16 != 0 { " ifetch" } else { "" },
        );
    }
    let _ = writeln!(out, "rip {:#018x}  cs {:#06x}  rflags {:#018x}", ctx.rip, ctx.cs, ctx.rflags);
    let _ = writeln!(out, "rsp {:#018x}  ss {:#06x}", ctx.rsp, ctx.ss);
    let _ = writeln!(out, "rax {:#018x}  rbx {:#018x}  rcx {:#018x}", ctx.rax, ctx.rbx, ctx.rcx);
    let _ = writeln!(out, "rdx {:#018x}  rsi {:#018x}  rdi {:#018x}", ctx.rdx, ctx.rsi, ctx.rdi);
    let _ = writeln!(out, "rbp {:#018x}  r8  {:#018x}  r9  {:#018x}", ctx.rbp, ctx.r8, ctx.r9);
    let _ = writeln!(out, "r10 {:#018x}  r11 {:#018x}  r12 {:#018x}", ctx.r10, ctx.r11, ctx.r12);
    let _ = writeln!(out, "r13 {:#018x}  r14 {:#018x}  r15 {:#018x}", ctx.r13, ctx.r14, ctx.r15);
}

fn paint(fb: &FirmwareFramebuffer, vector: u64) {
    const MAGENTA: u32 = 0xFF00FF;
    const WHITE: u32 = 0xFFFFFF;
    const BLACK: u32 = 0x000000;
    const CELL: usize = 24;

    fb.fill(MAGENTA);
    for bit in 0..8usize {
        let color = if vector >> (7 - bit) & 1 != 0 { WHITE } else { BLACK };
        fb.fill_rect(8 + bit * (CELL + 4), 8, CELL, CELL, color);
    }
}

/// Called by `isr_common` for every vector. Returns only for #BP.
#[unsafe(no_mangle)]
pub extern "efiapi" fn x86_exception_handler(ctx: &mut ExceptionContext) {
    report(ctx);
    if ctx.vector == VECTOR_BREAKPOINT {
        return;
    }
    if let Some(fb) = crate::services::exceptions::fault_framebuffer() {
        paint(fb, ctx.vector);
    }
    crate::services::cpu::hlt_frvr()
}
