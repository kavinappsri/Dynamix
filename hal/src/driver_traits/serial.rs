//! Interfaces shared by serial devices.

use crate::services::dtb::Dtb;
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

/// Returns the serial device selected by [`crate::probe_serial`], if boot discovery
/// completed successfully.
///
/// Exception and panic reporting can use this without coupling the kernel to a
/// particular UART implementation.
pub fn active_serial() -> Option<&'static dyn Serial> {
    ACTIVE_SERIAL.get().copied()
}


define_probe!("__start_serial_drivers", "__stop_serial_drivers", probe_serial, Serial, |driver| { ACTIVE_SERIAL.get_or_init(|| driver); } );

