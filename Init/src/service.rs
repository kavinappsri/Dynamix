pub trait Service {
    fn name(&self) -> &str;
    fn start(&self);
    fn stop(&self);
}