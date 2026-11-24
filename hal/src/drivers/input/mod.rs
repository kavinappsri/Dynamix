#[cfg(all(feature = "driver-i8042", target_arch = "x86_64"))]
/// The PC's PS/2 keyboard controller (i8042), keyboard port
pub mod i8042;
