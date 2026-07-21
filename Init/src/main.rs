pub mod logger;
pub mod config;
pub mod boot;
pub mod service;
pub mod logger_service;
pub mod service_manager;

fn main() {
    boot::start();
}
