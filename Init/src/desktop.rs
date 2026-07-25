use crate::display::Display;

pub struct Desktop {
    running: bool,
}

impl Desktop {
    pub fn new() -> Self {
        Self {
            running: true,
        }
    }
    pub fn launch(&self) {
        let display = Display::new();

        display.initialize();

        println!("Desktop Launched!!")
    }
    pub fn run(&mut self) {
        println!("Desktop loop running...");
    }

}