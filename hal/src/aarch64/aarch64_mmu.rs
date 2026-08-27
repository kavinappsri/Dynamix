//! Aarch64 post-boot mmu support
//!
//! On Demand mapping of additional physical memory. Can be mappad as both
//! device or normal

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::mmu::MmuError;

unsafe extern "C" {
    static _boottables_start: u8;
    static _devicetables_start: u8;
    static _devicetables_end: u8;
}

static TTBR0_ROOT: AtomicU64 = AtomicU64::new(0);
/// Difference between the kernel's higher-half virtual and physical addresses.
static KERNEL_VIRTUAL_OFFSET: AtomicU64 = AtomicU64::new(0);
static NEXT_FREE_TABLE: AtomicU64 = AtomicU64::new(0);
static TABLE_REGION_END: AtomicU64 = AtomicU64::new(0);
static MMU_STATE_READY: AtomicU64 = AtomicU64::new(0);


/// Number of 8-byte descriptors per table (4KB table / 8 bytes each).
const ENTRIES_PER_TABLE: usize = 512;

/// L2 block descriptor byte size (4KB granule, level 2).
const L2_BLOCK_SIZE: u64 = 2 * 1024 * 1024; // 2MB

/// L3 page descriptor byte size (4KB granule, level 3).
const L3_PAGE_SIZE: u64 = 4 * 1024; // 4KB


// Descriptor bit definitions (VMSAv8-64, 4KB granule)

const DESC_VALID: u64 = 1 << 0;
/// Bit 1: 1 = table descriptor (L1/L2) or page descriptor (L3).
///        0 = block descriptor (L1/L2 only).
const DESC_TABLE_OR_PAGE: u64 = 1 << 1;
/// Access flag - must be set or every first access takes an Access Flag fault
const DESC_AF: u64 = 1 << 10;
/// Shareability: inner-shareable, required for cache coherency across
/// cores on normal memory.
const DESC_SH_INNER: u64 = 0b11 << 8;
/// MAIR index 0 (programmed by aarch64_boot.s's _mmu_init): normal,
/// write-back cacheable memory.
const ATTR_IDX_NORMAL: u64 = 0 << 2;
/// MAIR index 1: device memory (nGnRnE), used for MMIO mappings.
const ATTR_IDX_DEVICE: u64 = 1 << 2;

/// Converts a higher-half kernel address to its physical address.
///
/// Translation-table descriptors always contain physical addresses, even
/// though Rust accesses the tables through their higher-half virtual mapping.
#[inline]
pub fn va_to_pa(address: u64) -> u64 {
    address.wrapping_sub(KERNEL_VIRTUAL_OFFSET.load(Ordering::Relaxed))
}


/// A single page-table page: 512 x 8-byte descriptors, 4KB total.
#[repr(C, align(4096))]
struct Table([u64; ENTRIES_PER_TABLE]);



/// Bump allocator over .devicetables region
struct TableBumpAllocator {
    next: *mut Table,
    end: *mut Table,
}

impl TableBumpAllocator {
    fn alloc_table(&mut self) -> Option<*mut Table> {
        if self.next >= self.end {
            return None;
        }
        let table = self.next;
        unsafe {
            core::ptr::write_bytes(table, 0, 1);
            self.next = self.next.add(1);
        }
        Some(table)
    }
}

/// Memory type for a mapping
pub enum MemType {
    Normal,
    Device,
}

impl MemType {
    const fn descriptor_bits(self) -> u64 {
        match self {
            MemType::Normal => ATTR_IDX_NORMAL | DESC_SH_INNER,
            MemType::Device => ATTR_IDX_DEVICE,
        }
    }
}

/// Maps `[phys_start, phys_start + len)` to `[virt_start, virt_start + len)`
fn map_range(
    root: &mut Table,
    alloc: &mut TableBumpAllocator,
    phys_start: u64,
    virt_start: u64,
    len: u64,
    mem_type: MemType,
) -> Result<(), MmuError> {
    let attrs = DESC_VALID | DESC_AF | mem_type.descriptor_bits();

    let mut offset = 0u64;
    while offset < len {
        let va = virt_start + offset;
        let pa = phys_start + offset;

        let l1_idx = ((va >> 30) & 0x1FF) as usize;
        let l2_idx = ((va >> 21) & 0x1FF) as usize;

        let l2_table = table_at_or_alloc(root, l1_idx, alloc)?;

        let remaining = len - offset;
        let can_use_block =
            (va % L2_BLOCK_SIZE == 0) && (pa % L2_BLOCK_SIZE == 0) && (remaining >= L2_BLOCK_SIZE);

        if can_use_block {
            l2_table.0[l2_idx] = pa | attrs;
            offset += L2_BLOCK_SIZE;
        } else {
            let l3_table = table_at_or_alloc(l2_table, l2_idx, alloc)?;
            let l3_idx = ((va >> 12) & 0x1FF) as usize;
            l3_table.0[l3_idx] = pa | attrs | DESC_TABLE_OR_PAGE;
            offset += L3_PAGE_SIZE;
        }
    }

    Ok(())
}

