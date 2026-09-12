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
/// Embedded boot-logo rendering.
pub mod logo;

use crate::console::Console;
use core::panic::PanicInfo;
use hal::services::dtb::Dtb;
use hal::{active_serial};
use hal::driver_traits::{serial, framebuffer};
use hal::services::pmm::PmmError;

unsafe extern "C" {
    static _kernel_vma_start: u8;
    static _kernel_vma_end: u8;
}

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
    hal::services::exceptions::init();

    //Initialize MMU services
    hal::services::mmu::init().expect("MMU already initialized!");

    //Map dtb
    const DTB_MAP_WINDOW: usize = 2 * 1024 * 1024;
    hal::services::mmu::map_normal(dtb_ptr, DTB_MAP_WINDOW).expect("DTB mapping failed");

    // Parse DTB
    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!(),
    };

    //set up allocator
    let (ram_base, ram_size) = dtb
        .find_node("memory")
        .and_then(|node| node.reg())
        .map(|(base, size)| (base as usize, size as usize))
        .expect("No usable memory node in DTB");

    let kernel_start = hal::services::mmu::va_to_pa(unsafe { &_kernel_vma_start as *const u8 as usize });
    let kernel_end = hal::services::mmu::va_to_pa(unsafe { &_kernel_vma_end as *const u8 as usize });

    let reserved = [
        hal::services::pmm::ReservedRange {
            start: kernel_start,
            end: kernel_end,
        },
        hal::services::pmm::ReservedRange {
            start: dtb_ptr,
            end: dtb_ptr + DTB_MAP_WINDOW,
        },
    ];

    hal::services::pmm::init(ram_base, ram_size, &reserved).expect("Physical memory manager init failed");

    // Set up UART & Console
    let uart = serial::probe_serial(&dtb).expect("No compatible serial driver in DTB");
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8; 128];

    console.writeln("Dynamix v0.1.0\n");
    console.writeln("[+] DTB, UART Found");

    // Set up the clocks, if the device has one
    match hal::driver_traits::clocks::probe_cclocks(&dtb) {
        Ok(_) => console.writeln("[+] Clock controller found"),
        Err(_) => console.writeln("[ ] No compatible clock controller in DTB"),
    }

    // Set up a display through a compiled HAL framebuffer driver.
    console.writeln("[+] Discovering framebuffer");
    let display = framebuffer::probe_framebuffer(&dtb).expect("No compatible framebuffer driver in DTB");


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
                    hal::services::power::init(method.as_str().unwrap());
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
    //hal::services::timer::delay_ms(5000);   // <-------
    //hal::services::power::system_off();         // <-------

    // Main Command Loop
    loop {
        console.write("dynamix <~ ");
        let length = console.read_ln(&mut command_buffer);
        console.writeln("");

        match &command_buffer[..length] {
            b"shut" => {
                console.writeln("Shutting down");
                hal::services::power::system_off();
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
                hal::services::timer::delay_ms(5000);

                hal::services::power::system_off();
            }
            b"pmm-stats" => {
                let stats = hal::services::pmm::stats();
                match stats {
                    Err(e) => {
                        console.writeln("ERROR!");
                        console.write("PmmError::");
                        match e {
                            PmmError::AlreadyInitialized => {console.writeln("Already initialized");}
                            PmmError::NotReady => {console.writeln("Not ready");}
                            PmmError::ReservedRegionOutOfRange => {console.writeln("Reserved region out of range");}
                            PmmError::MappingFailed(_) => {console.writeln("MappingFailed");}
                            PmmError::OrderTooLarge => {console.writeln("Order too large");}
                            PmmError::TooManyReservedRanges => {console.writeln("TooMany reserved ranges");}
                            PmmError::InvalidFree => {console.writeln("Invalid free");}
                        }
                    }
                    Ok((free, total)) => {

                        fn usize_to_str(mut num: usize, buf: &mut [u8; 20]) -> &str {
                            if num == 0 { return "0"; }
                            let mut i = 20;
                            while num > 0 {
                                i -= 1;
                                buf[i] = b'0' + (num % 10) as u8;
                                num /= 10;
                            }
                            core::str::from_utf8(&buf[i..]).unwrap()
                        }

                        let mut buf = [0u8;20];

                        console.writeln("Pmm stats:");
                        console.write("free frames: ");
                        console.writeln(usize_to_str(free, &mut buf));
                        console.write("total frames: ");
                        console.writeln(usize_to_str(total, &mut buf));
                    }
                }
            }
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}
