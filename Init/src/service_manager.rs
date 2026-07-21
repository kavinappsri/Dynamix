use crate::service::Service;

pub struct ServiceManager
{
    services: Vec<Box<dyn Service>>,
}

impl ServiceManager
{
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
        }
    }

    pub fn register(&mut self, service: Box<dyn Service>, ) {
        self.services.push(service);
    }

    pub fn start_all(&self) {
        for service in &self.services {
            println!("Starting {}...", service.name());

            service.start();
        }
    }
}