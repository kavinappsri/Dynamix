//! x86 port I/O.
//!
//! Thin wrappers over `in` / `out`.

use core::arch::asm;


#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: upheld by the caller.
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)) };
}


#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: upheld by the caller.
    unsafe { asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags)) };
    value
}


#[inline]
pub unsafe fn outw(port: u16, value: u16) {
    // SAFETY: upheld by the caller.
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags)) };
}


#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    // SAFETY: upheld by the caller.
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags)) };
    value
}
