//! Reading the kernel's own PE image: its headers and base relocations.

use crate::x86_64::x64_mmu::{PagingError, MAX_IDENTITY_ADDRESS, PAGE_SIZE};

unsafe extern "C" {
    static __ImageBase: u8;
}

// PE / COFF constants.
const DOS_MAGIC: u16 = 0x5A4D;
const PE_SIGNATURE: u32 = 0x0000_4550;
const PE32_PLUS_MAGIC: u16 = 0x20B;
const RELOC_DIRECTORY_INDEX: u64 = 5;
const RELOC_ABSOLUTE: u16 = 0;
const RELOC_DIR64: u16 = 10;

/// Where the firmware put the binary
#[derive(Clone, Copy)]
pub(super) struct ImageInfo {
    pub(super) base: u64,
    pub(super) size: u64,
    reloc_rva: u32,
    reloc_size: u32,
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
pub(super) unsafe fn read_image_info() -> Result<ImageInfo, PagingError> {
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
pub(super) unsafe fn walk_relocations(image: &ImageInfo, mut fix: impl FnMut(u64)) -> Result<(), PagingError> {
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

