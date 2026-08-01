#![no_std]
#![no_main]

pub mod uart;
pub mod console;

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
    //UART & Console initializing
    let uart = uart::Uart::new(0x09000000);
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8;128];

    console.write("Type Something:");
    let _length = console.readln(&mut command_buffer);
    console.writeln("Test");
    console.write_bytes(&command_buffer);




    loop {}
}