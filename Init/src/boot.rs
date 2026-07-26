use crate::display::mock_backend::MockDisplayBackend;
use crate::display::display::Display;
use crate::hardware_service::HardwareService;
use std::time::Instant;
use crate::logger_service::LoggerService;
use crate::service_manager::ServiceManager;
use crate::config::Config;
use crate::filesystem_service::FilesystemService;
use crate::boot_context::BootContext;
use crate::panic_handler::PanicHandler;

pub fn start() {
    println!("========================================");
    println!("            Dynamix OS");
    println!("========================================");
    println!();

    let init_start = Instant::now();

    let _context = match Config::new("../config/dynamix.conf") {
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

    let mut display = Display::new(Box::new(MockDisplayBackend::new()));
    let _ = display.initialize();
    display.clear();
    println!("Display: {}x{}", display.width(), display.height());

    println!("Init completed in {:.2?}", init_start.elapsed())
}
