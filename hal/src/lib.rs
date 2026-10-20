#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

//! Hardware abstractions used by the DynamixOS early-boot kernel.
//!
//! The crate is `no_std` and deliberately small. It provides:
//!
//! * the boot path for each architecture (`boot`, `x86_64::{uefi, handoff}`),
//!   ending in a [`services::boot_info::BootInfo`] passed to the kernel's `rust_main`;
//! * architecture-neutral services (`services`): MMU, physical memory manager,
//!   exceptions, timer, power, and hardware discovery over either a device
//!   tree or ACPI behind one [`services::device_query::DeviceQuery`] trait;
//! * driver traits and linker-section based driver registration
//!   (`driver_traits`), plus the drivers themselves (`drivers`).
//!
//! Hardware-specific constructors are unsafe because the caller owns the
//! mapping and exclusivity guarantees.
//!
//! # Examples
//!
//! ```ignore
//! use hal::{probe_serial, services::device_query::DeviceQuery};
//!
//! let uart = probe_serial(hardware)?;
//! uart.write_str("early boot complete\n");
//! ```

/// Aarch64 dependent features
#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "arm")]
pub mod armv7;

pub mod drivers;
pub mod services;
pub mod driver_traits;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;
pub mod boot;

#[cfg(target_arch = "x86_64")]
pub use acpi;
pub use driver_traits::framebuffer::{Framebuffer, FramebufferError};
pub use driver_traits::serial::{Serial, active_serial, probe_serial};

// Boot assembly compilation
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
use core::arch::global_asm;

#[cfg(target_arch = "aarch64")]
global_asm!(include_str!("asm/aarch64/aarch64_boot.s"));

#[cfg(target_arch = "arm")]
global_asm!(include_str!("asm/armv7/armv7_boot.s"));

#[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
compile_error!("This architecture is currently not supported");

