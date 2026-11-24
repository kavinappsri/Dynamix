//! x86 port I/O.
//!
//! Thin wrappers over `in` / `out`.

use core::arch::asm;

/// Writing a `u8` out of a port
///
/// # Safety
/// `port` must be a valid port
#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: upheld by the caller.
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)) };
}

/// Reading a `u8` out of a port
///
/// # Safety
/// `port` must be a valid port
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: upheld by the caller.
    unsafe { asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Writing a `u16` out of a port
///
/// # Safety
/// `port` must be a valid port
#[inline]
pub unsafe fn outw(port: u16, value: u16) {
    // SAFETY: upheld by the caller.
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags)) };
}

/// Reading a `u16` out of a port
///
/// # Safety
/// `port` must be a valid port
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    // SAFETY: upheld by the caller.
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Writing a `u32` out of a port
///
/// # Safety
/// `port` must be a valid port
#[inline]
pub unsafe fn outl(port: u16, value: u32) {
    // SAFETY: upheld by the caller.
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags)) };
}

/// Reading a `u32` out of a port
///
/// # Safety
/// `port` must be a valid port
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    // SAFETY: upheld by the caller.
    unsafe { asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags)) };
    value
}
