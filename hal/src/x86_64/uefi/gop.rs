//! Graphics Output Protocol (GOP) discovery.
//!
//! Only valid before `ExitBootServices`. The mode firmware already selected is
//! kept; choosing a better one is future polish.

use core::ffi::c_void;
use core::ptr;

use super::types::{self, EfiBootServices, EfiGraphicsOutputProtocol};
use crate::services::framebuffer::{FirmwareFramebuffer, PixelFormat};

/// Errors that can occur while locating or reading the GOP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GopError {
    /// `LocateProtocol` did not find a GOP handle
    NotFound,
    /// `LocateProtocol` returned an error status other than not-found.
    LocateFailed,
    /// The GOP's `mode` or `mode.info` pointer was null despite a success
    NullMode,
    /// The GOP exists but exposes no linear framebuffer (`BltOnly`, or a
    /// null base address), so the kernel cannot draw to it directly.
    NoLinearFramebuffer,
    /// The framebuffer uses a bitmask layout that we currently do not support
    UnsupportedPixelFormat,
    /// The reported geometry does not fit the reported framebuffer size.
    InconsistentGeometry,
}

impl GopError {
    pub const fn describe(self) -> &'static str {
        match self {
            GopError::NotFound => "GOP not found",
            GopError::LocateFailed => "GOP LocateProtocol failed",
            GopError::NullMode => "GOP reported a null mode",
            GopError::NoLinearFramebuffer => "GOP has no linear framebuffer",
            GopError::UnsupportedPixelFormat => "GOP pixel format unsupported",
            GopError::InconsistentGeometry => "GOP geometry does not fit its framebuffer",
        }
    }
}

/// Locates the Graphics Output Protocol via `LocateProtocol` and reads back
/// its current mode's framebuffer.
///
/// # Safety
/// `boot_services` must be valid 
pub unsafe fn locate_gop(boot_services: *mut EfiBootServices) -> Result<FirmwareFramebuffer, GopError> {
    let mut interface: *mut c_void = ptr::null_mut();

    // SAFETY: upheld by the caller.
    let status = unsafe {
        ((*boot_services).locate_protocol)(&types::EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID, ptr::null_mut(), &mut interface)
    };

    if types::efi_is_error(status) {
        return Err(if status == types::EFI_NOT_FOUND { GopError::NotFound } else { GopError::LocateFailed });
    }
    if interface.is_null() {
        return Err(GopError::NotFound);
    }

    let gop = interface as *mut EfiGraphicsOutputProtocol;

    // SAFETY: a successful `LocateProtocol` returns a valid protocol interface.
    let mode = unsafe { (*gop).mode };
    if mode.is_null() {
        return Err(GopError::NullMode);
    }
    // SAFETY: checked non-null just above.
    let mode = unsafe { &*mode };
    if mode.info.is_null() {
        return Err(GopError::NullMode);
    }
    // SAFETY: checked non-null just above.
    let info = unsafe { &*mode.info };

    let format = match info.pixel_format {
        types::PIXEL_RGB_RESERVED_8BIT_PER_COLOR => PixelFormat::Rgbx8,
        types::PIXEL_BGR_RESERVED_8BIT_PER_COLOR => PixelFormat::Bgrx8,
        types::PIXEL_BIT_MASK => {
            let masks = info.pixel_information;
            match (masks.red_mask, masks.green_mask, masks.blue_mask) {
                (0x0000_00FF, 0x0000_FF00, 0x00FF_0000) => PixelFormat::Rgbx8,
                (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) => PixelFormat::Bgrx8,
                _ => return Err(GopError::UnsupportedPixelFormat),
            }
        }
        _ => return Err(GopError::NoLinearFramebuffer),
    };

    let base = usize::try_from(mode.frame_buffer_base).map_err(|_| GopError::NoLinearFramebuffer)?;
    if base == 0 || mode.frame_buffer_size == 0 {
        return Err(GopError::NoLinearFramebuffer);
    }

    FirmwareFramebuffer::new(
        base,
        mode.frame_buffer_size,
        info.horizontal_resolution,
        info.vertical_resolution,
        info.pixels_per_scan_line,
        format,
    )
    .ok_or(GopError::InconsistentGeometry)
}
