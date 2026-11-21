#![no_std]
#![no_main]

//! Dynamix's kernel image.

extern crate alloc;

/// Polled console input and output over a HAL serial device.
pub mod console;
/// Embedded boot-logo rendering.
pub mod logo;
/// The UART command shell.
pub mod shell;
mod global;
pub mod selftest;

use core::fmt::Write;
use core::panic::PanicInfo;

use crate::global::*;

use hal::Framebuffer;
use hal::driver_traits::serial::SerialWriter;
use hal::services::addr::{PhysAddr, PhysRange};
use hal::services::boot_info::BootInfo;
use hal::services::cpu;
use hal::services::framebuffer::FirmwareFramebuffer;
use selftest::{heap_test, irq_test, time_test, work_test};

/// Colors are `0x00RRGGBB`; the framebuffer's pixel format is applied by the HAL.
const COLOR_BACKGROUND: u32 = 0x00_10_14_28;
const COLOR_RED: u32 = 0x00_FF_00_00;
const COLOR_GREEN: u32 = 0x00_00_FF_00;
const COLOR_BLUE: u32 = 0x00_00_00_FF;

/// Every panic ends up in the HAL's fault reporter, which prints the same
/// report as a CPU exception would (message, location, backtrace) and halts.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    hal::services::fault::panic(info)
}

/// Which bring-up steps worked; drawn as the status squares of the boot marker.
struct BootStatus {
    memory_map: bool,
    hardware: bool,
    pmm: bool,
    power: bool,
}

/// Draws the "the kernel is running" marker.
///
/// A red, green and blue block, in that order from left to right (wrong order
/// means the pixel format is being mis-encoded), with a row of four status
/// squares beneath them, green when the step worked and red when it did not,
/// left to right: memory map, device tree, physical memory manager, power.
fn draw_boot_marker(framebuffer: &FirmwareFramebuffer, status: &BootStatus) {
    let (width, height) = (framebuffer.width(), framebuffer.height());
    let block = (width / 8).min(height / 4).max(1);

    let left = (width - 3 * block) / 2;
    let top = (height - block) / 2;
    let color = |ok: bool| if ok { COLOR_GREEN } else { COLOR_RED };

    framebuffer.fill(COLOR_BACKGROUND);

    framebuffer.fill_rect(left, top, block, block, COLOR_RED);
    framebuffer.fill_rect(left + block, top, block, block, COLOR_GREEN);
    framebuffer.fill_rect(left + 2 * block, top, block, block, COLOR_BLUE);

    let results = [status.memory_map, status.hardware, status.pmm, status.power];
    let size = block / 2;
    let gap = (3 * block).saturating_sub(results.len() * size) / (results.len() + 1);
    let row_top = top + block + block / 4;
    for (i, ok) in results.into_iter().enumerate() {
        framebuffer.fill_rect(left + gap + i * (size + gap), row_top, size, size, color(ok));
    }
}

const DEV_AUTO_SHUTDOWN_MS: Option<u64> = None;
const DEV_HEAP_SELFTEST: bool = true;
const DEV_IRQ_SELFTEST: bool = true;
const DEV_TIME_SELFTEST: bool = true;
const DEV_WORK_SELFTEST: bool = true;

/// Prints a self-test verdict to the serial port, if there is one.
fn report_selftest(serial: Option<&dyn hal::Serial>, name: &str, outcome: Result<(), &'static str>) {
    let Some(uart) = serial else { return };
    let _ = match outcome {
        Ok(()) => writeln!(SerialWriter(uart), "[+] {} self-test passed", name),
        Err(why) => writeln!(SerialWriter(uart), "[!] {} self-test FAILED: {}", name, why),
    };
}

