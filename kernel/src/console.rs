//! A small line-oriented console over a polling serial device.

use core::char;
use alloc::string::String;

use hal::Serial;

/// A borrowed serial console with basic text, hexadecimal, and line input I/O.
pub struct Console<'a> {
    uart: &'a dyn Serial,
}

impl<'a> Console<'a> {
    /// Creates a console that sends and receives bytes through `uart`.
    pub const fn new(uart: &'a dyn Serial) -> Self {
        Self { uart }
    }

    /// Writes `text` without appending a line ending.
    pub fn write(&self, text: &str) {
        self.uart.write_str(text);
    }

    /// Writes `text` followed by a newline.
    pub fn writeln(&self, text: &str) {
        self.uart.write_str(text);
        self.uart.write_str("\n");
    }

    /// Writes every byte in `bytes` verbatim.
    pub fn write_bytes(&self, bytes: &[u8]) {
        for byte in bytes {
            self.uart.write_byte(*byte);
        }
    }

    /// Writes `hex` as sixteen uppercase hexadecimal digits without a prefix.
    pub fn write_hex(&self, hex: u64) {
        let hex_chars = b"0123456789ABCDEF";
        for i in (0..16).rev() {
            let byte = ((hex >> (i * 4)) & 0xF) as usize;
            self.write_bytes(&[hex_chars[byte]]);
        }
    }

    pub fn read_line(&mut self, buffer: &mut String) {
        loop {
            let byte = self.uart.read_byte();

            match byte {
                b'\n' | b'\r' => {
                    self.uart.write_byte(b'\n');
                    break;
                }
                b'\x08' | b'\x7F' => {
                    if !buffer.is_empty() {
                        buffer.pop();

                        self.uart.write_byte(b'\x08');
                        self.uart.write_byte(b' ');
                        self.uart.write_byte(b'\x08');
                    }
                }
                _ => {
                    buffer.push(char::from(byte));
                    self.uart.write_byte(byte);
                }
            }
        }
    }
}

impl<'a> core::fmt::Write for Console<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.uart.write_str(s);
        Ok(())
    }
}

