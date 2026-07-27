use crate::display::display::Framebuffer;

pub trait DisplayBackend {
    fn initialize(&mut self) -> Result<(), String>;
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn clear(&mut self, color: u32);
    fn mainloop(&mut self);
    fn present(&mut self, framebuffer: &Framebuffer);

}