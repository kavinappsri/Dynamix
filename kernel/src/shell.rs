//! UART shell


use core::fmt::Write;
use alloc::string::String;
use alloc::vec::Vec;

use crate::console::Console;
use crate::logo;
use crate::global::HEAP;

use hal::driver_traits::framebuffer::Framebuffer;

pub enum Command {
    Fill,
    Shutdown,
    Stats,
    Unknown,
}

/// Runs the command loop forever.
pub fn run(console: &mut Console, display: &dyn Framebuffer) -> ! {
    let mut input_buffer = String::new();
    let mut lower = String::new();
    let mut args = Vec::<&str>::new();

    loop {
        console.write("dynamix <~ ");
        console.read_line(&mut input_buffer);
        console.writeln("");

        lower = input_buffer.to_lowercase();
        args = lower.split_whitespace().collect();

        let cmd = match args.first() {
            Some(&"fill") => Command::Fill,
            Some(&"shutdown") => Command::Shutdown,
            Some(&"stats") => Command::Stats,
            _ => Command::Unknown,
        };

        match cmd {
            Command::Fill => {
                let Some(color) = args.get(1) else {
                    let _ = writeln!(console, "Usage: fill <color>");
                    continue;
                };

                match *color {
                    "red" => {
                        display.fill(0x00_ff_00_00);
                    }
                    "green" => {
                        display.fill(0x00_00_ff_00);
                    }
                    "blue" => {
                        display.fill(0x00_00_00_ff);
                    }
                    "white" => {
                        display.fill(0x00_ff_ff_ff);
                    }
                    "black" => {
                        display.fill(0x00_00_00_00);
                    }
                    "logo" => {
                        display.fill(0x00_00_00_00);
                        let _ = logo::draw_centered(display);
                    }
                    _ => {
                        let _ = writeln!(console, "Unknown color");
                    }
                }
            }
            Command::Shutdown => {
                let _ = writeln!(console, "Shutting down... :)");
                hal::services::power::system_off();
            }
            Command::Stats => {
                let Some(service) = args.get(1) else {
                    let _ = writeln!(console, "Usage: stats <service>");
                    continue;
                };

                match *service {
                    "pmm" => {
                        match hal::services::pmm::stats() {
                            Ok((free, total)) => {
                                let _ = writeln!(console, "Pmm stats:\nfree frames: {free}\ntotal frames: {total}");
                            }
                            Err(e) => {
                                let _ = writeln!(console, "ERROR!\n{e:?}");
                            }
                        }
                    }
                    "heap" => {
                        let stats = HEAP.stats();
                        let _ = writeln!(console, "{:#?}", stats);
                    }
                    _ => {
                        let _ = writeln!(console, "Unknown service");
                    }
                }
            }
            Command::Unknown => {
                let _ = writeln!(console, "Unknown command");
            }
        }
        
        args.clear();
        lower.clear();
        input_buffer.clear();
    }
}