#[unsafe(no_mangle)]
pub fn rust_main(boot_info: &'static mut BootInfo<'static>) -> ! {
    // Exceptions must be initialized first
    hal::services::exceptions::init();

    hal::services::mmu::init().expect("MMU init failed");

    let hardware = boot_info.hardware.as_ref();

    // Map an output device first, framebuffer in this case
    let mut framebuffer = None;
    if let Some(fb) = boot_info.framebuffer {
        let window = PhysRange::new(PhysAddr::new(fb.base()), fb.size());
        if hal::services::mmu::map_normal_nc(window).is_ok() {
            hal::services::exceptions::set_fault_framebuffer(fb);
            framebuffer = Some(fb);
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

    // The console is the first serial port that was bound.
    let serial = hal::active_serial();

    if let (Some(uart), Some(hardware), Some(init)) = (serial, hardware, &hardware_init) {
        let _ = match init {
            Ok(()) => writeln!(SerialWriter(uart), "[+] Hardware description ready: {}", hardware),
            Err(err) => writeln!(SerialWriter(uart), "[!] Hardware description failed: {}", err),
        };
    }
    if let (Some(uart), Some(report)) = (serial, &device_report) {
        let _ = match report {
            Ok(report) => writeln!(
                SerialWriter(uart),
                "[+] Devices: {} found, {} bound, {} failed",
                report.devices, report.bound, report.failed
            ),
            Err(err) => writeln!(SerialWriter(uart), "[!] Device discovery failed: {}", err),
        };
    }

    if DEV_HEAP_SELFTEST {
        heap_test::run(&HEAP);
        if let Some(uart) = serial {

            let _ = writeln!(SerialWriter(uart), "[+] Kernel heap self-test passed: {:?}", HEAP.stats());
        }
    }

    if DEV_TIME_SELFTEST {
        report_selftest(serial, "Timer", time_test::run());
    }

    // Interrupt controller, bound with the rest of the devices above
    match hal::services::interrupts::init() {
        Ok(()) => {
            if DEV_IRQ_SELFTEST {
                let outcome = irq_test::run();
                if let Some(uart) = serial {
                    match outcome {
                        Ok(()) => { let _ = writeln!(SerialWriter(uart), "[+] Interrupt self-test passed"); }
                        Err(why) => { let _ = writeln!(SerialWriter(uart), "[!] Interrupt self-test FAILED: {}", why); }
                    }
                }
            }
        }
        Err(err) => {
            if let Some(uart) = serial {
                let _ = writeln!(SerialWriter(uart), "[!] No interrupt controller: {:?}", err);
            }
        }
    }

    if DEV_WORK_SELFTEST {
        report_selftest(serial, "Work queue", work_test::run());
    }

    if let Some(hardware) = hardware {
        if let (Some(uart), Err(err)) = (serial, hardware.enable()) {
            let _ = writeln!(SerialWriter(uart), "[!] Hardware description enable failed: {}", err);
        }
    }

    // The interrupt-driven timers
    if hal::services::interrupts::is_ready() {
        let timer_ready = hal::services::timer::init();
        if let Some(uart) = serial {
            let _ = match &timer_ready {
                Ok(()) => writeln!(SerialWriter(uart), "[+] Interrupt timer ready"),
                Err(err) => writeln!(SerialWriter(uart), "[!] Interrupt timer unavailable: {:?}", err),
            };
        }
        cpu::irq_enable();
    }

    let power_ok = hal::services::power::is_available();

    if let Some(fb) = framebuffer {
        let status = BootStatus {
            memory_map: !boot_info.memory_regions.is_empty(),
            hardware: matches!(hardware_init, Some(Ok(()))),
            pmm: matches!(hal::services::pmm::stats(), Ok((free, _)) if free > 0),
            power: power_ok,
        };
        draw_boot_marker(&fb, &status);
    }

    // Real display driver if the platform has one
    let display: Option<&'static dyn Framebuffer> = hal::driver_traits::framebuffer::primary_framebuffer()
        .or_else(|| hal::services::exceptions::fault_framebuffer().map(|fb| fb as &'static dyn Framebuffer));

    if let (Some(uart), Some(display)) = (serial, display) {
        let mut console = console::Console::new(uart);
        display.fill(0x00_00_00_00);
        if logo::draw_centered(display).is_err() {
            let _ = writeln!(console, "[ ] Failed to draw boot logo");
        }
    } else if let Some(display) = display {
        display.fill(0x00_00_00_00);
        let _ = logo::draw_centered(display);
    } else if let Some(fb) = framebuffer {
        fb.fill(COLOR_RED);
    }

    if let Some(ms) = DEV_AUTO_SHUTDOWN_MS {
        hal::services::timer::delay_ms(ms);
        hal::services::power::system_off();
    }

    if let (Some(uart), Some(display)) = (serial, display) {
        let mut console = console::Console::new(uart);
        console.writeln("Dynamix active");



        shell::run(&mut console, display)
    }

    cpu::hlt_frvr()
}
