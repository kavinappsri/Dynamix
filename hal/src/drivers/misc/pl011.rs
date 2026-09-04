//! ARM PrimeCell PL011 polling serial driver.

use crate::services::mmio::Mmio;
use crate::services::sync::StaticCell;
use crate::{Serial, register_serial_driver};

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
    /// It must remain exclusively usable by this driver while the returned
    /// value is used.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let uart = unsafe { hal::aarch64::pl011::Pl011Uart::new(0x0900_0000) };
    /// ```
    pub const unsafe fn new(base: usize) -> Self {
        Self {
            // SAFETY: upheld by this constructor's contract.
            registers: unsafe { Mmio::new(base) },
        }
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

fn init_pl011_uart(base: usize, _node: Option<&dyn crate::driver_traits::device_tree::DeviceTreeNode>) -> &'static dyn Serial {
    static INSTANCE: StaticCell<Pl011Uart> = StaticCell::uninit();
    INSTANCE.get_or_init(|| {
        // SAFETY: device-tree addresses are identity mapped during early boot.
        unsafe { Pl011Uart::new(base) }
    })
}

register_serial_driver!(PL011_UART, "arm,pl011", init_pl011_uart);
