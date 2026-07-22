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

    pub fn start_all(&mut self) {
        for service in &mut self.services {

            match service.start() {
                Ok(_) => println!("[OK] {}", service.name()),
                Err(e) => println!("[FAIL] {} | Err: {}", service.name(), e),
            }
        }
    }
}