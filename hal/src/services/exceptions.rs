//! Exceptions bridge

#[cfg(target_arch = "aarch64")]
use crate::aarch64;

#[cfg(target_arch = "arm")]
use crate::armv7;



/// Writes the exception vector table 
pub fn init() {
    #[cfg(target_arch = "aarch64")]
    aarch64::aarch64_exceptions::init();

    #[cfg(target_arch = "arm")]
    armv7::armv7_exceptions::init()
}