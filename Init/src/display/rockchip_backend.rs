use super::backend::DisplayBackend;


// TODO - IMPLEMENT ROCKCHIP BACKEND
pub struct RockchipDisplayBackend;

impl RockchipDisplayBackend {
    pub fn new() -> Self {
        Self
    }
}

impl DisplayBackend for RockchipDisplayBackend {
    fn initialize(&mut self) -> Result<(), String> {
        Err("Rockchip backend not implemented.".to_string())
    }
    fn width(&self) -> u32 {
        0
    }

    fn height(&self) -> u32 {
        0
    }

    fn clear(&mut self, color: u32) {

    }
    fn mainloop(&mut self) {
        
    }
    fn present(&mut self) {
        
    }
    

}