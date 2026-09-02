#![no_std]
#![no_main]

//! Dynamix's kernel image.
//!
//! Assembly transfers control to `rust_main` with the pointer to the
//! FDT in the first register. Currently the kernel shows its boot logo
//! and shuts down after 5s (d-shut), this can be removied, and doing that
//! expose a shell via the debug UART


/// Polled console input and output over a HAL serial device.
pub mod console;
/// Minimal Flattened Device Tree parser used during boot discovery.
pub mod dtb;
/// Embedded boot-logo rendering.
pub mod logo;

use crate::console::Console;
use crate::dtb::Dtb;
use core::panic::PanicInfo;
use hal::{active_serial, probe_framebuffer, probe_serial};

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
    hal::exceptions::init();

    //Initialize MMU services
    hal::mmu::init().expect("MMU already initialized!");

    //Map dtb
    const DTB_MAP_WINDOW: usize = 2 * 1024 * 1024;
    hal::mmu::map_normal(dtb_ptr, DTB_MAP_WINDOW).expect("DTB mapping failed");

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


    // Set up power
    match dtb.find_compatible("arm,psci") {
        None => {
            console.writeln("[ ] PSCI Not Found");
            panic!();
        }
        Some(psci) => {
            console.writeln("[+] PSCI Found");

            match psci.property("method") {
                Some(method) => {
                    hal::power::init(method.as_str().unwrap());
                    console.writeln("[+] PSCI Method found and set");
                }
                None => {
                    console.writeln("[ ] No PSCI Method Found:");
                    panic!();
                }
            }
        }
    }

    display.fill(0x00_00_00_00);

    logo::draw_centered(display).expect("Failed to draw boot logo");

    //Automatic d-shut - ONLY FOR DEV TESTING - Remove to get to cmd line
    hal::timer::delay_ms(5000);   // <-------
    hal::power::system_off();         // <-------

    // Main Command Loop
    loop {
        console.write("dynamix <~ ");
        let length = console.read_ln(&mut command_buffer);
        console.writeln("");

        match &command_buffer[..length] {
            b"shutdown" => {
                console.writeln("Shutting down");
                hal::power::system_off();
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
                hal::timer::delay_ms(5000);

                hal::power::system_off();
            }
            b"kavin" => {
                console.writeln("Kavin is always better than gootam");
            }
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}
