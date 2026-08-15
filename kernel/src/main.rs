#![no_std]
#![no_main]

pub mod console;
pub mod dtb;
pub mod exceptions;
pub mod fw_cfg;
pub mod power;
pub mod ramfb;
pub mod timer;

use core::arch::global_asm;
use core::panic::PanicInfo;

use crate::console::Console;
use crate::dtb::Dtb;
use crate::fw_cfg::FwCfg;
use crate::ramfb::RamFb;

use hal::{aarch64::pl011::Pl011Uart, probe_serial, Serial};

global_asm!(include_str!("boot.s"));

const WIDTH: usize = 1024;
const HEIGHT: usize = 600;

#[repr(C, align(4))]
struct Framebuffer([u32; WIDTH * HEIGHT]);
unsafe impl Sync for Framebuffer {}

static FRAMEBUFFER: Framebuffer = Framebuffer([0; WIDTH * HEIGHT]);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // SAFETY: QEMU virt maps the early PL011 UART at this fixed address.
    let uart = unsafe { Pl011Uart::new(0x09000000) };
    uart.write_str("PANIC: Dynamix stage fright\n");
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
pub extern "C" fn rust_main(dtb_ptr: usize) -> ! {
    // Initialize Exception Vector table
    exceptions::init();

    // Parse DTB
    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!(),
    };

    // Set up UART & Console
    let uart = probe_serial(&dtb).expect("No compatible serial driver in DTB");
    let mut console = Console::new(uart);
    let mut command_buffer = [0u8; 128];

    console.writeln("Dynamix v0.1.0\n");
    console.writeln("[+] DTB, UART Found");

    // Set up FW_CFG
    console.writeln("[+] Attempting to find fw_cfg");
    let fw_cfg_node = dtb
        .find_compatible("qemu,fw-cfg-mmio")
        .expect("fw_cfg Not Found");
    let (fw_cfg_base, _) = fw_cfg_node.reg().expect("Failed to read fw_cfg reg");
    console.writeln("[+] Found fw_cfg");
    let fw_cfg = FwCfg::new(fw_cfg_base as usize);

    // Set up Framebuffer
    console.writeln("[+] Setting up Framebuffer");
    let fb_ptr = FRAMEBUFFER.0.as_ptr() as *mut u32;
    let display = RamFb::new(&fw_cfg, fb_ptr, WIDTH, HEIGHT).expect("Failed to initialize RamFb");

    display.fill_screen(0x00_00_00_00);

    display.draw_logo();

    //Get frq
    let frq = timer::get_cntfrq();

    //Automatic d-shut - ONLY FOR DEV TESTING
    timer::delay_ms(5000, frq);

    power::shut();

    // Main Command Loop
    loop {
        console.write("dynamix <~ ");
        let length = console.read_ln(&mut command_buffer);
        console.writeln("");

        match &command_buffer[..length] {
            b"shutdown" => {
                console.writeln("Shutting down");
                power::shut();
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
            b"d-shut" => {
                timer::delay_ms(5000, frq);

                power::shut()
            }
            b"kavin" => {
                console.writeln("Kavin is always better than gootam");
            }
            _ => console.writeln("Unrecognized Command"),
        }

        command_buffer.fill(0);
    }
}
