//! fatal fault reporting.


use core::fmt::{self, Display, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::driver_traits::serial::active_serial;
use crate::services::{cpu, exceptions};

/// Frames printed at most, including the faulting `pc`.
const MAX_FRAMES: usize = 24;

/// How far above the faulting stack pointer a frame record may be before the
/// walk gives up
const MAX_STACK_SPAN: usize = 256 * 1024;

/// A code address is only believed if it lies this close to the kernel image
/// (measured from `dynamix_fault_anchor`). The image is a few MiB; anything
/// further away is a stale or garbage stack value, not a return address.
#[cfg(not(target_arch = "x86_64"))]
const IMAGE_WINDOW: usize = 32 * 1024 * 1024;

/// Most candidates the x86_64 stack scan prints.
#[cfg(target_arch = "x86_64")]
const MAX_SCAN_HITS: usize = 48;

/// How much stack the x86_64 scan looks at.
#[cfg(target_arch = "x86_64")]
const MAX_SCAN_BYTES: usize = 8 * 1024;

/// A pointer-sized value printed as `0x` plus zero-padded hex digits, so the
/// report lines up on every architecture.
pub struct Hex(pub usize);

impl Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#0w$x}", self.0, w = 2 + 2 * size_of::<usize>())
    }
}

/// What kind of death this is; decides the fallback framebuffer colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultClass {
    /// A CPU exception (data abort, page fault, undefined instruction, ...).
    Exception,
    /// A Rust panic.
    Panic,
}

impl FaultClass {
    const fn color(self) -> u32 {
        match self {
            FaultClass::Exception => 0x00_FF_00_FF, // magenta
            FaultClass::Panic => 0x00_FF_00_00,     // red
        }
    }
}

/// Everything the reporter needs to know about a fault. Each architecture
/// fills this from its saved exception context.
pub struct FaultReport<'a> {
    pub class: FaultClass,
    /// Short name of the fault, e.g. `#PF page fault` or `Data Abort`.
    pub kind: &'a str,
    /// Human readable cause, decoded from the architecture's syndrome registers.
    pub cause: &'a dyn Display,
    /// The instruction that faulted, if there is one.
    pub pc: Option<usize>,
    /// The memory address involved, where the architecture reports one.
    pub fault_addr: Option<usize>,
    /// A register dump; each line must end in a newline.
    pub registers: Option<&'a dyn Display>,
    /// Frame pointer of the faulting code, the start of the backtrace chain.
    /// Zero if unknown.
    pub frame_pointer: usize,
    /// Link register, for architectures where it is the caller of a leaf function.
    pub link_register: Option<usize>,
    /// Any address on (or next to) the faulting stack, usually its stack
    /// pointer. Used to sanity check frame pointers before dereferencing them.
    pub stack_hint: usize,
    /// One byte identifying the fault (vector, exception class, ...). Shown as
    /// eight black/white squares on the framebuffer.
    pub code: u8,
}

/// Where fault output goes: the active serial device, or, before one was
/// probed, whatever raw output the architecture can do blindly.
struct Out;

impl Write for Out {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        match active_serial() {
            Some(uart) => uart.write_str(s),
            None => s.bytes().for_each(emergency_putc),
        }
        Ok(())
    }
}

