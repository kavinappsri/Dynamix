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
//! let uart = probe_serial(&tree)?;111.4285714286
//! uart.write_str("early boot complete\n");
//! # Ok::<(), hal::ProbeError>(())
//! ```


/// Aarch64 dependent features
#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "arm")]
pub mod armv7;

pub mod drivers;
pub mod services;
pub mod driver_traits;


pub use driver_traits::framebuffer::{Framebuffer, FramebufferError};
pub use driver_traits::serial::{Serial, active_serial, probe_serial};

use core::arch::global_asm;


// Boot assembly compilation

#[cfg(target_arch = "aarch64")]
global_asm!(include_str!("asm/aarch64/aarch64_boot.s"));

#[cfg(target_arch = "arm")]
global_asm!(include_str!("asm/armv7/armv7_boot.s"));

#[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
compile_error!("This architecture is currently not supported");

