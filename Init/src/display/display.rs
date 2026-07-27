use crate::display::backend::DisplayBackend;

pub struct Display {
    backend: Box<dyn DisplayBackend>,
    framebuffer: Option<Framebuffer>,
}

impl Display {
    pub fn new(backend: Box<dyn DisplayBackend>) -> Self {
        Self {
            backend,
            framebuffer: None,
        }
    }

    pub fn initialize(&mut self) -> Result<(), String> {
        self.backend.initialize()?;
        let virtual_framebuffer = Framebuffer::new(self.backend.width(), self.backend.height());
        self.framebuffer = Some(virtual_framebuffer);
        Ok(())
    }

    pub fn clear(&mut self, color: u32) {
        self.backend.clear(color);
    }

    pub fn width(&self) -> u32 {
        self.backend.width()
    }

    pub fn height(&self) -> u32 {
        self.backend.height()
    }
    pub fn mainloop(&mut self) {
        self.backend.mainloop()
    }

    pub fn present(&mut self) {
        self.backend.present(self.framebuffer.as_ref().unwrap())
    }
}

pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Framebuffer {

    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width*height) as usize]
        }
    }

    pub fn clear(&mut self, color: u32) {
        for pixel in self.pixels.iter_mut() {
            *pixel = color;
        }
    }

}