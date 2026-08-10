// main.rs
#![no_std]
#![no_main]

pub mod uart;
pub mod console;
pub mod exceptions;
pub mod dtb;
pub mod fw_cfg;
pub mod ramfb;

use core::panic::PanicInfo;
use core::arch::global_asm;
use crate::console::Console;
use crate::dtb::Dtb;
use crate::fw_cfg::FwCfg;
use crate::ramfb::RamFb;

global_asm!(include_str!("boot.s"));

const WIDTH: usize = 1024;
const HEIGHT: usize = 600;

#[repr(C, align(4))]
struct Framebuffer([u32; WIDTH * HEIGHT]);
unsafe impl Sync for Framebuffer {}

static FRAMEBUFFER: Framebuffer = Framebuffer([0; WIDTH * HEIGHT]);

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let uart = uart::Uart::new(0x09000000);
    uart.write_string("PANIC: Dynamix stage fright\n");
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_main(dtb_ptr: usize) -> ! {
    // Initialize Exception Vector table
    exceptions::init();

    // Parse DTB
    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!(),
    };

    // Set up UART & Console
    let uart_node = dtb.find_compatible("arm,pl011").expect("UART node not found");
    let (uart_base, _) = uart_node.reg().expect("UART reg not found");
    let uart = uart::Uart::new(uart_base as usize);
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8; 128];

    console.writeln("Dynamix v0.1.0\n");
    console.writeln("[+] DTB, UART Found");

    // Set up FW_CFG
    console.writeln("[+] Attempting to find fw_cfg");
    let fw_cfg_node = dtb.find_compatible("qemu,fw-cfg-mmio").expect("fw_cfg Not Found");
    let (fw_cfg_base, _) = fw_cfg_node.reg().expect("Failed to read fw_cfg reg");
    console.writeln("[+] Found fw_cfg");
    let fw_cfg = FwCfg::new(fw_cfg_base as usize);

    // Set up Framebuffer
    console.writeln("[+] Setting up Framebuffer");
    let fb_ptr = FRAMEBUFFER.0.as_ptr() as *mut u32;
    let display = RamFb::new(&fw_cfg, fb_ptr, WIDTH, HEIGHT)
        .expect("Failed to initialize RamFb");

    display.fill_screen(0x00_00_00_00);

    display.draw_logo();

    // Main Command Loop
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
            b"logo" => {
                display.fill_screen(0x00_00_00_00);

                display.draw_logo();
                console.writeln("Displaying Dynamix Boot Logo");
            }
            b"red" => {
                display.fill_screen(0x00_FF_00_00);
                console.writeln("Screen set to Red");
            }
            b"green" => {
                display.fill_screen(0x00_00_FF_00);
                console.writeln("Screen set to Green");
            }
            b"blue" => {
                display.fill_screen(0x00_00_00_FF);
                console.writeln("Screen set to Blue");
            }
            b"white" => {
                display.fill_screen(0x00_FF_FF_FF);
                console.writeln("Screen set to White");
            }
            b"clear" | b"black" => {
                display.fill_screen(0x00_00_00_00);
                console.writeln("Screen cleared");
            }
            b"yellow" => {
                display.fill_screen(0x00_FF_FF_00);
                console.writeln("Screen set to Yellow");
            }
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}