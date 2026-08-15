//! ARM PrimeCell PL011 polling serial driver.

use crate::{mmio::Mmio, register_serial_driver, sync::StaticCell, Serial};

/// A PL011 UART whose registers are directly mapped into the address space.
pub struct Pl011Uart {
    registers: Mmio,
}

impl Pl011Uart {
    const DATA: usize = 0x00;
    const FLAG: usize = 0x18;
    const FLAG_RECEIVE_EMPTY: u32 = 1 << 4;
    const FLAG_TRANSMIT_FULL: u32 = 1 << 5;

    /// Creates a UART from its mapped register base.
    ///
    /// # Safety
    /// `base` must be a mapped PL011 register block.
    pub const unsafe fn new(base: usize) -> Self {
        Self {
            // SAFETY: upheld by this constructor's contract.
            registers: unsafe { Mmio::new(base) },
        }
    }
}

impl Serial for Pl011Uart {
    fn write_byte(&self, byte: u8) {
        while self.registers.read32(Self::FLAG) & Self::FLAG_TRANSMIT_FULL != 0 {}
        self.registers.write32(Self::DATA, u32::from(byte));
    }

    fn read_byte(&self) -> u8 {
        while self.registers.read32(Self::FLAG) & Self::FLAG_RECEIVE_EMPTY != 0 {}
        self.registers.read32(Self::DATA) as u8
    }
}

fn init_pl011_uart(base: usize) -> &'static dyn Serial {
    static INSTANCE: StaticCell<Pl011Uart> = StaticCell::uninit();
    INSTANCE.get_or_init(|| {
        // SAFETY: device-tree addresses are identity mapped during early boot.
        unsafe { Pl011Uart::new(base) }
    })
}

register_serial_driver!(PL011_UART, "arm,pl011", init_pl011_uart);
