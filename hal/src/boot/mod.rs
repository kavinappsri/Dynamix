//! The HAL half of the boot path: what happens between the first instruction
//! and `rust_main`.
//!
//! Each platform builds a [`BootInfo`] its own way (`fdt` here for
//! aarch64/armv7, `x86_64::uefi::entry` for x86_64) and ends in
//! [`enter_kernel`], so the kernel sees the same entry point everywhere.

#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
mod fdt;

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::services::boot_info::BootInfo;

unsafe extern "Rust" {
    /// Defined by the kernel crate.
    fn rust_main(boot: &'static mut BootInfo<'static>) -> !;
}

/// Where `BootInfo` lives from the moment it is complete. It sits inside the
/// image, so on x86_64 it is reachable at its load address before the jump to
/// the higher half and at its higher-half alias after it (the firmware stack
/// it was built on is neither mapped afterwards nor safe to keep).
struct BootInfoSlot(UnsafeCell<MaybeUninit<BootInfo<'static>>>);

// SAFETY: written once by the boot core before the kernel starts; afterwards
// only the single `&'static mut` from `stored` exists.
unsafe impl Sync for BootInfoSlot {}

static BOOT_INFO: BootInfoSlot = BootInfoSlot(UnsafeCell::new(MaybeUninit::uninit()));
static STORED: AtomicBool = AtomicBool::new(false);

/// Moves `boot` into the slot. May only be called once.
pub(crate) fn store(boot: BootInfo<'static>) {
    assert!(!STORED.swap(true, Ordering::AcqRel), "boot info stored twice");
    // SAFETY: the flag above makes this the only writer, before any reader.
    unsafe { (*BOOT_INFO.0.get()).write(boot) };
}

/// The stored boot info.
///
/// # Safety
/// [`store`] must have run, and the returned reference must be the only one
/// alive (it is handed to `rust_main` and nobody else).
pub(crate) unsafe fn stored() -> &'static mut BootInfo<'static> {
    // SAFETY: upheld by the caller.
    unsafe { (*BOOT_INFO.0.get()).assume_init_mut() }
}

/// Calls the kernel's `rust_main` with the stored boot info.
///
/// # Safety
/// [`store`] must have run and the boot info must be complete.
pub(crate) unsafe fn enter_kernel() -> ! {
    // SAFETY: upheld by the caller; `rust_main` never returns.
    unsafe { rust_main(stored()) }
}
