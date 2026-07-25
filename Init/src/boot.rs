use crate::hardware_service::HardwareService;
use std::time::Instant;
use crate::logger_service::LoggerService;
use crate::service_manager::ServiceManager;
use crate::config::Config;
use crate::filesystem_service::FilesystemService;
use crate::boot_context::BootContext;
use crate::panic_handler::PanicHandler;
use crate::desktop::Desktop;

pub fn start() {
    println!("========================================");
    println!("            Dynamix OS");
    println!("========================================");
    println!();
    println!("Starting boot sequence...");
    println!();

    let init_start = Instant::now();

    let context = match Config::new("../config/dynamix.conf") {
        Ok(config) => BootContext::new(config),
        Err(_) => {
            PanicHandler::panic(
                "Boot",
                "dynamix.conf could not be accessed",
            );
            return;
        }
    };





    let mut manager = ServiceManager::new();

    manager.register(Box::new(LoggerService::new()));
    manager.register(Box::new(HardwareService::new()));
    manager.register(Box::new(FilesystemService::new()));

    manager.start_all();

    let mut desktop = Desktop::new();

    desktop.launch();

    desktop.run();

    println!("Init completed in {:.2?}", init_start.elapsed())
}
