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

    pub fn clear(&mut self) {
        self.backend.clear();
    }

    pub fn width(&self) -> u32 {
        self.backend.width()
    }

    pub fn height(&self) -> u32 {
        self.backend.height()
    }
}