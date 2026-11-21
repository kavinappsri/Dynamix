#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

//! Hardware abstractions used by the DynamixOS early-boot kernel.
//!
//! The crate is `no_std` and deliberately small. It provides:
//!
//! * the boot path for each architecture (`boot`, `x86_64::{uefi, handoff}`),
//!   ending in a [`services::boot_info::BootInfo`] passed to the kernel's `rust_main`;
//! * architecture-neutral services (`services`): MMU, physical memory manager,
//!   exceptions, timer, power, and the device model
//!   ([`services::device`]), which turns a device tree or the ACPI namespace
//!   into devices and probes drivers for them;
//! * driver traits, the driver table and the registries of bound drivers
//!   (`driver_traits`), plus the drivers themselves (`drivers`).
//!
//! # Examples
//!
//! ```ignore
//! hal::services::device::init(hardware)?;
//!
//! let uart = hal::active_serial().expect("no serial port");
//! uart.write_byte(b'!');
//! ```

extern crate alloc;

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

pub use driver_traits::framebuffer::{Framebuffer, FramebufferError};
pub use driver_traits::serial::{Serial, active_serial};

// Boot assembly compilation
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
use core::arch::global_asm;

#[cfg(target_arch = "aarch64")]
global_asm!(include_str!("asm/aarch64/aarch64_boot.s"));

#[cfg(target_arch = "arm")]
global_asm!(include_str!("asm/armv7/armv7_boot.s"));

#[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
compile_error!("This architecture is currently not supported");

