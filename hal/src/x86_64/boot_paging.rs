//! Boot-time page tables and the jump into the higher half.


use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::x86_64::boot_stack;
use crate::x86_64::x64_mmu::{self, MemKind, PagingError, KERNEL_VIRT_BASE, MAX_IDENTITY_ADDRESS, PAGE_SIZE};

unsafe extern "C" {
    static __ImageBase: u8;
}

const CR4_PGE: u64 = 1 << 7;

// PE / COFF constants.
const DOS_MAGIC: u16 = 0x5A4D;
const PE_SIGNATURE: u32 = 0x0000_4550;
const PE32_PLUS_MAGIC: u16 = 0x20B;
const RELOC_DIRECTORY_INDEX: u64 = 5;
const RELOC_ABSOLUTE: u16 = 0;
const RELOC_DIR64: u16 = 10;

static IMAGE_BASE: AtomicU64 = AtomicU64::new(0);
static IMAGE_SIZE: AtomicU64 = AtomicU64::new(0);

/// Where the firmware put the binary
#[derive(Clone, Copy)]
struct ImageInfo {
    base: u64,
    size: u64,
    reloc_rva: u32,
    reloc_size: u32,
}

/// Result of [`prepare`]: tables are built but not yet loaded.
pub struct Prepared {
    root: u64,
    image: ImageInfo,
}

/// Function [`activate`] ends in, running at its higher-half address on the
/// higher-half alias of the boot stack.
pub type HigherHalfEntry = unsafe extern "efiapi" fn() -> !;

/// Physical `(base, size)` of the kernel image once `prepare` has run.
pub fn image_range() -> Option<(u64, u64)> {
    match IMAGE_BASE.load(Ordering::Acquire) {
        0 => None,
        base => Some((base, IMAGE_SIZE.load(Ordering::Relaxed))),
    }
}

/// Reads a `T` from an arbitrary address.
///
/// # Safety
/// `address` must be readable for `size_of::<T>()` bytes.
unsafe fn read<T: Copy>(address: u64) -> T {
    unsafe { (address as *const T).read_unaligned() }
}

/// Reads the image's own PE headers.
///
/// # Safety
/// Must run on the kernel image as loaded by firmware (headers intact).
unsafe fn read_image_info() -> Result<ImageInfo, PagingError> {
    let base = (&raw const __ImageBase) as u64;
    if base & (PAGE_SIZE - 1) != 0 {
        return Err(PagingError::BadImage);
    }


    unsafe {
        if read::<u16>(base) != DOS_MAGIC {
            return Err(PagingError::BadImage);
        }
        let pe = base + read::<u32>(base + 0x3C) as u64;
        if read::<u32>(pe) != PE_SIGNATURE {
            return Err(PagingError::BadImage);
        }

        let optional = pe + 24;
        if read::<u16>(optional) != PE32_PLUS_MAGIC {
            return Err(PagingError::BadImage);
        }

        let size = read::<u32>(optional + 56) as u64;
        let directory = optional + 112 + RELOC_DIRECTORY_INDEX * 8;
        let image = ImageInfo {
            base,
            size: (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1),
            reloc_rva: read::<u32>(directory),
            reloc_size: read::<u32>(directory + 4),
        };


        if image.size == 0 || image.size >= 0x8000_0000 || image.reloc_rva == 0 || image.reloc_size == 0 {
            return Err(PagingError::BadImage);
        }
        if image.base.checked_add(image.size).is_none_or(|end| end > MAX_IDENTITY_ADDRESS) {
            return Err(PagingError::AddressTooHigh);
        }
        Ok(image)
    }
}

/// Walks the image's base-relocation table, calling `fix` with the address of
/// every 64-bit fixup. Fails on anything but `DIR64` / padding entries.
unsafe fn walk_relocations(image: &ImageInfo, mut fix: impl FnMut(u64)) -> Result<(), PagingError> {
    let mut cursor = image.base + image.reloc_rva as u64;
    let end = cursor + image.reloc_size as u64;

    while cursor + 8 <= end {
        // SAFETY: upheld by the caller; block bounds are checked below.
        let (page_rva, block_size) = unsafe { (read::<u32>(cursor), read::<u32>(cursor + 4) as u64) };
        if block_size < 8 || cursor + block_size > end {
            return Err(PagingError::BadImage);
        }

        let mut entry_at = cursor + 8;
        while entry_at < cursor + block_size {
            let entry = unsafe { read::<u16>(entry_at) };
            match entry >> 12 {
                RELOC_ABSOLUTE => {}
                RELOC_DIR64 => fix(image.base + page_rva as u64 + (entry & 0x0FFF) as u64),
                _ => return Err(PagingError::BadImage),
            }
            entry_at += 2;
        }
        cursor += block_size;
    }
    Ok(())
}

/// Builds the boot tables for the image, without loading them.
///
/// This is for a recovarable state, `CR3` isnt touched yet, so the stack
/// nd framebuffer shud be fine, which would allow us to give error msg if an error
/// occurs

pub unsafe fn prepare() -> Result<Prepared, PagingError> {
    if x64_mmu::root().is_some() {
        return Err(PagingError::AlreadyInstalled);
    }

    let image = unsafe { read_image_info()? };

    unsafe { walk_relocations(&image, |_| {})? };

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
        walk_relocations(&image, |address| {
            let slot = address as *mut u64;
            slot.write_unaligned(slot.read_unaligned().wrapping_add(delta));
        })
    };

    x64_mmu::set_virt_offset(delta);


    let entry_va = (entry as usize as u64).wrapping_add(delta);
    let stack_va = (boot_stack::stack_top() as u64).wrapping_add(delta);


    unsafe { boot_stack::call_on_stack(stack_va as usize, entry_va as usize, 0) }
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
