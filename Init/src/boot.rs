use crate::hardware_service::HardwareService;
use std::time::Instant;
use crate::logger_service::LoggerService;
use crate::service_manager::ServiceManager;
use crate::config;

pub fn start() {
    println!("========================================");
    println!("            Dynamix OS");
    println!("========================================");
    println!();
    println!("Starting boot sequence...");
    println!();

    let init_start = Instant::now();

    match config::load() {

        Ok(contents) => {
            println!("[OK] Configuration loaded.");
            println!("{}", contents);
        }

        Err(error) => {
            println!("[FAIL] Configuration: {}", error);
        }

    }

    let config = config::Config::new();
    match config {
        Ok(configobj) => {
            println!("{}", configobj.get("hostname").unwrap().to_string())
        },
        Err(_) => todo!()
    }


    let mut manager = ServiceManager::new();

    manager.register(Box::new(LoggerService::new()));
    manager.register(Box::new(HardwareService::new()));

    manager.start_all();

    println!("Init completed in {:.2?}", init_start.elapsed())
}
