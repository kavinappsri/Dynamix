#![no_std]
#![no_main]

//! Dynamix's kernel image.

extern crate alloc;

/// Byte streams (UART, screen, ...) that the console reads from and writes to.
pub mod io;
/// Line-oriented console over an input stream and an output stream.
pub mod console;
/// Embedded boot-logo rendering.
pub mod logo;
/// The 8x8 bitmap font used for on-screen text.
pub mod font;
/// Text output on a framebuffer.
pub mod text_display;
/// The command shell.
pub mod shell;
mod global;
pub mod selftest;

use core::fmt::Write;
use core::panic::PanicInfo;

use crate::global::*;

use hal::Framebuffer;
use hal::services::addr::{PhysAddr, PhysRange};
use hal::services::boot_info::BootInfo;
use hal::services::cpu;
use selftest::{heap_test, irq_test, time_test, work_test};

use crate::io::{Broadcast, OutputStream, SerialStream, StreamWriter};
use crate::text_display::TextDisplay;

/// Every panic ends up in the HAL's fault reporter, which prints the same
/// report as a CPU exception would (message, location, backtrace) and halts.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    hal::services::fault::panic(info)
}

const DEV_AUTO_SHUTDOWN_MS: Option<u64> = None;
const DEV_HEAP_SELFTEST: bool = true;
const DEV_IRQ_SELFTEST: bool = true;
const DEV_TIME_SELFTEST: bool = true;
const DEV_WORK_SELFTEST: bool = true;

/// Prints a self-test verdict.
fn report_selftest<E: core::fmt::Display>(log: &mut impl Write, name: &str, outcome: Result<(), E>) {
    let _ = match outcome {
        Ok(()) => writeln!(log, "[+] {} self-test passed", name),
        Err(why) => writeln!(log, "[!] {} self-test failed: {}", name, why),
    };
}

#[unsafe(no_mangle)]
pub fn rust_main(boot_info: &'static mut BootInfo<'static>) -> ! {
    // Exceptions must be initialized first
    hal::services::exceptions::init();

    hal::services::mmu::init().expect("MMU init failed");

    let hardware = boot_info.hardware.as_ref();

    // Map the firmware framebuffer first, so a fault can still be reported on screen
    if let Some(fb) = boot_info.framebuffer {
        let window = PhysRange::new(PhysAddr::new(fb.base()), fb.size());
        if hal::services::mmu::map_normal_nc(window).is_ok() {
            hal::services::exceptions::set_fault_framebuffer(fb);
        }
    }

    // Physical memory: keep the kernel image and the hardware description
    // out of the free pool.
    let mut reserved = [PhysRange::EMPTY; 2];
    let mut reserved_count = 0;

    let kernel_image = hal::services::mmu::kernel_image_range();
    if !kernel_image.is_empty() {
        reserved[reserved_count] = kernel_image;
        reserved_count += 1;
    }
    if let Some(range) = boot_info.hardware_reserved_range {
        reserved[reserved_count] = range;
        reserved_count += 1;
    }

    hal::services::pmm::init(boot_info.memory_regions.as_slice(), &reserved[..reserved_count]).expect("Physical memory manager init failed");

    let hardware_init = hardware.map(|hardware| hardware.init());

    // Every device the firmware describes gets its driver here: serial ports,
    // the interrupt controller, clocks, power domains and displays alike. The
    // call is the same on every architecture.
    let device_report = match (hardware, &hardware_init) {
        (Some(hardware), Some(Ok(()))) => Some(hal::services::device::init(hardware)),
        _ => None,
    };

    // The kernel's output: the console serial port (the first one that was
    // bound) and the display, as text. Either may be missing; whatever is
    // there gets every message from here on.
    let uart = hal::active_serial().map(SerialStream);
    let display: Option<&'static dyn Framebuffer> = hal::driver_traits::framebuffer::primary_framebuffer()
        .or_else(|| hal::services::exceptions::fault_framebuffer().map(|fb| fb as &'static dyn Framebuffer));
    let screen = display.and_then(TextDisplay::new);

    let sinks = [
        uart.as_ref().map(|uart| uart as &dyn OutputStream),
        screen.as_ref().map(|screen| screen as &dyn OutputStream),
    ];
    let output = Broadcast::new(&sinks);
    let mut log = StreamWriter(&output);

    if let (Some(hardware), Some(init)) = (hardware, &hardware_init) {
        let _ = match init {
            Ok(()) => writeln!(log, "[+] Hardware description ready: {}", hardware),
            Err(err) => writeln!(log, "[!] Hardware description failed: {}", err),
        };
    }
    if let Some(report) = &device_report {
        let _ = match report {
            Ok(report) => writeln!(
                log,
                "[+] Devices: {} found, {} bound, {} failed",
                report.devices, report.bound, report.failed
            ),
            Err(err) => writeln!(log, "[!] Device discovery failed: {}", err),
        };
    }

    let memory_regions = boot_info.memory_regions.as_slice().len();
    let _ = writeln!(log, "[{}] Memory map: {} regions", if memory_regions > 0 { '+' } else { '!' }, memory_regions);
    let _ = match hal::services::pmm::stats() {
        Ok((free, total)) => writeln!(log, "[+] Physical memory: {} of {} frames free", free, total),
        Err(err) => writeln!(log, "[!] Physical memory manager unavailable: {:?}", err),
    };

    if DEV_HEAP_SELFTEST {
        heap_test::run(&HEAP);
        let _ = writeln!(log, "[+] Kernel heap self-test passed: {:?}", HEAP.stats());
    }

    if DEV_TIME_SELFTEST {
        report_selftest(&mut log, "Timer", time_test::run());
    }

    // Interrupt controller, bound with the rest of the devices above
    match hal::services::interrupts::init() {
        Ok(()) => {
            if DEV_IRQ_SELFTEST {
                report_selftest(&mut log, "Interrupt", irq_test::run());
            }
        }
        Err(err) => {
            let _ = writeln!(log, "[!] No interrupt controller: {:?}", err);
        }
    }

    if DEV_WORK_SELFTEST {
        report_selftest(&mut log, "Work queue", work_test::run());
    }

    if let Some(hardware) = hardware {
        if let Err(err) = hardware.enable() {
            let _ = writeln!(log, "[!] Hardware description enable failed: {}", err);
        }
    }

    // The interrupt-driven timers
    if hal::services::interrupts::is_ready() {
        let _ = match hal::services::timer::init() {
            Ok(()) => writeln!(log, "[+] Interrupt timer ready"),
            Err(err) => writeln!(log, "[!] Interrupt timer unavailable: {:?}", err),
        };
        cpu::irq_enable();
    }

    let _ = if hal::services::power::is_available() {
        writeln!(log, "[+] Power control available")
    } else {
        writeln!(log, "[!] Power control unavailable")
    };

    if let Some(ms) = DEV_AUTO_SHUTDOWN_MS {
        hal::services::timer::delay_ms(ms);
        hal::services::power::system_off();
    }

    // The shell reads from the serial port; the display, if there is one, only
    // serves its `fill` command (its text output already comes through `output`).
    // Other input devices, a keyboard say, would go here as an `InputStream`.
    if let Some(input) = &uart {
        let mut console = console::Console::new(input, &output);
        console.writeln("Dynamix active");

        shell::run(&mut console, display)
    }

    cpu::hlt_frvr()
}