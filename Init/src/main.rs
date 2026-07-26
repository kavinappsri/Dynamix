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
pub mod graphics;

fn main() {
    boot::start();
    /*
    let event_loop = EventLoop::new().unwrap();
    let mut app = TestWindow::new();
    event_loop.run_app(&mut app).unwrap();
     */
}
