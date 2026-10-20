//! The UART shell: a command loop over the serial console that can change the
//! display and shut the machine down.

use core::fmt::Write;

use crate::console::Console;
use crate::logo;
use hal::driver_traits::framebuffer::Framebuffer;

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

                match logo::draw_centered(display) {
                    Ok(()) => console.writeln("Displaying Dynamix Boot Logo"),
                    Err(e) => {
                        let _ = writeln!(console, "Cannot draw the boot logo: {e:?}");
                    }
                }
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
            b"pmm-stats" => match hal::services::pmm::stats() {
                Ok((free, total)) => {
                    let _ = writeln!(console, "Pmm stats:\nfree frames: {free}\ntotal frames: {total}");
                }
                Err(e) => {
                    let _ = writeln!(console, "ERROR!\n{e:?}");
                }
            },
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}
