//! Byte streams

use core::fmt;

use hal::Serial;

pub trait OutputStream: Sync {
    /// Writes one byte.
    fn write_byte(&self, byte: u8);

    /// Writes a UTF-8 string
    fn write_str(&self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }
}


pub trait InputStream: Sync {
    /// Blocks until a byte is available.
    fn read_byte(&self) -> u8;
}

/// A HAL serial port as a stream: both input and output.
#[derive(Clone, Copy)]
pub struct SerialStream<'a>(pub &'a dyn Serial);

impl OutputStream for SerialStream<'_> {
    fn write_byte(&self, byte: u8) {
        self.0.write_byte(byte);
    }

    fn write_str(&self, text: &str) {
        self.0.write_str(text);
    }
}

impl InputStream for SerialStream<'_> {
    fn read_byte(&self) -> u8 {
        self.0.read_byte()
    }
}


#[derive(Clone, Copy)]
pub struct Broadcast<'a> {
    sinks: &'a [Option<&'a dyn OutputStream>],
}

impl<'a> Broadcast<'a> {
    pub const fn new(sinks: &'a [Option<&'a dyn OutputStream>]) -> Self {
        Self { sinks }
    }
}

impl OutputStream for Broadcast<'_> {
    fn write_byte(&self, byte: u8) {
        for sink in self.sinks.iter().flatten() {
            sink.write_byte(byte);
        }
    }

    fn write_str(&self, text: &str) {
        for sink in self.sinks.iter().flatten() {
            sink.write_str(text);
        }
    }
}

/// Adapts an [`OutputStream`] to [`core::fmt::Write`], for `write!` and `writeln!`.
pub struct StreamWriter<'a>(pub &'a dyn OutputStream);

impl fmt::Write for StreamWriter<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write_str(text);
        Ok(())
    }
}