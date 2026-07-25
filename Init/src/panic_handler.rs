pub struct PanicHandler;

impl PanicHandler {
    pub fn panic(service: &str, reason: &str) {
        println!(">>>>>>>> OPERATING SYSTEM PANIC <<<<<<<<");
        println!();
        println!("Dynamix ran into a fatal error during runtime :(");
        println!();
        println!("Service: {}", service);
        println!("Reason: {}", reason);
        println!("Dynamix has stopped to protect the integrity of the system");
        println!();
        println!("┐(´～｀)┌");
    }

}