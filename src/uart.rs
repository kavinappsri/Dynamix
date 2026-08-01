pub struct Uart {
    base: usize,
}

impl Uart {

    const FR: usize = 0x18;
    const DR: usize = 0x00;

    pub const fn new(base: usize) -> Self {
        Self {
            base
        }
    }

    pub fn write_byte(&self, byte: u8) {


        unsafe {
            while ((self.base + Uart::FR) as *const u32).read_volatile() & (1 << 5) != 0 {}

            ((self.base + Uart::DR) as *mut u32).write_volatile(byte as u32);
        }
    }

    pub fn read_byte(&self) -> u8 {


        unsafe {
            while ((self.base + Uart::FR) as *const u32).read_volatile() & (1 << 4) != 0 {}

            ((self.base + Uart::DR) as *const u32).read_volatile() as u8
        }
    }

    pub fn write_string(&self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }
}