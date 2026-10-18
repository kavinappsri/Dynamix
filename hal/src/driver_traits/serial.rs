//! Interfaces shared by serial devices.

use crate::services::sync::StaticCell;
use crate::define_probe;

static ACTIVE_SERIAL: StaticCell<&'static dyn Serial> = StaticCell::uninit();

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
    ///
    /// # Examples
    ///
    /// ```ignore
    /// uart.write_str("booting...\n");
    /// ```
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

/// Returns the serial device selected by [`crate::probe_serial`], if boot discovery
/// completed successfully.
pub fn active_serial() -> Option<&'static dyn Serial> {
    ACTIVE_SERIAL.get().copied()
}


define_probe!(Serial, probe_serial, |driver| { ACTIVE_SERIAL.get_or_init(|| driver); } );

