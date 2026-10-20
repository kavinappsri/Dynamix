#![no_std]
#![no_main]

//! Dynamix's kernel image.
/// Polled console input and output over a HAL serial device.
pub mod console;
/// Embedded boot-logo rendering.
pub mod logo;
/// The UART command shell.
pub mod shell;

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use hal::Framebuffer;
use hal::driver_traits::serial::SerialWriter;
use hal::services::addr::{PhysAddr, PhysRange};
use hal::services::boot_info::BootInfo;
use hal::services::cpu;
use hal::services::framebuffer::FirmwareFramebuffer;

/// Colors are `0x00RRGGBB`; the framebuffer's pixel format is applied by the HAL.
const COLOR_BACKGROUND: u32 = 0x00_10_14_28;
const COLOR_RED: u32 = 0x00_FF_00_00;
const COLOR_GREEN: u32 = 0x00_00_FF_00;
const COLOR_BLUE: u32 = 0x00_00_00_FF;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // A panic while reporting a panic must not recurse.
    static PANICKING: AtomicBool = AtomicBool::new(false);

    if !PANICKING.swap(true, Ordering::AcqRel) {
        if let Some(uart) = hal::active_serial() {
            let _ = writeln!(SerialWriter(uart), "\n== DYNAMIX stage fright ==\n{info}");
        }

        // The HAL keeps the framebuffer registered for fatal faults; a panic is one.
        if let Some(framebuffer) = hal::services::exceptions::fault_framebuffer() {
            framebuffer.fill(COLOR_RED);
        }
    }

    cpu::hlt_frvr()
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

/// Dev aid: power off this many milliseconds after boot completes, which
/// exercises the timer and power services end to end. `None` halts instead.
const DEV_AUTO_SHUTDOWN_MS: Option<u64> = None;

#[unsafe(no_mangle)]
pub fn rust_main(boot_info: &'static mut BootInfo<'static>) -> ! {
    // Exceptions must be initialized first
    hal::services::exceptions::init();

    hal::services::mmu::init().expect("MMU init failed");

    // Serial as early as the MMU allows, so later failures can be reported.
    let hardware = boot_info.hardware.as_ref();
    let serial = hardware.and_then(|hardware| hal::probe_serial(hardware).ok());

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

    // SoC support drivers the display may depend on; a miss is fine.
    if let Some(hardware) = hardware {
        let _ = hal::driver_traits::power_domain::probe_power_domains(hardware);
        let _ = hal::driver_traits::clocks::probe_clocks(hardware);
    }

    let power_ok = match hardware {
        Some(hardware) => hal::services::power::init(hardware).is_ok(),
        None => false,
    };

    if let Some(fb) = framebuffer {
        let status = BootStatus {
            memory_map: !boot_info.memory_regions.is_empty(),
            hardware: hardware.is_some(),
            pmm: matches!(hal::services::pmm::stats(), Ok((free, _)) if free > 0),
            power: power_ok,
        };
        draw_boot_marker(&fb, &status);
    }

    // Real display driver if the platform has one, otherwise whatever the
    // firmware left us. Probing only reports what it found; this is where
    // the kernel decides what a miss means.
    let display: Option<&'static dyn Framebuffer> = hardware
        .and_then(|hardware| hal::driver_traits::framebuffer::probe_framebuffer(hardware).ok())
        .or_else(|| hal::services::exceptions::fault_framebuffer().map(|fb| fb as &'static dyn Framebuffer));

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
