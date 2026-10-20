//! `GetMemoryMap` retrieval.

use core::cell::UnsafeCell;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, Ordering};

use super::types::{self, EfiBootServices, EfiMemoryDescriptor, EfiStatus};
use crate::services::addr::{PhysAddr, PhysRange};

/// Physical memory below this address is never reported as usable.
const LOW_MEMORY_LIMIT: usize = 1 << 20;

/// UEFI memory descriptors count in 4 KiB pages, fixed by the spec.
const UEFI_PAGE_SIZE: usize = 4096;

/// Size of the static descriptor buffer (room for well over a thousand
/// descriptors; typical firmware reports under a hundred). It lives in `.bss`
/// because there is no allocator yet.
const BUFFER_BYTES: usize = 64 * 1024;

/// Smallest descriptor we can parse: u32 type + u32 padding + four u64s.
const MIN_DESCRIPTOR_SIZE: usize = size_of::<u32>() * 2 + size_of::<u64>() * 4;

#[repr(C, align(8))]
struct MapBuffer(UnsafeCell<[u8; BUFFER_BYTES]>);

// SAFETY: handed out at most once, through `MemoryMap::take`.
unsafe impl Sync for MapBuffer {}

static BUFFER: MapBuffer = MapBuffer(UnsafeCell::new([0; BUFFER_BYTES]));
static BUFFER_TAKEN: AtomicBool = AtomicBool::new(false);

/// The result of `GetMemoryMap`, stored in a static buffer.
pub struct MemoryMap {
    buffer: &'static mut [u8],
    map_size: usize,
    /// Opaque key identifying the current snapshot. `ExitBootServices` needs
    /// it unchanged; any intervening allocation invalidates it.
    pub map_key: usize,
    /// Size in bytes of one descriptor, as reported by firmware.
    pub descriptor_size: usize,
    pub descriptor_version: u32,
}

/// Errors that can occur while fetching the memory map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryMapError {
    /// Firmware's map did not fit the static buffer; `needed` is the size it asked for.
    BufferTooSmall { needed: usize },
    /// Firmware's `descriptor_size` is smaller than the fields we read.
    DescriptorSizeTooSmall { reported: usize },
    /// `GetMemoryMap` returned an error status.
    GetMemoryMapFailed(EfiStatus),
}

impl MemoryMap {
    /// Claims the static buffer. `None` if it was already claimed.
    pub fn take() -> Option<Self> {
        if BUFFER_TAKEN.swap(true, Ordering::AcqRel) {
            return None;
        }
        // SAFETY: the flag above guarantees this is the only reference.
        let buffer = unsafe { &mut *BUFFER.0.get() };
        Some(Self { buffer, map_size: 0, map_key: 0, descriptor_size: 0, descriptor_version: 0 })
    }

    /// Calls `GetMemoryMap`, replacing the stored snapshot.
    ///
    /// # Safety
    /// `boot_services` must be valid (Boot Services not yet exited).
    pub unsafe fn refresh(&mut self, boot_services: *mut EfiBootServices) -> Result<(), MemoryMapError> {
        let mut size = BUFFER_BYTES;
        let mut key = 0usize;
        let mut descriptor_size = 0usize;
        let mut version = 0u32;

        // SAFETY: upheld by the caller; the buffer is `BUFFER_BYTES` long.
        let status = unsafe {
            ((*boot_services).get_memory_map)(
                &mut size,
                self.buffer.as_mut_ptr().cast::<EfiMemoryDescriptor>(),
                &mut key,
                &mut descriptor_size,
                &mut version,
            )
        };

        if types::efi_is_error(status) {
            return Err(match status {
                types::EFI_BUFFER_TOO_SMALL => MemoryMapError::BufferTooSmall { needed: size },
                other => MemoryMapError::GetMemoryMapFailed(other),
            });
        }
        if descriptor_size < MIN_DESCRIPTOR_SIZE {
            return Err(MemoryMapError::DescriptorSizeTooSmall { reported: descriptor_size });
        }

        self.map_size = size;
        self.map_key = key;
        self.descriptor_size = descriptor_size;
        self.descriptor_version = version;
        Ok(())
    }

    pub fn len(&self) -> usize {
        if self.descriptor_size == 0 { 0 } else { self.map_size / self.descriptor_size }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every descriptor firmware returned, in the order it returned them.
    pub fn entries(&self) -> impl Iterator<Item = EfiMemoryDescriptor> + '_ {
        let stride = self.descriptor_size;

        (0..self.len()).map(move |i| {
            let bytes = &self.buffer[i * stride..(i + 1) * stride];
            // Offsets 0, 8, 16, 24, 32 are UEFI's EFI_MEMORY_DESCRIPTOR layout.
            EfiMemoryDescriptor {
                ty: u32::from_ne_bytes(bytes[0..4].try_into().unwrap()),
                physical_start: u64::from_ne_bytes(bytes[8..16].try_into().unwrap()),
                virtual_start: u64::from_ne_bytes(bytes[16..24].try_into().unwrap()),
                number_of_pages: u64::from_ne_bytes(bytes[24..32].try_into().unwrap()),
                attribute: u64::from_ne_bytes(bytes[32..40].try_into().unwrap()),
            }
        })
    }

    /// Writes the free RAM into `out`, sorted by address with touching
    /// descriptors merged, and returns how many regions were written. Regions
    /// that do not fit in `out` are dropped.
    ///
    /// TODO: also reclaim `EfiBootServicesCode/Data` once nothing uses
    /// them.
    pub fn fill_usable_regions(&self, out: &mut [PhysRange]) -> usize {
        let mut written = 0;

        for entry in self.entries() {
            if entry.ty != types::EFI_CONVENTIONAL_MEMORY || entry.number_of_pages == 0 {
                continue;
            }

            let mut range = PhysRange::new(
                PhysAddr::new(entry.physical_start as usize),
                entry.number_of_pages as usize * UEFI_PAGE_SIZE,
            );

            // Leave the first MiB (real-mode IVT/BDA, option ROMs, address 0)
            // out of the pool; frame 0 would also hand out a null pointer.
            if range.end().as_usize() <= LOW_MEMORY_LIMIT {
                continue;
            }
            if range.start().as_usize() < LOW_MEMORY_LIMIT {
                range = PhysRange::from_bounds(PhysAddr::new(LOW_MEMORY_LIMIT), range.end());
            }

            insert_region(out, &mut written, range);
        }

        written
    }
}

/// Inserts `region` into the address-sorted `out[..*written]`, merging it
/// with every entry it overlaps or touches. Drops it if it touches nothing and
/// the list is full.
fn insert_region(out: &mut [PhysRange], written: &mut usize, region: PhysRange) {
    let mut start = region.start();
    let mut end = region.end();

    // Entries are disjoint and sorted, so ends ascend too: the first entry
    // ending at or after `start` is the first one that can touch the region,
    // and the touching ones are contiguous from there.
    let first = out[..*written].iter().position(|r| r.end() >= start).unwrap_or(*written);
    let mut last = first; // exclusive
    while last < *written && out[last].start() <= end {
        start = start.min(out[last].start());
        end = end.max(out[last].end());
        last += 1;
    }
    let merged = PhysRange::from_bounds(start, end);

    if first == last {
        // Touches nothing: a plain insertion.
        if *written == out.len() {
            return;
        }
        out.copy_within(first..*written, first + 1);
        out[first] = merged;
        *written += 1;
    } else {
        // Replaces `first..last` with the one merged entry.
        out[first] = merged;
        let removed = last - first - 1;
        if removed > 0 {
            out.copy_within(last..*written, first + 1);
            *written -= removed;
        }
    }
}