/// Output with no driver and no setup.
///
/// x86_64 has a fixed place to look: COM1 in port I/O space. The arm boards
/// have no such thing: a UART there is MMIO at a board specific address that
/// is only mapped once its driver has probed it, and touching it earlier would
/// fault again. So before serial is up, those targets can only paint the
/// framebuffer.
#[cfg(target_arch = "x86_64")]
fn emergency_putc(byte: u8) {
    use crate::x86_64::port::{inb, outb};
    const COM1: u16 = 0x3F8;
    const LINE_STATUS: u16 = 5;
    const TX_EMPTY: u8 = 1 << 5;

    // SAFETY: COM1 is a plain I/O port; a machine without one just ignores it.
    unsafe {
        for _ in 0..10_000 {
            if inb(COM1 + LINE_STATUS) & TX_EMPTY != 0 {
                break;
            }
        }
        outb(COM1, byte);
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn emergency_putc(_byte: u8) {}

/// Marker symbol: its run-time address is printed with every report so a
/// symbolizer can compute the image's load slide.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn dynamix_fault_anchor() {}

static FATAL: AtomicBool = AtomicBool::new(false);

/// Reports a fault that cannot be recovered from and halts the machine.
///
/// A fault that happens while this runs does not recurse: the second entry
/// prints one line through the raw output path and halts.
pub fn fatal(report: &FaultReport<'_>) -> ! {
    cpu::irq_disable();

    if FATAL.swap(true, Ordering::AcqRel) {
        let _ = Out.write_str("\n!!! fault while reporting a fault, halting !!!\n");
        cpu::hlt_frvr();
    }

    // Paint first: it works without serial, and it still shows up if the
    // serial output below hangs.
    paint(report.class, report.code);

    print(report);
    let _ = writeln!(Out, "\nSystem halted.");
    cpu::hlt_frvr()
}

/// Prints the report without halting (for faults the kernel survives, such as
/// an x86 breakpoint).
pub fn print(report: &FaultReport<'_>) {
    let mut out = Out;
    let _ = writeln!(out, "\n=== DYNAMIX FAULT: {} ===", report.kind);
    let _ = writeln!(out, "cause : {}", report.cause);
    if let Some(pc) = report.pc {
        let _ = writeln!(out, "pc    : {}", Hex(pc));
    }
    if let Some(addr) = report.fault_addr {
        let _ = writeln!(out, "addr  : {}", Hex(addr));
    }
    if let Some(registers) = report.registers {
        let _ = writeln!(out, "registers:");
        let _ = write!(out, "{registers}");
    }
    print_backtrace(&mut out, report);
    let _ = writeln!(out, "anchor: {}", Hex(dynamix_fault_anchor as *const () as usize));
}

#[cfg(not(target_arch = "x86_64"))]
fn in_image(address: usize) -> bool {
    address.abs_diff(dynamix_fault_anchor as *const () as usize) < IMAGE_WINDOW
}

fn print_backtrace(out: &mut Out, report: &FaultReport<'_>) {
    let _ = writeln!(out, "backtrace (code addresses, symbolize with scripts/symbolize.py):");

    let mut n = 0;
    if let Some(pc) = report.pc {
        let _ = writeln!(out, "  #{n} {} pc", Hex(pc));
        n += 1;
    }
    if let Some(lr) = report.link_register {
        let _ = writeln!(out, "  #{n} {} lr", Hex(lr));
        n += 1;
    }

    // The UEFI target uses the Windows x64 ABI, where `rbp` does not reliably
    // point at a `[previous rbp, return address]` record, so the frame chain
    // cannot be followed there. Scan the stack instead.
    #[cfg(target_arch = "x86_64")]
    scan_stack(out, report, n);
    #[cfg(not(target_arch = "x86_64"))]
    walk_frame_chain(out, report, n);
}

/// Follows the frame-pointer chain. Frame records are
/// `[previous frame pointer, return address]` on aarch64 and on ARM (in ARM
/// state, with `r11`) when the code is built with frame pointers.
#[cfg(not(target_arch = "x86_64"))]
fn walk_frame_chain(out: &mut Out, report: &FaultReport<'_>, mut n: usize) {
    let word = size_of::<usize>();
    let mut fp = report.frame_pointer;
    let mut first = true;
    while n < MAX_FRAMES {
        let plausible = fp != 0
            && fp % word == 0
            && fp.saturating_add(0x1000) >= report.stack_hint
            && fp.saturating_sub(report.stack_hint) <= MAX_STACK_SPAN;
        if !plausible {
            break;
        }

        // SAFETY: `fp` is aligned and within `MAX_STACK_SPAN` of the faulting
        // stack, so it is overwhelmingly likely to be mapped stack memory. If
        // it is not, the nested fault is caught by the `FATAL` guard.
        let (next, ret) = unsafe { ((fp as *const usize).read(), ((fp + word) as *const usize).read()) };
        if ret == 0 || !in_image(ret) {
            break;
        }
        // In a function that has set up its frame, the first return address
        // is the link register that was just printed.
        let duplicate = first && Some(ret) == report.link_register;
        first = false;
        if !duplicate {
            let _ = writeln!(out, "  #{n} {}", Hex(ret));
            n += 1;
        }
        if next <= fp {
            break;
        }
        fp = next;
    }
}

/// Prints every value on the stack that looks like a return address: inside
/// the kernel image, and directly after a `call` instruction. This is a
/// heuristic: it finds the real return addresses, plus stale ones left over
/// from earlier calls that happen to still be on the stack.
#[cfg(target_arch = "x86_64")]
fn scan_stack(out: &mut Out, report: &FaultReport<'_>, mut n: usize) {
    let Some((image_start, image_end)) = crate::x86_64::handoff::image_bounds() else {
        let _ = writeln!(out, "  (image bounds not known yet, no stack scan)");
        return;
    };
    let _ = writeln!(out, "  (stack scan: candidates only, nearest call first; may include stale entries)");

    let word = size_of::<usize>();
    let start = report.stack_hint & !(word - 1);
    let top = crate::x86_64::handoff::stack::stack_top();
    let end = if top >= start && top - start <= MAX_SCAN_BYTES { top } else { start + 2048 };

    let mut hits = 0;
    let mut at = start;
    while at < end && hits < MAX_SCAN_HITS {
        // SAFETY: within the boot stack (or a couple of KiB above the stack
        // pointer when that is not the boot stack); a bad read is caught by
        // the `FATAL` guard.
        let value = unsafe { (at as *const usize).read() };
        if follows_call(value, image_start, image_end) {
            let _ = writeln!(out, "  #{n} {} ?", Hex(value));
            n += 1;
            hits += 1;
        }
        at += word;
    }
}

/// Whether `address` lies inside the image and the bytes just before it are
/// the end of a `call` instruction, so it could be a return address.
#[cfg(target_arch = "x86_64")]
fn follows_call(address: usize, image_start: usize, image_end: usize) -> bool {
    // The last 8 bytes before a return address are examined, so all of them
    // must be inside the image.
    if address < image_start + 8 || address >= image_end {
        return false;
    }
    // SAFETY: `address - 8 .. address` is inside the mapped kernel image.
    let b = |back: usize| unsafe { ((address - back) as *const u8).read() };

    b(5) == 0xE8                                      // call rel32
        || (b(6) == 0xFF && b(5) == 0x15)             // call [rip + disp32]
        || (b(6) == 0xFF && (b(5) & 0xF8) == 0x90)    // call [reg + disp32]
        || (b(3) == 0xFF && (b(2) & 0xF8) == 0x50)    // call [reg + disp8]
        || (b(2) == 0xFF && (b(1) & 0xF8) == 0xD0)    // call reg
        || (b(3) == 0x41 && b(2) == 0xFF && (b(1) & 0xF8) == 0xD0) // call r8-r15
}

/// The colour fallback for machines with no serial: the whole framebuffer in
/// the class colour, and eight squares (most significant bit first) showing
/// `code`, white for 1 and black for 0.
fn paint(class: FaultClass, code: u8) {
    const WHITE: u32 = 0x00_FF_FF_FF;
    const BLACK: u32 = 0x00_00_00_00;
    const CELL: usize = 24;

    let Some(fb) = exceptions::fault_framebuffer() else { return };

    fb.fill(class.color());
    for bit in 0..8usize {
        let color = if (code >> (7 - bit)) & 1 != 0 { WHITE } else { BLACK };
        fb.fill_rect(8 + bit * (CELL + 4), 8, CELL, CELL, color);
    }
}

/// The kernel's panic handler body: reports through the same path as a CPU
/// exception, then halts. Call it from `#[panic_handler]`.
#[inline(never)]
pub fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    let marker = 0u8;
    let report = FaultReport {
        class: FaultClass::Panic,
        kind: "Kernel panic",
        cause: info,
        pc: None,
        fault_addr: None,
        registers: None,
        frame_pointer: current_frame_pointer(),
        link_register: None,
        stack_hint: core::hint::black_box(&marker) as *const u8 as usize,
        code: 0,
    };
    fatal(&report)
}

#[inline(always)]
fn current_frame_pointer() -> usize {
    let fp: usize;
    // SAFETY: reads a register, nothing else.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, rbp", out(reg) fp, options(nomem, nostack, preserves_flags));
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("mov {}, x29", out(reg) fp, options(nomem, nostack, preserves_flags));
        #[cfg(target_arch = "arm")]
        core::arch::asm!("mov {}, r11", out(reg) fp, options(nomem, nostack, preserves_flags));
    }
    fp
}

