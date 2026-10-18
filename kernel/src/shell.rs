//! The UART shell: a command loop over the serial console that can change the
//! display and shut the machine down.
//!
//! Parked behind the `drivers` feature until the driver subsystem is back on
//! every architecture (Phase 4, step 13).

use crate::console::Console;
use crate::logo;
use hal::driver_traits::framebuffer::Framebuffer;
use hal::services::pmm::PmmError;

/// Runs the command loop forever.
pub fn run(console: &mut Console, display: &dyn Framebuffer) -> ! {
    let mut command_buffer = [0u8; 128];

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
                            PmmError::NoMemory => {console.writeln("No memory");}
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
