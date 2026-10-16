//! Graphics Output Protocol (GOP) Discovery
//!
//! This module only currently contains functions that are valid before
//! `ExitBootServices()` are called and UEFI is terminated.

use core::ffi::c_void;
use core::ptr;

use crate::x86_64::uefi_types;
use crate::services::boot_info::{PixelFormat, RawFramebufferInfo};

/// Framebuffer geometry and pixel layout. Read out of the
/// GOP's current mode
#[derive(Copy, Clone, Debug)]
pub struct GopFramebufferInfo {
    /// Physical address of pixel (0, 0). Identity-mapped by firmware
    pub base: u64,
    /// Size in bytes of the region `base` points at.
    pub size: usize,
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Pixels per scanline - may exceed `width` if the hardware pads rows.
    pub pixels_per_scan_line: u32,
    pub pixel_format: uefi_types::EfiGraphicsPixelFormat,
}

/// Errors that can occur while locating or reading the GOP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GopError {
    /// `LocateProtocol` did not find a GOP handle
    NotFound,
    /// `LocateProtocol` returned an error status other than not-found.
    LocateFailed,
    /// The GOP's `mode` or `mode.info` pointer was null despite a success
    NullMode,
    /// `SetMode` returned an error status.
    SetModeFailed,
    /// The GOP exists but exposes no linear framebuffer (`BltOnly`, or a
    /// null base address), so the kernel cannot draw to it directly.
    NoLinearFramebuffer,
    /// The framebuffer uses a bitmask layout that we currently do not support
    UnsupportedPixelFormat,
}

/// Locates the Graphics Output Protocol via `LocateProtocol` and reads back
/// its current mode's framebuffer info.
pub unsafe fn locate_gop(boot_services: *mut uefi_types::EfiBootServices) -> Result<RawFramebufferInfo, GopError> {
    let mut interface: *mut c_void = ptr::null_mut();

    let status = unsafe {
        ((*boot_services).locate_protocol)(
            &uefi_types::EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,
            ptr::null_mut(),
            &mut interface
        )
    };

    if uefi_types::efi_is_error(status) {
        return Err(if status == uefi_types::EFI_NOT_FOUND {
            GopError::NotFound
        } else {
            GopError::LocateFailed
        });
    }

    if interface.is_null() {
        return Err(GopError::NotFound);
    }

    let gop = interface as *mut uefi_types::EfiGraphicsOutputProtocol;

    let mode = unsafe { (*gop).mode };
    if mode.is_null() {
        return Err(GopError::NullMode);
    }

    let mode = unsafe { &*mode };
    if mode.info.is_null() {
        return Err(GopError::NullMode);
    }

    let info = unsafe { &*mode.info };

    let pixel_format = match info.pixel_format {
        uefi_types::PIXEL_RGB_RESERVED_8BIT_PER_COLOR => PixelFormat::Rgbx8,
        uefi_types::PIXEL_BGR_RESERVED_8BIT_PER_COLOR => PixelFormat::Bgrx8,
        uefi_types::PIXEL_BIT_MASK => {
            let masks = info.pixel_information;
            match (masks.red_mask, masks.green_mask, masks.blue_mask) {
                (0x0000_00FF, 0x0000_FF00, 0x00FF_0000) => PixelFormat::Rgbx8,
                (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) => PixelFormat::Bgrx8,
                _ => return Err(GopError::UnsupportedPixelFormat),
            }
        }
        _ => return Err(GopError::NoLinearFramebuffer),
    };

    if mode.frame_buffer_base == 0 || mode.frame_buffer_size == 0 {
        return Err(GopError::NoLinearFramebuffer);
    }

    Ok(RawFramebufferInfo {
        base: mode.frame_buffer_base,
        size: mode.frame_buffer_size,
        width: info.horizontal_resolution,
        height: info.vertical_resolution,
        pixels_per_scan_line: info.pixels_per_scan_line,
        pixel_format,
    })
}

