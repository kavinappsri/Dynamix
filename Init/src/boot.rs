use crate::logger;
use crate::logger_service::LoggerService;
use crate::service_manager::ServiceManager;

pub fn start() {
    logger::info("Starting Dynamix...");

    initialize_hardware();

    load_configuration();

    initialize_services();

    let mut manager = ServiceManager::new();

    manager.register(Box::new(LoggerService));

    manager.start_all();
}

fn initialize_hardware() {
    logger::info("Initializing hardware...");
}

fn load_configuration() {
    logger::info("Loading configuration...");
}

fn initialize_services() {
    logger::info("Initializing services...");
}