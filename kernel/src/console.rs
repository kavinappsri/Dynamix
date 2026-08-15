use hal::Serial;

pub struct Console<'a> {
    uart: &'a dyn Serial,
}

impl<'a> Console<'a> {
    pub const fn new(uart: &'a dyn Serial) -> Self {
        Self { uart }
    }

    pub fn write(&self, text: &str) {
        self.uart.write_str(text);
    }

    pub fn writeln(&self, text: &str) {
        self.uart.write_str(text);
        self.uart.write_str("\n");
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        for byte in bytes {
            self.uart.write_byte(*byte);
        }
    }

    pub fn write_hex(&self, hex: u64) {
        let hex_chars = b"0123456789ABCDEF";
        for i in (0..16).rev() {
            let byte = ((hex >> (i * 4)) & 0xF) as usize;
            self.write_bytes(&[hex_chars[byte]]);
        }
    }

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