/// Ways to kill the kernel on purpose, to check that the fault path works on
/// the architecture at hand. Each one is the same request on every
/// architecture; how it is carried out is the HAL's business.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashKind {
    /// A Rust `panic!`.
    Panic,
    /// A read from an address that is not mapped.
    UnmappedRead,
    /// Execution of a permanently undefined instruction.
    UndefinedInstruction,
}

/// Triggers the requested crash. Never returns.
pub fn crash(kind: CrashKind) -> ! {
    match kind {
        CrashKind::Panic => panic!("deliberate crash test"),
        CrashKind::UnmappedRead => {
            // The very top page of the address space: not mapped by any of
            // the kernel's page tables.
            let address = (usize::MAX & !0xFFF) as *const u32;
            // SAFETY: not safe, on purpose: this is the fault being tested.
            let value = unsafe { core::ptr::read_volatile(address) };
            panic!("unmapped read unexpectedly returned {value:#x}")
        }
        CrashKind::UndefinedInstruction => {
            // SAFETY: not safe, on purpose: this is the fault being tested.
            unsafe {
                #[cfg(target_arch = "x86_64")]
                core::arch::asm!("ud2");
                #[cfg(target_arch = "aarch64")]
                core::arch::asm!(".inst 0x00000000");
                #[cfg(target_arch = "arm")]
                core::arch::asm!(".inst 0xe7f000f0");
            }
            panic!("undefined instruction did not fault")
        }
    }
}
