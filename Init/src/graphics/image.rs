pub enum PixelFormat {
    RGB888,
    RGB565,
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}