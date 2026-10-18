#![no_std]
#![no_main]

//! Dynamix's kernel image.
/// Polled console input and output over a HAL serial device.
pub mod console;
/// Embedded boot-logo rendering.
pub mod logo;
/// The UART command shell.
pub mod shell;

use core::panic::PanicInfo;
use hal::Framebuffer;
use hal::services::boot_info::{BootInfo, RawFramebufferInfo};
use hal::services::cpu;
use hal::services::sync::StaticCell;

/// Colors are `0x00RRGGBB`; the framebuffer's pixel format is applied by the HAL.
const COLOR_BACKGROUND: u32 = 0x00_10_14_28;
const COLOR_RED: u32 = 0x00_FF_00_00;
const COLOR_GREEN: u32 = 0x00_00_FF_00;
const COLOR_BLUE: u32 = 0x00_00_00_FF;

/// The firmware framebuffer, kept so the panic handler can paint the screen
/// red even though it has no access to `BootInfo`.
static PANIC_FRAMEBUFFER: StaticCell<RawFramebufferInfo> = StaticCell::uninit();

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    report_over_serial(info);

    if let Some(framebuffer) = PANIC_FRAMEBUFFER.get() {
        framebuffer.fill(COLOR_RED);
    }

    cpu::hlt_frvr()
}

/// Writes the panic location to the active serial device, if there is one.
fn report_over_serial(info: &PanicInfo) {
    let Some(uart) = hal::active_serial() else { return };
    uart.write_str("==DYNAMIX stage fright ==\n");
    uart.write_str("LOCATION: ");

    if let Some(loc) = info.location() {
        uart.write_str("at ");
        uart.write_str(loc.file());
        uart.write_str(":");

        // Convert u32 line number to a stack &str
        let mut line_buf = [0u8; 10];
        let mut n = loc.line();
        let mut idx = line_buf.len();

        if n == 0 {
            idx -= 1;
            line_buf[idx] = b'0';
        } else {
            while n > 0 && idx > 0 {
                idx -= 1;
                line_buf[idx] = b'0' + (n % 10) as u8;
                n /= 10;
            }
        }

        if let Ok(line_str) = core::str::from_utf8(&line_buf[idx..]) {
            uart.write_str(line_str);
        }
    }
}

/// Which bring-up steps worked; drawn as the status squares of the boot marker.
struct BootStatus {
    memory_map: bool,
    device_tree: bool,
    pmm: bool,
    power: bool,
}

/// Draws the "the kernel is running" marker.
///
/// A red, green and blue block, in that order from left to right (wrong order
/// means the pixel format is being mis-encoded), with a row of four status
/// squares beneath them, green when the step worked and red when it did not,
/// left to right: memory map, device tree, physical memory manager, power.
fn draw_boot_marker(framebuffer: &RawFramebufferInfo, status: &BootStatus) {
    let (width, height) = (framebuffer.width as usize, framebuffer.height as usize);
    let block = (width / 8).min(height / 4).max(1);

    let left = (width - 3 * block) / 2;
    let top = (height - block) / 2;
    let color = |ok: bool| if ok { COLOR_GREEN } else { COLOR_RED };

    framebuffer.fill(COLOR_BACKGROUND);

    framebuffer.fill_rect(left, top, block, block, COLOR_RED);
    framebuffer.fill_rect(left + block, top, block, block, COLOR_GREEN);
    framebuffer.fill_rect(left + 2 * block, top, block, block, COLOR_BLUE);

    let results = [status.memory_map, status.device_tree, status.pmm, status.power];
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
pub fn rust_main(boot_info: BootInfo<'static>) -> ! {
    // Exceptions must be initialized first
    hal::services::exceptions::init();

    hal::services::mmu::init().expect("MMU init failed");

    // Serial as early as the MMU allows, so later failures can be reported.
    let tree = boot_info.device_tree.as_ref();
    let serial = tree.and_then(|tree| hal::probe_serial(tree).ok());

    // Map an output device first, framebuffer in this case
    let mut framebuffer = None;
    if let Some(fb) = boot_info.framebuffer {
        if hal::services::mmu::map_normal_nc(fb.base as usize, fb.size).is_ok() {
            PANIC_FRAMEBUFFER.get_or_init(|| fb);
            hal::services::exceptions::set_fault_framebuffer(fb);
            framebuffer = Some(fb);
        }
    }

    // Physical memory
    let (kernel_start, kernel_end) = hal::services::mmu::kernel_image_range();
    let mut reserved = [hal::services::pmm::ReservedRange { start: 0, end: 0 }; 2];
    let mut reserved_count = 0;

    if kernel_end > kernel_start {
        reserved[reserved_count] = hal::services::pmm::ReservedRange { start: kernel_start, end: kernel_end };
        reserved_count += 1;
    }
    if let Some((base, size)) = boot_info.device_tree_reserved_range {
        reserved[reserved_count] = hal::services::pmm::ReservedRange { start: base, end: base + size };
        reserved_count += 1;
    }


    hal::services::pmm::init(boot_info.memory_regions.as_slice(), &reserved[..reserved_count]).expect("Physical memory manager init failed");

    // SoC support drivers the display may depend on; a miss is fine.
    if let Some(tree) = tree {
        let _ = hal::driver_traits::power_domain::probe_power_domains(tree);
        let _ = hal::driver_traits::clocks::probe_clocks(tree);
    }

    let power_ok = match &boot_info.device_tree {
        Some(tree) => hal::services::power::init(tree).is_ok(),
        None => false,
    };

    if let Some(fb) = framebuffer {
        let status = BootStatus {
            memory_map: !boot_info.memory_regions.is_empty(),
            device_tree: boot_info.device_tree.is_some(),
            pmm: matches!(hal::services::pmm::stats(), Ok((free, _)) if free > 0),
            power: power_ok,
        };
        draw_boot_marker(&fb, &status);
    }

    // Real display driver if the platform has one, otherwise whatever the
    // firmware left us. Probing only reports what it found; this is where
    // the kernel decides what a miss means.
    let display: Option<&'static dyn Framebuffer> = tree
        .and_then(|tree| hal::driver_traits::framebuffer::probe_framebuffer(tree).ok())
        .or_else(|| PANIC_FRAMEBUFFER.get().map(|fb| fb as &'static dyn Framebuffer));

    if let Some(ms) = DEV_AUTO_SHUTDOWN_MS {
        hal::services::timer::delay_ms(ms);
        hal::services::power::system_off();
    }



    if let (Some(uart), Some(display)) = (serial, display) {
        let mut console = console::Console::new(uart);
        console.writeln("Dynamix kernel up.");
        shell::run(&mut console, display)
    }

    cpu::hlt_frvr()
}
