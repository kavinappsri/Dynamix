use crate::logger;
use crate::service::{Service, ServiceState};

pub struct LoggerService {
    _state: ServiceState
}

impl LoggerService {
    pub fn new() -> Self{
        Self {
            _state: ServiceState::Created,
        }
    }
}

impl Service for LoggerService {
    fn name(&self) -> &str {
        "Logger"
    }

    fn start(&mut self) -> Result<(), String> {
        logger::info("Logger service started.");
        Ok(())
    }

    fn stop(&self) {
        logger::info("Logger service stopped.");
    }
}