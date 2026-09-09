//! DesignWare 8250 UART Driver

use crate::{register_driver, Serial};
use crate::services::mmio::Mmio;
use crate::services::sync::StaticCell;
use crate::driver_traits::driver::{Driver, DriverProbeError};

pub struct Dw8250Uart {
    base: Mmio,
}

impl Dw8250Uart {

    // 32-bit aligned register offsets (Standard 8250 offset << 2)
    const REG_RBR: usize = 0x00 << 2; // 0x00 - Receive Buffer (R)
    const REG_THR: usize = 0x00 << 2; // 0x00 - Transmit Holding (W)
    const REG_DLL: usize = 0x00 << 2; // 0x00 - Divisor Latch Low (W, DLAB=1)
    const REG_IER: usize = 0x01 << 2; // 0x04 - Interrupt Enable (R/W)
    const REG_DLM: usize = 0x01 << 2; // 0x04 - Divisor Latch High (W, DLAB=1)
    const REG_FCR: usize = 0x02 << 2; // 0x08 - FIFO Control (W)
    const REG_LCR: usize = 0x03 << 2; // 0x0C - Line Control (R/W)
    const REG_LSR: usize = 0x05 << 2; // 0x14 - Line Status (R)
    const REG_USR: usize = 0x1F << 2; // 0x7C - DesignWare UART Status Register (R)


    // Bitmasks
    const LCR_DLAB: u32 = 0x80;
    const LCR_8N1: u32 = 0x03;
    const LSR_DR: u32 = 0x01;
    const LSR_THRE: u32 = 0x20;
    const USR_BUSY: u32 = 0x01;
    const FCR_ENABLE: u32 = 0x01;
    const FCR_CLEAR_RCVR: u32 = 0x02;
    const FCR_CLEAR_XMIT: u32 = 0x04;

    fn wait_idle(&self) {
        let mut timeout = 1_000_000;
        while (self.base.read32(Self::REG_USR) & Self::USR_BUSY) != 0 && timeout > 0 {
            core::hint::spin_loop();
            timeout -= 1;
        }
    }

    pub unsafe fn new(base: usize, clk_frq: u32, baud_rate: u32) -> Self {
        let registers = unsafe { Mmio::new(base) };
        let uart = Self { base: registers };

        // Disable Interrupts
        uart.base.write32(Self::REG_IER, 0x00);

        // Safe divisor calculation
        let effective_baud = if baud_rate == 0 { 115200 } else { baud_rate };
        let divisor = if clk_frq == 0 {
            1
        } else {
            ((clk_frq + (8 * effective_baud)) / (16 * effective_baud)).max(1)
        };

        // Wait for idle hardware state
        uart.wait_idle();

        // Set DLAB bit in LCR (base + 0x0C)
        uart.base.write32(Self::REG_LCR, Self::LCR_DLAB);

        // Program Divisor Latch Low (base + 0x00) and High (base + 0x04)
        uart.base.write32(Self::REG_DLL, divisor & 0xFF);
        uart.base.write32(Self::REG_DLM, (divisor >> 8) & 0xFF);

        // Wait for idle state before clearing DLAB
        uart.wait_idle();

        // Lock divisor and set 8N1 frame format (base + 0x0C)
        uart.base.write32(Self::REG_LCR, Self::LCR_8N1);

        // Enable and reset FIFOs (base + 0x08)
        uart.base.write32(Self::REG_FCR, Self::FCR_ENABLE | Self::FCR_CLEAR_RCVR | Self::FCR_CLEAR_XMIT);

        uart
    }
}

impl Serial for Dw8250Uart {
    fn write_byte(&self, byte: u8) {
        while (self.base.read32(Self::REG_LSR) & Self::LSR_THRE) == 0 {
            core::hint::spin_loop();
        }
        self.base.write32(Self::REG_THR, byte as u32);
    }

    fn read_byte(&self) -> u8 {
        while (self.base.read32(Self::REG_LSR) & Self::LSR_DR) == 0 {
            core::hint::spin_loop();
        }
        (self.base.read32(Self::REG_RBR) & 0xFF) as u8
    }
}

fn init_dw_8250_uart(base: usize, tree: &crate::services::dtb::Dtb) -> Result<&'static dyn Serial, DriverProbeError> {
    static INSTANCE: StaticCell<Dw8250Uart> = StaticCell::uninit();

    let clock_hz = tree
        .find_compatible("rockchip,serial")
        .and_then(|n| n.property("clock-frequency")?.as_u32())
        .unwrap_or(24_000_000);

    Ok(INSTANCE.get_or_init(|| {
        unsafe {Dw8250Uart::new(base, clock_hz, 115200)}
    }))
}

register_driver!(DW_8250_UART, "rockchip,serial", init_dw_8250_uart, Serial);