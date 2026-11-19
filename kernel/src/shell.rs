//! UART shell


use core::fmt::Write;
use alloc::string::String;
use alloc::vec::Vec;

use crate::console::Console;
use crate::logo;
use crate::global::HEAP;

use hal::driver_traits::framebuffer::Framebuffer;
use hal::services::fault::{self, CrashKind};

pub enum Command {
    Fill,
    Shutdown,
    Stats,
    Crash,
    Dmesg,
    Sleep,
    Uptime,
    Unknown,
}

/// Runs the command loop forever.
pub fn run(console: &mut Console, display: &dyn Framebuffer) -> ! {
    let mut input_buffer = String::new();

    loop {
        // Deferred work (interrupt bottom halves) runs here, between commands.
        // `read_line` below blocks, so nothing runs while a prompt is waiting.
        hal::services::work::run_pending();

        // Cleared at the top, not the bottom: the usage branches below
        // `continue`, and a stale half-typed line must not leak into the next one.
        input_buffer.clear();

        console.write("dynamix <~ ");
        console.read_line(&mut input_buffer);
        console.writeln("");

        input_buffer = input_buffer.to_lowercase();
        let args: Vec<&str> = input_buffer.split_whitespace().collect();

        let cmd = match args.first() {
            Some(&"fill") => Command::Fill,
            Some(&"shutdown") => Command::Shutdown,
            Some(&"stats") => Command::Stats,
            Some(&"crash") => Command::Crash,
            Some(&"dmesg") => Command::Dmesg,
            Some(&"sleep") => Command::Sleep,
            Some(&"uptime") => Command::Uptime,
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
                    "work" => {
                        let _ = writeln!(console, "{:#?}", hal::services::work::stats());
                    }
                    "timer" => {
                        let _ = writeln!(console, "{:#?}", hal::services::timer::stats());
                    }
                    "klog" => {
                        let _ = writeln!(console, "{:#?}", hal::services::klog::stats());
                    }
                    _ => {
                        let _ = writeln!(console, "Unknown service");
                    }
                }
            }
            Command::Dmesg => {
                let dumped = hal::services::klog::dump(console, 0);
                let stats = hal::services::klog::stats();
                if dumped.lost > 0 || stats.dropped > 0 {
                    let _ = writeln!(console, "-- {} older records were overwritten --", stats.dropped);
                }
            }
            Command::Sleep => {
                let Some(ms) = args.get(1).and_then(|ms| ms.parse::<u64>().ok()) else {
                    let _ = writeln!(console, "Usage: sleep <milliseconds>");
                    continue;
                };
                let start = hal::services::timer::now_ns();
                hal::services::timer::sleep(core::time::Duration::from_millis(ms));
                let elapsed_us = hal::services::timer::now_ns().wrapping_sub(start) / 1_000;
                let _ = writeln!(console, "Slept {}.{:03} ms", elapsed_us / 1_000, elapsed_us % 1_000);
            }
            Command::Uptime => {
                let ms = hal::services::timer::now_ns() / 1_000_000;
                let _ = writeln!(console, "Up {}.{:03} s", ms / 1_000, ms % 1_000);
            }
            Command::Crash => {
                let kind = match args.get(1).copied() {
                    Some("panic") => CrashKind::Panic,
                    Some("read") => CrashKind::UnmappedRead,
                    Some("undef") => CrashKind::UndefinedInstruction,
                    _ => {
                        let _ = writeln!(console, "Usage: crash <panic|read|undef>");
                        continue;
                    }
                };
                let _ = writeln!(console, "Crashing on purpose ({kind:?})...");
                fault::crash(kind);
            }
            Command::Unknown => {
                let _ = writeln!(console, "Unknown command");
            }
        }
    }
}
