//! Mmu Services Abstraction


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmuError {
    MmuNotReady,
    AlreadyInitialized,
    OutOfTableSpace,
}

pub fn init() -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::init()
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn map_device(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::map_device(phys as u64, len as u64)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn map_normal(phys: usize, len: usize) -> Result<(), MmuError> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::map_normal(phys as u64, len as u64)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        compile_error!("Unsupported architecture");
    }
}

pub fn va_to_pa(addr: usize) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        crate::aarch64::aarch64_mmu::va_to_pa(addr as u64) as usize
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        compile_error!("Unsupported architecture");
    }
}
