use crate::uart::Uart;

pub struct Console {
    uart: Uart,
}

impl Console {
    pub const fn new(uart: Uart) -> Self {
        Self {
            uart,
        }
    }

    pub fn write(&self, text: &str) {
        self.uart.write_string(text);
    }

    pub fn writeln(&self, text: &str) {
        self.uart.write_string(text);
        self.uart.write_string("\n");
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        for byte in bytes {
            self.uart.write_byte(*byte);
        }
    }

    pub fn readln(&mut self, buffer: &mut [u8]) -> usize {
        let mut index = 0;

        loop {
            let byte = self.uart.read_byte();

            match byte {

                //Return
                b'\n' | b'\r' => {
                    self.uart.write_byte(b'\n');
                    break;
                }

                //Delete
                b'\x08' => {
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