use crate::display::backend::DisplayBackend;

pub struct Display {
    backend: Box<dyn DisplayBackend>,
}

impl Display {
    pub fn new(backend: Box<dyn DisplayBackend>) -> Self {
        Self {
            backend,
        }
    }

    pub fn initialize(&mut self) -> Result<(), String> {
        self.backend.initialize()
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
}