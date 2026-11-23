//! Interfaces shared by serial devices.

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::registry::{Class, Entry, Iter};

static SERIALS: Class<dyn Serial> = Class::new();

/// A polling serial device.
///
/// Interrupt and configuration support belong in the driver once the kernel has
/// the infrastructure to use them.
pub trait Serial: Sync {
    /// Blocks until `byte` has been transmitted.
    fn write_byte(&self, byte: u8);

    /// Blocks until a byte has been received.
    fn read_byte(&self) -> u8;

    /// Writes a UTF-8 string without allocating.
    fn write_str(&self, value: &str) {
        for byte in value.bytes() {
            self.write_byte(byte);
        }
    }
}

/// Adapts a [`Serial`] device to [`core::fmt::Write`]
pub struct SerialWriter<'a>(pub &'a dyn Serial);

impl core::fmt::Write for SerialWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_str(s);
        Ok(())
    }
}

/// The console: the serial port firmware names as its own if it names one,
/// else the first that was bound.
///
/// Lock-free and allocation-free, so the panic and fault paths can use it.
pub fn active_serial() -> Option<&'static dyn Serial> {
    SERIALS.primary()
}

/// Every serial port that was bound, in the order they were bound.
pub fn serials() -> Iter<Entry<dyn Serial>> {
    SERIALS.iter()
}

/// Makes `entry`, one of [`serials`], the console.
pub fn prefer(entry: &'static Entry<dyn Serial>) {
    SERIALS.prefer(entry);
}

/// Called by a driver's probe to hand over the serial port it started on `device`.
pub fn register(device: &'static Device, uart: impl Serial + 'static) -> &'static dyn Serial {
    let uart: &'static dyn Serial = Box::leak(Box::new(uart));
    SERIALS.register(device, uart);
    uart
}
