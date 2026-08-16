#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

//! Hardware abstractions used by the DynamixOS early-boot kernel.
//!
//! The crate is `no_std` and deliberately small. It provides volatile MMIO
//! access, polling serial I/O, device-tree based driver discovery, and optional
//! platform-specific drivers. QEMU `virt` support is enabled by the current
//! default feature set, but the HAL is organized to host drivers for additional
//! platforms. Hardware-specific constructors are unsafe because the caller
//! owns the mapping and exclusivity guarantees.
//!
//! # Examples
//!
//! ```ignore
//! use hal::{probe_serial, DeviceTree};
//!
//! let uart = probe_serial(&tree)?;
//! uart.write_str("early boot complete\n");
//! # Ok::<(), hal::ProbeError>(())
//! ```

#[cfg(target_arch = "aarch64")]
/// AArch64-specific device drivers.
pub mod aarch64;
/// Driver discovery based on bootloader device-tree data.
pub mod device_tree;
/// Generic framebuffer interface and driver discovery.
pub mod framebuffer;
/// Volatile access to identity-mapped MMIO register blocks.
pub mod mmio;
#[cfg(any(feature = "fw-cfg", feature = "ramfb"))]
/// Drivers for virtual devices exposed by QEMU `virt`.
pub mod qemu;
/// Common polling serial-device interface.
pub mod serial;
mod sync;

pub use device_tree::{DeviceTree, ProbeError, active_serial, probe_serial};
pub use framebuffer::{Framebuffer, FramebufferError, probe_framebuffer};
pub use serial::Serial;
