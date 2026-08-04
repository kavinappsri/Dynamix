#![no_std]
#![no_main]

pub mod uart;
pub mod console;
pub mod exceptions;

use core::panic::PanicInfo;
use core::arch::global_asm;
use crate::console::Console;

global_asm!(include_str!("boot.s"));

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let uart = uart::Uart::new(0x09000000);
    uart.write_string("PANIC: Dynamix stage fright\n");
    loop {}
}


#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    //Initialize Exception Vector table
    exceptions::init();

    //UART & Console initializing
    let uart = uart::Uart::new(0x09000000);
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8;128];

    loop {
        console.write("dynamix <~ ");
        let length = console.readln(&mut command_buffer);
        console.writeln("");


        match &command_buffer[..length] {
            b"shutdown" => {
                console.writeln("Shutting down");
                unsafe {
                    core::arch::asm!(
                    "hvc #0",
                    in("x0") 0x84000008u64,
                    options(nostack, nomem)
                    );
                }
            }
            b"crash" => {
                unsafe {
                    let invalid_ptr = 0xDEAD_BEEF_0000_0000 as *mut u64;
                    invalid_ptr.write_volatile(42);
                }
            }
            _ => console.writeln("Unrecognized Command")
        }

        command_buffer.fill(0);
    }
}