/// Returns the next-level table pointed to by `parent.0[idx]`, allocating
/// and installing a fresh one if that entry is not yet a valid table
fn table_at_or_alloc<'a>(
    parent: &mut Table,
    idx: usize,
    alloc: &mut TableBumpAllocator,
) -> Result<&'a mut Table, MmuError> {
    let entry = parent.0[idx];
    let table_ptr = if entry & DESC_VALID != 0 {
        (entry & 0x0000_FFFF_FFFF_F000) as *mut Table
    } else {
        let new_table = alloc.alloc_table().ok_or(MmuError::OutOfTableSpace)?;
        parent.0[idx] = va_to_pa(new_table as u64)
            | DESC_VALID
            | DESC_TABLE_OR_PAGE;
        new_table
    };
    Ok(unsafe { &mut *table_ptr })
}

/// Publishes the live `TTBR0_EL1` root
pub fn init() -> Result<(), MmuError> {
    if MMU_STATE_READY.compare_exchange(0, u64::MAX, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err(MmuError::AlreadyInitialized);
    }

    let ttbr0_root: u64;
    let boottables_start: u64;
    let devicetables_start: u64;
    let devicetables_end: u64;
    unsafe {
        asm!("mrs {}, ttbr0_el1", out(reg) ttbr0_root, options(nostack, nomem));
        boottables_start = &raw const _boottables_start as u64;
        devicetables_start = &raw const _devicetables_start as u64;
        devicetables_end = &raw const _devicetables_end as u64;
    }

    let ttbr0_root = ttbr0_root & 0x0000_FFFF_FFFF_F000;
    TTBR0_ROOT.store(ttbr0_root, Ordering::Relaxed);
    KERNEL_VIRTUAL_OFFSET.store(boottables_start.wrapping_sub(ttbr0_root), Ordering::Relaxed);
    NEXT_FREE_TABLE.store(devicetables_start, Ordering::Relaxed);
    TABLE_REGION_END.store(devicetables_end, Ordering::Relaxed);
    MMU_STATE_READY.store(1, Ordering::Release);

    Ok(())
}

/// Identity-maps `len` bytes of ordinary (cacheable) memory starting at
/// the physical address `phys` -- for RAM regions that are not the
/// kernel's own image and were not covered by the boot-time mapping
/// (`_mmu_init`'s fixed 32MB window), most notably the boot DTB blob,
/// which the bootloader can place anywhere in RAM.
///
/// # Errors
/// Returns [`MmuError::MmuNotReady`] if called before [`init`], or
/// [`MmuError::OutOfTableSpace`] if the reserved `.devicetables`
/// region is exhausted.
pub fn map_normal(phys: u64, len: u64) -> Result<(), MmuError> {
    map_impl(phys, len, MemType::Normal)
}

/// Identity-maps `len` bytes of device (MMIO) memory starting at the
/// physical address `phys`, so that ordinary loads/stores to that
/// range succeed once the MMU is enabled.
///
/// Device MMIO is identity-mapped (VA == PA) into TTBR0's range,
///  Call this with a (base, size) pair discovered from the boot DTB before touching the
/// corresponding device's registers; a device access before its
/// `map_device` call takes a translation fault.
///
/// # Errors
/// Returns [`MmuError::MmuNotReady`] if called before [`init`], or
/// [`MmuError::OutOfTableSpace`] if the reserved `.devicetables`
/// region is exhausted.
pub fn map_device(phys: u64, len: u64) -> Result<(), MmuError> {
    map_impl(phys, len, MemType::Device)
}

fn map_impl(phys: u64, len: u64, mem_type: MemType) -> Result<(), MmuError> {
    if MMU_STATE_READY.load(Ordering::Acquire) != 1 {
        return Err(MmuError::MmuNotReady);
    }

    let aligned_phys = phys & !0xFFF;
    let align_adjust = phys - aligned_phys;
    let aligned_len = (len + align_adjust + 0xFFF) & !0xFFF;

    let ttbr0_root_ptr = TTBR0_ROOT.load(Ordering::Relaxed) as *mut Table;
    let next = NEXT_FREE_TABLE.load(Ordering::Relaxed) as *mut Table;
    let end = TABLE_REGION_END.load(Ordering::Relaxed) as *mut Table;

    let result = unsafe {
        let mut alloc = TableBumpAllocator { next, end };
        let ttbr0_root = &mut *ttbr0_root_ptr;
        map_range(ttbr0_root, &mut alloc, aligned_phys, aligned_phys, aligned_len, mem_type).map(|()| alloc.next)
    };

    let new_next = match result {
        Ok(next) => next,
        Err(e) => return Err(e),
    };

    NEXT_FREE_TABLE.store(new_next as u64, Ordering::Relaxed);

    unsafe {
        let mut va = aligned_phys;
        let mapped_end = aligned_phys + aligned_len;
        while va < mapped_end {
            asm!("tlbi vaae1is, {}", in(reg) va >> 12, options(nostack));
            va += L3_PAGE_SIZE;
        }
        asm!("dsb ish", "isb", options(nostack));
    }

    Ok(())
}
