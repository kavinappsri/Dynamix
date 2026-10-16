//! `GetMemoryMap` retrieval.

use core::mem::size_of;

/// Physical memory below this address is never reported as usable.
const LOW_MEMORY_LIMIT: usize = 1 << 20;

use crate::services::device_query::MemoryRegion;
use crate::x86_64::uefi_types;

/// UEFI memory descriptors count in 4 KiB pages, fixed by the spec.
const UEFI_PAGE_SIZE: usize = 4096;

/// Static storage for the fetched map. There is no heap allocator yet so this
/// is a fixed-size buffer
const BUFFER_BYTES: usize = 16 * 1024;

/// Descriptor storage, aligned the way firmware expects to write it.
#[repr(C, align(8))]
struct MapBuffer([u8; BUFFER_BYTES]);

/// `GetMemoryMap` result
pub struct MemoryMap {
    buffer: MapBuffer,
    map_size: usize,
    /// Opaque key identifying this exact map snapshot. Required unchanged by
    /// `ExitBootServices`. any intervening allocation invalidates it.
    pub map_key: usize,
    /// Size in bytes of one descriptor, as reported by firmware
    pub descriptor_size: usize,
    pub descriptor_version: u32,
}

impl MemoryMap {
    /// Every descriptor firmware returned, in the order it returned them.
    pub fn entries(&self) -> impl Iterator<Item = uefi_types::EfiMemoryDescriptor> + '_ {
        let stride = self.descriptor_size;

        (0..self.len()).map(move |i| {
            let offset = i * stride;
            let bytes = &self.buffer.0[offset..offset + stride];
            // The field offsets (0, 8, 16, 24, 32) are UEFI's
            // EFI_MEMORY_DESCRIPTOR layout - u32 Type + u32 padding, then
            // four u64s.
            uefi_types::EfiMemoryDescriptor {
                ty: u32::from_ne_bytes(bytes[0..4].try_into().unwrap()),
                physical_start: u64::from_ne_bytes(bytes[8..16].try_into().unwrap()),
                virtual_start: u64::from_ne_bytes(bytes[16..24].try_into().unwrap()),
                number_of_pages: u64::from_ne_bytes(bytes[24..32].try_into().unwrap()),
                attribute: u64::from_ne_bytes(bytes[32..40].try_into().unwrap()),
            }
        })
    }

    pub fn len(&self) -> usize {
        if self.descriptor_size == 0 {
            0
        } else {
            self.map_size / self.descriptor_size
        }
    }

    /// Writes the free RAM (`EfiConventionalMemory`) into `out`, merging
    /// physically adjacent descriptors, and returns how many regions were
    /// written.
    pub fn fill_usable_regions(&self, out: &mut [MemoryRegion]) -> usize {
        let mut written = 0;

        for entry in self.entries() {
            if entry.ty != uefi_types::EFI_CONVENTIONAL_MEMORY || entry.number_of_pages == 0 {
                continue;
            }

            let mut base = entry.physical_start as usize;
            let mut size = entry.number_of_pages as usize * UEFI_PAGE_SIZE;

            // Leave the first MiB (real-mode IVT/BDA, option ROMs, address 0) out
            // of the pool: nothing there is worth having, and frame 0 would hand
            // out a null pointer.
            let end = base + size;
            if end <= LOW_MEMORY_LIMIT {
                continue;
            }
            if base < LOW_MEMORY_LIMIT {
                base = LOW_MEMORY_LIMIT;
                size = end - LOW_MEMORY_LIMIT;
            }

            if written > 0 {
                let last = &mut out[written - 1];
                if last.base + last.size == base {
                    last.size += size;
                    continue;
                }
            }

            if written == out.len() {
                break;
            }
            out[written] = MemoryRegion { base, size };
            written += 1;
        }

        written
    }
}

/// Errors that can occur while fetching the memory map
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryMapError {
    /// Firmware's map did not fit the static buffer; `needed` is the size it asked for.
    BufferTooSmall { needed: usize },
    /// Firmware's `descriptor_size` is smaller than the descriptor fields we
    /// read (This indicates we need to rewrite this section of the code)
    DescriptorSizeTooSmall { reported: usize },
    /// `GetMemoryMap` returned an error status.
    GetMemoryMapFailed(uefi_types::EfiStatus),
}

/// Calls `GetMemoryMap` and copies every descriptor into a static buffer.
pub unsafe fn get_memory_map(boot_services: *mut uefi_types::EfiBootServices) -> Result<MemoryMap, MemoryMapError> {
    let mut map = MemoryMap {
        buffer: MapBuffer([0u8; BUFFER_BYTES]),
        map_size: 0,
        map_key: 0,
        descriptor_size: 0,
        descriptor_version: 0,
    };

    let mut memory_map_size = BUFFER_BYTES;
    let mut map_key: usize = 0;
    let mut descriptor_size: usize = 0;
    let mut descriptor_version: u32 = 0;

    let status = unsafe {
        ((*boot_services).get_memory_map)(
            &mut memory_map_size,
            map.buffer.0.as_mut_ptr() as *mut _,
            &mut map_key,
            &mut descriptor_size,
            &mut descriptor_version,
        )
    };

    if uefi_types::efi_is_error(status) {
        return match status {
            uefi_types::EFI_BUFFER_TOO_SMALL => {
                Err(MemoryMapError::BufferTooSmall {
                    needed: memory_map_size,
                })
            }
            other => Err(MemoryMapError::GetMemoryMapFailed(other)),
        };
    }

    const MIN_DESCRIPTOR_SIZE: usize = size_of::<u32>() * 2 + size_of::<u64>() * 4;
    if descriptor_size < MIN_DESCRIPTOR_SIZE {
        return Err(MemoryMapError::DescriptorSizeTooSmall {
            reported: descriptor_size,
        });
    }

    map.map_size = memory_map_size;
    map.map_key = map_key;
    map.descriptor_size = descriptor_size;
    map.descriptor_version = descriptor_version;

    Ok(map)
}