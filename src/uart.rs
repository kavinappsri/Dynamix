pub struct Uart {
    base: usize,
}

impl Uart {
    pub const fn new(base: usize) -> Self {
        Self {
            base
        }
    }

    pub fn write_byte(&self, byte: u8) {
        const FR: usize = 0x18;
        const DR: usize = 0x00;

        unsafe {
            while ((self.base + FR) as *const u32).read_volatile() & (1 << 5) != 0 {}

            ((self.base + DR) as *mut u32).write_volatile(byte as u32);
        }
    }

    pub fn write_string(&self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }
}