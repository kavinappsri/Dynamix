//! Drivers for QEMU-provided virtual devices.

/// QEMU firmware configuration (`fw_cfg`) device support.
pub mod fw_cfg;
/// QEMU RAM framebuffer configuration and drawing support.
pub mod ramfb;

pub use fw_cfg::{FwCfg, FwCfgError};
pub use ramfb::{Framebuffer, RamFb, RamFbError};
