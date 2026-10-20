//! Device-tree boot path (aarch64 and armv7).

use crate::services::boot_info::{BootInfo, HardwareDescription, RegionList};
use crate::services::dtb::Dtb;

/// Called by the assembly boot stub with the bootloader's DTB pointer, once
/// the MMU is on and the CPU is running in the higher half.
#[unsafe(no_mangle)]
pub extern "C" fn fdt_entry(dtb_ptr: usize) -> ! {
    // SAFETY: the bootloader passed a pointer to a DTB it placed in RAM.
    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!("invalid DTB"),
    };

    let mut boot = BootInfo::empty();
    boot.memory_regions = RegionList::from_fill(|out| dtb.memory_regions(out));
    boot.hardware_reserved_range = Some(dtb.physical_range());
    boot.hardware = Some(HardwareDescription::Fdt(dtb));

    super::store(boot);
    // SAFETY: just stored, and complete.
    unsafe { super::enter_kernel() }
}
