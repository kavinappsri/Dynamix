#![no_std]
#![no_main]

pub mod uart;

use core::panic::PanicInfo;
use core::arch::global_asm;

global_asm!(include_str!("boot.s"));

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let uart = uart::Uart::new(0x09000000);
    uart.write_string("PANIC: Dynamix stage fright\n");
    loop {}
}


#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    let uart = uart::Uart::new(0x09000000);
    uart.write_string("Hello World");
    loop {}
}