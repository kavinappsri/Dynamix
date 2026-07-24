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
    
    let config = config::Config::new("../config/dynamix.conf");
    match config {
        Ok(configobj) => {
            println!("hostname: {}", configobj.get("hostname").unwrap().to_string());
            println!("verbose boot: {}", configobj.get_bool("boot.verbose").unwrap());
            println!("theme: {}", configobj.get("theme").unwrap().to_string());
            println!("ui scale: {}", configobj.get("ui.scale").unwrap().to_string());
        },
        Err(_) => todo!()
    }


    let mut manager = ServiceManager::new();

    manager.register(Box::new(LoggerService::new()));
    manager.register(Box::new(HardwareService::new()));

    manager.start_all();

    println!("Init completed in {:.2?}", init_start.elapsed())
}
