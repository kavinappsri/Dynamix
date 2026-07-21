use crate::logger;
use crate::service::Service;

pub struct LoggerService;

impl Service for LoggerService {
    fn name(&self) -> &str {
        "Logger"
    }

    fn start(&self) {
        logger::info("Logger service started.");
    }

    fn stop(&self) {
        logger::info("Logger service stopped.");
    }
}