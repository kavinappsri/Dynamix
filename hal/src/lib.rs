#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_arch = "aarch64")]
pub mod aarch64;
pub mod device_tree;
pub mod mmio;
pub mod serial;
mod sync;

pub use device_tree::{probe_serial, DeviceTree, ProbeError};
pub use serial::Serial;
