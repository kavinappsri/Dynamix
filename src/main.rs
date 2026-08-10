#![no_std]
#![no_main]

pub mod uart;
pub mod console;
pub mod exceptions;
pub mod dtb;

use core::panic::PanicInfo;
use core::arch::global_asm;
use crate::console::Console;
use crate::dtb::Dtb;

global_asm!(include_str!("boot.s"));

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let uart = uart::Uart::new(0x09000000);
    uart.write_string("PANIC: Dynamix stage fright\n");
    loop {}
}


#[unsafe(no_mangle)]
pub extern "C" fn rust_main(dtb_ptr: usize) -> ! {
    // 1. Initialize Exception Vector table
    exceptions::init();

    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!(),
    };

    let uart_node = match dtb.find_compatible("arm,pl011") {
        Some(node) => node,
        None => panic!(),
    };

    // 5. Inspect 'reg' Address Resolution
    let base_address = match uart_node.reg() {
        Some((addr, _size)) => addr as usize,
        None => panic!(),
    };


    let uart = uart::Uart::new(base_address);
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8; 128];



    loop {
        console.write("dynamix <~ ");
        let length = console.read_ln(&mut command_buffer);
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
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}