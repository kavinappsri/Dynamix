//! Interfaces shared by serial devices.

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
