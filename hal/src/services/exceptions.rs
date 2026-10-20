//! Exceptions bridge

#[cfg(target_arch = "aarch64")]
use crate::aarch64;

#[cfg(target_arch = "arm")]
use crate::armv7;

use crate::services::framebuffer::FirmwareFramebuffer;
use crate::services::sync::StaticCell;

static FAULT_FRAMEBUFFER: StaticCell<FirmwareFramebuffer> = StaticCell::uninit();

/// Writes the exception vector table / descriptor tables
pub fn init() {
    #[cfg(target_arch = "aarch64")]
    aarch64::aarch64_exceptions::init();

    #[cfg(target_arch = "arm")]
    armv7::armv7_exceptions::init();

    #[cfg(target_arch = "x86_64")]
    crate::x86_64::x64_exceptions::init();
}

/// Registers a (mapped) framebuffer to paint when something fatal happens: an
/// exception, or a kernel panic. Only the first call counts.
pub fn set_fault_framebuffer(framebuffer: FirmwareFramebuffer) {
    FAULT_FRAMEBUFFER.get_or_init(|| framebuffer);
}

/// The framebuffer registered with [`set_fault_framebuffer`], if any.
pub fn fault_framebuffer() -> Option<&'static FirmwareFramebuffer> {
    FAULT_FRAMEBUFFER.get()
}
