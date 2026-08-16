//! Drivers for QEMU-provided virtual devices.

/// QEMU firmware configuration (`fw_cfg`) device support.
#[cfg(feature = "fw-cfg")]
pub(crate) mod fw_cfg;
/// QEMU RAM framebuffer driver.
#[cfg(feature = "ramfb")]
mod ramfb;
