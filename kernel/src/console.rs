//! A small line-oriented console over an input stream and an output stream.

use core::char;
use alloc::string::String;

use crate::io::{InputStream, OutputStream};

/// Basic text, hexadecimal, and line input I/O over borrowed streams.
///
/// Where the bytes come from and go to is up to the streams: a UART, a
/// keyboard, a screen, or several outputs at once (see [`crate::io::Broadcast`]).
pub struct Console<'a> {
    input: &'a dyn InputStream,
    output: &'a dyn OutputStream,
}

impl<'a> Console<'a> {
    /// Creates a console that reads bytes from `input` and writes them to `output`.
    pub const fn new(input: &'a dyn InputStream, output: &'a dyn OutputStream) -> Self {
        Self { input, output }
    }

    /// Writes `text` without appending a line ending.
    pub fn write(&self, text: &str) {
        self.output.write_str(text);
    }

    /// Writes `text` followed by a newline.
    pub fn writeln(&self, text: &str) {
        self.output.write_str(text);
        self.output.write_str("\n");
    }

    /// Writes every byte in `bytes` verbatim.
    pub fn write_bytes(&self, bytes: &[u8]) {
        for byte in bytes {
            self.output.write_byte(*byte);
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

    /// Reads one line into `buffer`, echoing what is typed to the output and
    /// handling backspace. The line ending is not stored.
    pub fn read_line(&mut self, buffer: &mut String) {
        loop {
            let byte = self.input.read_byte();

            match byte {
                b'\n' | b'\r' => {
                    self.output.write_byte(b'\n');
                    break;
                }
                b'\x08' | b'\x7F' => {
                    if !buffer.is_empty() {
                        buffer.pop();

                        self.output.write_byte(b'\x08');
                        self.output.write_byte(b' ');
                        self.output.write_byte(b'\x08');
                    }
                }
                _ => {
                    buffer.push(char::from(byte));
                    self.output.write_byte(byte);
                }
            }
        }
    }
}

impl<'a> core::fmt::Write for Console<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.output.write_str(s);
        Ok(())
    }
}