//! Mmu Services Abstraction


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmuError {
    /// [`mmu::init()`] was either not called or failed
    MmuNotReady,
    /// [`mmu::init()`] was already called
    AlreadyInitialized,
    /// No More memory for page tables (use the allocator or increase size)
    OutOfTableSpace,
    /// A remap touched memory that is not mapped.
    NotMapped,
    /// The range is malformed or outside what the architecture can map.
    InvalidRange,
}

#[cfg(target_arch = "x86_64")]
use crate::x86_64::x64_mmu::{self as x86_mmu, MemKind, PagingError};

#[cfg(target_arch = "x86_64")]
fn from_paging(error: PagingError) -> MmuError {
    match error {
        PagingError::OutOfTableSpace => MmuError::OutOfTableSpace,
        PagingError::NotInstalled => MmuError::MmuNotReady,
        PagingError::AlreadyInstalled => MmuError::AlreadyInitialized,
        PagingError::NotMapped => MmuError::NotMapped,
        PagingError::AddressTooHigh | PagingError::Misaligned | PagingError::BadImage => MmuError::InvalidRange,
    }
}

pub fn init() -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::init()
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::init()
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_mmu::init().map_err(from_paging)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn map_device(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::map_device(phys as u64, len as u64)
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::map_device(phys as u32, len as u32)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_mmu::map_identity(phys as u64, len as u64, MemKind::Device).map_err(from_paging)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn map_normal(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::map_normal(phys as u64, len as u64)
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::map_normal(phys as u32, len as u32)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_mmu::map_identity(phys as u64, len as u64, MemKind::Normal).map_err(from_paging)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn map_normal_nc(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::map_normal_nc(phys as u64, len as u64)
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::map_normal_nc(phys as u32, len as u32)
    }
    #[cfg(target_arch = "x86_64")]
    {
        // Write-combining once `init` has programmed the PAT, uncached before that.
        x86_mmu::map_identity(phys as u64, len as u64, MemKind::NormalNc).map_err(from_paging)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn va_to_pa(addr: usize) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::va_to_pa(addr as u64) as usize
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::va_to_pa(addr as u32) as usize
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_mmu::va_to_pa(addr as u64) as usize
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

/// Changes an already mapped range to normal non cacheable attributes
pub fn remap_normal_nc(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::remap_normal_nc(phys as u64, len as u64)
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::remap_normal_nc(phys as u32, len as u32)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_mmu::remap_identity(phys as u64, len as u64, MemKind::Normal).map_err(from_paging)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

/// Changes an already mapped range to normal attribute
pub fn remap_normal(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::remap_normal(phys as u64, len as u64)
    }
    #[cfg(target_arch = "arm")]
    {
        crate::armv7::armv7_mmu::remap_normal(phys as u32, len as u32)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_mmu::remap_identity(phys as u64, len as u64, MemKind::NormalNc).map_err(from_paging)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}

/// Physical `[start, end)` of the kernel image, including its boot stack and
/// static tables. Must be reserved from the physical allocator; on x86_64 UEFI
/// already keeps it out of the usable memory map, so this is belt and braces.
pub fn kernel_image_range() -> (usize, usize) {
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    {
        unsafe extern "C" {
            static _kernel_vma_start: u8;
            static _kernel_vma_end: u8;
        }
        // SAFETY: linker-provided symbols; only their addresses are used.
        unsafe { (va_to_pa((&raw const _kernel_vma_start) as usize), va_to_pa((&raw const _kernel_vma_end) as usize)) }
    }
    #[cfg(target_arch = "x86_64")]
    {
        crate::x86_64::boot_paging::image_range()
            .map(|(base, size)| (base as usize, (base + size) as usize))
            .unwrap_or((0, 0))
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        compile_error!("Unsupported architecture");
    }
}
