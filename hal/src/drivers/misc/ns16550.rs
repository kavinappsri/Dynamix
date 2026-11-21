//! 16550-compatible polling UART over x86 port I/O (the PC's COM ports).

use crate::Serial;
use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::serial;
use crate::services::device::{Device, IoPorts};
use crate::x86_64::port::{inb, outb};

/// Upper bound on status polls per transmitted byte, so a machine with no
/// working UART cannot hang the fault and panic paths that print through it.
const TX_POLL_LIMIT: u32 = 100_000;

/// A 16550 UART whose registers sit in I/O port space.
pub struct Ns16550 {
    base: u16,
}

impl Ns16550 {
    const DATA: u16 = 0; // RBR / THR (DLL while DLAB is set)
    const INTERRUPT_ENABLE: u16 = 1; // IER (DLM while DLAB is set)
    const FIFO_CONTROL: u16 = 2;
    const LINE_CONTROL: u16 = 3;
    const MODEM_CONTROL: u16 = 4;
    const LINE_STATUS: u16 = 5;
    const SCRATCH: u16 = 7;

    const LSR_DATA_READY: u8 = 1 << 0;
    const LSR_TX_EMPTY: u8 = 1 << 5;

    /// Creates a UART from the I/O ports the firmware gave it.
    pub const fn new(ports: IoPorts) -> Self {
        Self { base: ports.base() }
    }

    /// Whether something that behaves like a 16550 answers at these ports.
    fn present(&self) -> bool {
        unsafe {
            outb(self.base + Self::SCRATCH, 0xA5);
            if inb(self.base + Self::SCRATCH) != 0xA5 {
                return false;
            }
            outb(self.base + Self::SCRATCH, 0x5A);
            inb(self.base + Self::SCRATCH) == 0x5A
        }
    }

    /// 115200 8N1, FIFOs on, interrupts off.
    fn configure(&self) {
        unsafe {
            outb(self.base + Self::INTERRUPT_ENABLE, 0x00);
            outb(self.base + Self::LINE_CONTROL, 0x80); // DLAB on
            outb(self.base + Self::DATA, 0x01); // divisor 1 -> 115200 baud
            outb(self.base + Self::INTERRUPT_ENABLE, 0x00);
            outb(self.base + Self::LINE_CONTROL, 0x03); // 8 bits, no parity, 1 stop
            outb(self.base + Self::FIFO_CONTROL, 0xC7); // enable and clear FIFOs
            outb(self.base + Self::MODEM_CONTROL, 0x0B); // DTR, RTS, OUT2
        }
    }
}

impl Serial for Ns16550 {
    /// Waits (boundedly) for the transmitter to be ready, then sends `byte`.
    /// The byte is dropped if the UART never becomes ready.
    fn write_byte(&self, byte: u8) {
        unsafe {
            for _ in 0..TX_POLL_LIMIT {
                if inb(self.base + Self::LINE_STATUS) & Self::LSR_TX_EMPTY != 0 {
                    break;
                }
                core::hint::spin_loop();
            }
            outb(self.base + Self::DATA, byte);
        }
    }

    /// Polls until a received byte is available and returns it.
    fn read_byte(&self) -> u8 {
        unsafe {
            while inb(self.base + Self::LINE_STATUS) & Self::LSR_DATA_READY == 0 {
                core::hint::spin_loop();
            }
            inb(self.base + Self::DATA)
        }
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let uart = Ns16550::new(dev.io(0)?);
    if !uart.present() {
        return Err(ProbeError::NotPresent);
    }
    uart.configure();
    serial::register(dev, uart);
    Ok(())
}

// ACPI names a 16550 `PNP0501`.
driver! {
    name: "ns16550",
    matches: [Match::Acpi("PNP0501")],
    probe: probe,
}
