use crate::service::Service;

pub struct HardwareService;

impl HardwareService {
    pub fn new() -> Self {
        Self
    }

    pub fn detect() {
        println!("Detecting hardware... [Make sure to use logger in future]");

        println!("Architecture : {}", std::env::consts::ARCH);
        println!("Operating System : {}", std::env::consts::OS);

        match std::env::current_dir() {
            Ok(path) => println!("Current Directory: {}", path.display()),
            Err(_) => println!("Error in getting current directory")
        }
    }
}

impl Service for HardwareService {
    fn name(&self) -> &str {
        "Hardware Service"
    }

    fn start(&mut self) -> Result<(), String> {
        Self::detect();
        Ok(())
    }

    fn stop(&self) {
        todo!("Hardware Service Stop")
    }
}