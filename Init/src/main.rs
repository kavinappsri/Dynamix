pub mod logger;
pub mod config;
pub mod boot;
pub mod service;
pub mod logger_service;
pub mod service_manager;
pub mod hardware_service;
pub mod filesystem_service;
pub mod device_manager;
pub mod panic_handler;
pub mod boot_context;
pub mod display;

fn main() {
    boot::start();
}
