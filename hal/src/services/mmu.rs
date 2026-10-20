//! MMU services, forwarded to the architecture backend.
//!
//! Every backend (`aarch64_mmu`, `armv7_mmu`, `x64_mmu`) exposes the same
//! functions with the same signatures, so this module is just a re-export
//! plus the shared error type.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmuError {
    /// [`init()`] was either not called or failed
    MmuNotReady,
    /// [`init()`] was already called
    AlreadyInitialized,
    /// No More memory for page tables (use the allocator or increase size)
    OutOfTableSpace,
    /// A remap touched memory that is not mapped.
    NotMapped,
    /// The range is malformed or outside what the architecture can map.
    InvalidRange,
    /// Part of the range is already mapped with a different memory type.
    TypeConflict,
}

#[cfg(target_arch = "aarch64")]
use crate::aarch64::aarch64_mmu as arch;
#[cfg(target_arch = "arm")]
use crate::armv7::armv7_mmu as arch;
#[cfg(target_arch = "x86_64")]
use crate::x86_64::x64_mmu as arch;

/// Brings the MMU (or the CPU-side paging setup) to its working state.
pub use arch::init;
/// Whether the `map_*` functions can be used yet.
pub use arch::is_ready;
/// Maps a range as device memory (identity mapped).
pub use arch::map_device;
/// Maps a range as ordinary cacheable memory (identity mapped).
pub use arch::map_normal;
/// Maps a range as non-cacheable memory (identity mapped).
pub use arch::map_normal_nc;
/// Changes an already mapped range to ordinary cacheable memory.
pub use arch::remap_normal;
/// Changes an already mapped range to non-cacheable memory.
pub use arch::remap_normal_nc;
/// Converts a kernel virtual address to its physical address.
pub use arch::va_to_pa;
/// Physical range of the kernel image, including its boot stack and static
/// tables. Must be reserved from the physical allocator.
pub use arch::kernel_image_range;
