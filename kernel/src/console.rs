//! A small line-oriented console over a polling serial device.

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

    /// Reads one terminal line into `buffer` and returns the number of bytes stored.
    ///
    /// Input is echoed. Carriage return and newline terminate the line;
    /// backspace and delete remove the most recently stored byte. Additional
    /// input is discarded once `buffer` is full, until a line ending arrives.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut line = [0; 64];
    /// let length = console.read_ln(&mut line);
    /// let command = &line[..length];
    /// ```
    pub fn read_ln(&mut self, buffer: &mut [u8]) -> usize {
        let mut index = 0;

        loop {
            let byte = self.uart.read_byte();

            match byte {
                //Return
                b'\n' | b'\r' => {
                    self.uart.write_byte(b'\n');
                    break;
                }
                // Backspace (0x08 = BS / Ctrl+H, 0x7F = DEL / Backspace key)
                b'\x08' | b'\x7F' => {
                    if index > 0 {
                        index -= 1;

                        self.uart.write_byte(b'\x08');
                        self.uart.write_byte(b' ');
                        self.uart.write_byte(b'\x08');
                    }
                }
                _ => {
                    if index < buffer.len() {
                        buffer[index] = byte;
                        index += 1;

                        self.uart.write_byte(byte);
                    }
                }
            }
        }

        index
    }
}
