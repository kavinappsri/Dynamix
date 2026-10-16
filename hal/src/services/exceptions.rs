//! Exceptions bridge

#[cfg(target_arch = "aarch64")]
use crate::aarch64;

#[cfg(target_arch = "arm")]
use crate::armv7;

use crate::services::boot_info::RawFramebufferInfo;
use crate::services::sync::StaticCell;

static FAULT_FRAMEBUFFER: StaticCell<RawFramebufferInfo> = StaticCell::uninit();

/// Writes the exception vector table / descriptor tables
pub fn init() {
    #[cfg(target_arch = "aarch64")]
    aarch64::aarch64_exceptions::init();

    #[cfg(target_arch = "arm")]
    armv7::armv7_exceptions::init();

    #[cfg(target_arch = "x86_64")]
    crate::x86_64::x64_exceptions::init();
}

/// Registers a (mapped) framebuffer the fault reporter may paint when an
/// exception is fatal. Only the first call counts.
pub fn set_fault_framebuffer(framebuffer: RawFramebufferInfo) {
    FAULT_FRAMEBUFFER.get_or_init(|| framebuffer);
}

#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(crate) fn fault_framebuffer() -> Option<&'static RawFramebufferInfo> {
    FAULT_FRAMEBUFFER.get()
}
