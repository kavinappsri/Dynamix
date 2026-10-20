//! The kernel's boot stack, and helpers to run code on it.

use core::arch::asm;
use core::cell::UnsafeCell;

use crate::services::boot_info::BootInfo;

/// Size of the boot stack.
pub const BOOT_STACK_SIZE: usize = 64 * 1024;

#[repr(C, align(4096))]
struct BootStack(UnsafeCell<[u8; BOOT_STACK_SIZE]>);

// SAFETY: the stack is only ever used by the one core that runs `rust_main`.
unsafe impl Sync for BootStack {}

static BOOT_STACK: BootStack = BootStack(UnsafeCell::new([0; BOOT_STACK_SIZE]));


pub fn stack_top() -> usize {
    BOOT_STACK.0.get() as usize + BOOT_STACK_SIZE
}


pub type StackEntry = unsafe extern "efiapi" fn(*mut BootInfo<'static>) -> !;

/// Points `rsp` at `top` and calls `entry(arg)` there.
pub unsafe fn call_on_stack(top: usize, entry: usize, arg: usize) -> ! {
    unsafe {
        asm!(
        "mov rsp, {top}",
        "xor ebp, ebp",
        "sub rsp, 32",
        "call {entry}",
        "ud2",
        top = in(reg) top,
        entry = in(reg) entry,
        in("rcx") arg,
        options(noreturn),
        )
    }
}

/// Switches to the boot stack and calls `entry(arg)` on it.
pub unsafe fn switch_and_call(arg: *mut BootInfo<'static>, entry: StackEntry) -> ! {
    unsafe { call_on_stack(stack_top(), entry as usize, arg as usize) }
}
