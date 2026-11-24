//! ARM PrimeCell PL011 polling serial driver.

use crate::Serial;
use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::serial;
use crate::services::device::Device;
use crate::services::mmio::Mmio;

/// A PL011 UART whose registers are directly mapped into the address space.
pub struct Pl011Uart {
    registers: Mmio,
}

impl Pl011Uart {
    const DATA: usize = 0x00;
    const FLAG: usize = 0x18;
    const FLAG_RECEIVE_EMPTY: u32 = 1 << 4;
    const FLAG_TRANSMIT_FULL: u32 = 1 << 5;

    /// Creates a UART from its register window.
    pub const fn new(registers: Mmio) -> Self {
        Self { registers }
    }
}

impl Serial for Pl011Uart {
    /// Polls until the transmit FIFO has capacity, then transmits `byte`.
    fn write_byte(&self, byte: u8) {
        while self.registers.read32(Self::FLAG) & Self::FLAG_TRANSMIT_FULL != 0 {}
        self.registers.write32(Self::DATA, u32::from(byte));
    }

    /// Polls until a received byte is available and returns it.
    fn read_byte(&self) -> u8 {
        while self.registers.read32(Self::FLAG) & Self::FLAG_RECEIVE_EMPTY != 0 {}
        self.registers.read32(Self::DATA) as u8
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let registers = dev.mem(0)?.mmio();
    serial::register(dev, Pl011Uart::new(registers));
    Ok(())
}

driver! {
    name: "pl011",
    matches: [Match::Fdt("arm,pl011")],
    probe: probe,
}
