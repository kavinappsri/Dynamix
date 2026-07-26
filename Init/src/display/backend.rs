pub trait DisplayBackend {
    fn initialize(&mut self) -> Result<(), String>;
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn clear(&mut self);

}