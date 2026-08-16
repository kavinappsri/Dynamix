#![no_std]
#![no_main]

//! DynamixOS's freestanding AArch64 kernel image.
//!
//! Assembly transfers control to [`rust_main`] with a bootloader-supplied
//! Flattened Device Tree pointer. The kernel installs exception vectors, uses
//! the DTB to discover serial and platform services, then presents a small
//! serial console and framebuffer demonstration. The current platform path
//! uses QEMU `virt` services; the boot structure is not limited to that target.
//!
//! This binary is not a hosted Rust application. Build it for
//! `aarch64-unknown-none` and load it through a compatible AArch64 boot flow.

/// Polled console input and output over a HAL serial device.
pub mod console;
/// Minimal Flattened Device Tree parser used during boot discovery.
pub mod dtb;
/// AArch64 exception-vector setup and exception reporting.
pub mod exceptions;
/// Embedded boot-logo rendering.
pub mod logo;
/// Platform power-control primitives.
pub mod power;
/// AArch64 architectural timer access and busy-wait delays.
pub mod timer;

use core::arch::global_asm;
use core::panic::PanicInfo;

use crate::console::Console;
use crate::dtb::Dtb;
use hal::{active_serial, probe_framebuffer, probe_serial};

#[cfg(target_arch = "aarch64")]
global_asm!(include_str!("../aarch64/aarch64_boot.s"));

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let Some(uart) = active_serial() else { loop {} };
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

    loop {}
}

#[unsafe(no_mangle)]
/// Rust entry point called by the AArch64 assembly bootstrap.
///
/// `dtb_ptr` is the physical/identity-mapped address passed by the bootloader
/// in `x0`. On success this function does not return; it enters the console
/// loop and can request shutdown through the active platform path.
///
/// # Panics
///
/// Panics when the boot DTB is invalid, no supported serial device is found,
/// a compiled framebuffer driver cannot be initialized.
pub extern "C" fn rust_main(dtb_ptr: usize) -> ! {
    // Initialize Exception Vector table
    exceptions::init();

    // Parse DTB
    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!(),
    };

    // Set up UART & Console
    let uart = probe_serial(&dtb).expect("No compatible serial driver in DTB");
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8; 128];

    console.writeln("Dynamix v0.1.0\n");
    console.writeln("[+] DTB, UART Found");

    // Set up a display through a compiled HAL framebuffer driver.
    console.writeln("[+] Discovering framebuffer");
    let display = probe_framebuffer(&dtb).expect("No compatible framebuffer driver in DTB");

    display.fill(0x00_00_00_00);

    logo::draw_centered(display).expect("Failed to draw boot logo");

    //Get frq
    let frq = timer::get_cntfrq();

    //Automatic d-shut - ONLY FOR DEV TESTING
    timer::delay_ms(5000, frq);

    power::shut();

    // Main Command Loop
    loop {
        console.write("dynamix <~ ");
        let length = console.read_ln(&mut command_buffer);
        console.writeln("");

        match &command_buffer[..length] {
            b"shutdown" => {
                console.writeln("Shutting down");
                power::shut();
            }
            b"logo" => {
                display.fill(0x00_00_00_00);

                logo::draw_centered(display).expect("Failed to draw boot logo");
                console.writeln("Displaying Dynamix Boot Logo");
            }
            b"red" => {
                display.fill(0x00_FF_00_00);
                console.writeln("Screen set to Red");
            }
            b"green" => {
                display.fill(0x00_00_FF_00);
                console.writeln("Screen set to Green");
            }
            b"blue" => {
                display.fill(0x00_00_00_FF);
                console.writeln("Screen set to Blue");
            }
            b"white" => {
                display.fill(0x00_FF_FF_FF);
                console.writeln("Screen set to White");
            }
            b"clear" | b"black" => {
                display.fill(0x00_00_00_00);
                console.writeln("Screen cleared");
            }
            b"yellow" => {
                display.fill(0x00_FF_FF_00);
                console.writeln("Screen set to Yellow");
            }
            b"d-shut" => {
                timer::delay_ms(5000, frq);

                power::shut()
            }
            b"kavin" => {
                console.writeln("Kavin is always better than gootam");
            }
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}
