//! x86_64 support.
//!
//! * `uefi/`    - firmware bindings and everything that runs before `ExitBootServices`
//! * `handoff/` - the one-time transition onto the kernel's own stack and page tables
//! * the rest   - runtime backends for the HAL services (`x64_*`) and CPU support

pub mod gdt;
pub mod handoff;
pub mod port;
pub mod uefi;
pub mod x64_exceptions;
pub mod x64_mmu;
pub mod x64_power;
pub mod x64_timer;
