//! Boot-time page tables and the jump into the higher half.


use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use super::pe_image::{self, ImageInfo};
use super::stack;
use crate::services::addr::{PhysAddr, PhysRange};
use crate::x86_64::x64_mmu::{self, MemKind, PagingError, KERNEL_VIRT_BASE};

const CR4_PGE: u64 = 1 << 7;

static IMAGE_BASE: AtomicU64 = AtomicU64::new(0);
static IMAGE_SIZE: AtomicU64 = AtomicU64::new(0);

/// Result of [`prepare`]: tables are built but not yet loaded.
pub struct Prepared {
    root: u64,
    image: ImageInfo,
}

/// Function [`activate`] ends in, running at its higher-half address on the
/// higher-half alias of the boot stack.
pub type HigherHalfEntry = unsafe extern "efiapi" fn() -> !;

/// Physical range of the kernel image once `prepare` has run.
pub fn image_range() -> Option<PhysRange> {
    match IMAGE_BASE.load(Ordering::Acquire) {
        0 => None,
        base => Some(PhysRange::new(PhysAddr::new(base as usize), IMAGE_SIZE.load(Ordering::Relaxed) as usize)),
    }
}

/// Builds the boot tables for the image, without loading them.
///
/// TODO(hardening):  Setting EFER.NXE, mapping each PE section with its
/// own permissions and unmapping a page below the stack
///
/// Nothing here is irreversible: `CR3` is not touched yet, so the stack and
/// framebuffer are still fine and the caller can report an error.
///
/// # Safety
/// Must run once, on the kernel image as loaded by firmware.
pub unsafe fn prepare() -> Result<Prepared, PagingError> {
    if x64_mmu::root().is_some() {
        return Err(PagingError::AlreadyInstalled);
    }

    let image = unsafe { pe_image::read_image_info()? };

    unsafe { pe_image::walk_relocations(&image, |_| {})? };

    let root = x64_mmu::alloc_table()?;
    unsafe {

        x64_mmu::map_range(root, image.base, image.base, image.size, MemKind::Normal)?;
        x64_mmu::map_range(root, KERNEL_VIRT_BASE, image.base, image.size, MemKind::Normal)?;
    }

    x64_mmu::install_root(root)?;
    IMAGE_SIZE.store(image.size, Ordering::Relaxed);
    IMAGE_BASE.store(image.base, Ordering::Release);
    Ok(Prepared { root, image })
}

/// Loads the prepared tables, relocates the image to the higher half and
/// continues in `entry` on the higher-half boot stack.
pub unsafe fn activate(prepared: Prepared, entry: HigherHalfEntry) -> ! {
    let image = prepared.image;
    let delta = KERNEL_VIRT_BASE.wrapping_sub(image.base);


    unsafe { load_cr3(prepared.root) };


    let _ = unsafe {
        pe_image::walk_relocations(&image, |address| {
            let slot = address as *mut u64;
            slot.write_unaligned(slot.read_unaligned().wrapping_add(delta));
        })
    };

    x64_mmu::set_virt_offset(delta);


    let entry_va = (entry as usize as u64).wrapping_add(delta);
    let stack_va = (stack::stack_top() as u64).wrapping_add(delta);


    unsafe { stack::call_on_stack(stack_va as usize, entry_va as usize, 0) }
}

/// Loads `root` into `CR3`, also dropping any global TLB entries firmware's
/// tables may have left behind.
unsafe fn load_cr3(root: u64) {
    unsafe {
        let cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        if cr4 & CR4_PGE != 0 {
            asm!("mov cr4, {}", in(reg) cr4 & !CR4_PGE, options(nostack, preserves_flags));
        }
        asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
        if cr4 & CR4_PGE != 0 {
            asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
        }
    }
}
