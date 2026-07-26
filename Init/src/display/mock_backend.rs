use super::backend::DisplayBackend;

pub struct MockDisplayBackend {
    width: u32,
    height: u32,
    initialized: bool,
}

impl MockDisplayBackend {
    pub fn new() -> Self {
        Self {
            width: 1024,
            height: 768,
            initialized: false,
        }
    }
}

impl DisplayBackend for MockDisplayBackend {
    fn initialize(&mut self) -> Result<(), String> {
        self.initialized = true;
        println!("[OK] Mock display initialized.");
        Ok(())
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn clear(&mut self) {
        println!("display cleared.");
    }
}