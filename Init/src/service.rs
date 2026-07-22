pub trait Service {
    fn name(&self) -> &str;
    fn start(&mut self) -> Result<(), String>;
    fn stop(&self);
}

pub enum ServiceState {
    Created,
    Started,
    Running,
    Stopped,
    Failed,
}