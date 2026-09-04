//! Armv7 post boot mmu support
//!
//! On-demand mapping of additional physical memory into TTBR0. Can be
//! mapped as either device or normal

use crate::services::mmu::MmuError;
use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

unsafe extern "C" {
    static _boottables_start: u8;
    static _devicetables_start: u8;
    static _devicetables_end: u8;
}



static TTBR0_ROOT: AtomicU32 = AtomicU32::new(0);
static KERNEL_VIRTUAL_OFFSET: AtomicU32 = AtomicU32::new(0);
static NEXT_FREE_TABLE: AtomicU32 = AtomicU32::new(0);
static TABLE_REGION_END: AtomicU32 = AtomicU32::new(0);
static MMU_STATE_READY: AtomicU32 = AtomicU32::new(0);



/// Number of 4-byte descriptors per L1 table (16KB table / 4 bytes each).
const L1_ENTRIES: usize = 4096;
/// Number of 4-byte descriptors per L2 table (1KB table / 4 bytes each).
const L2_ENTRIES: usize = 256;

/// L1 section descriptor byte size (short-descriptor format).
const L1_SECTION_SIZE: u32 = 1 * 1024 * 1024; // 1MB
/// L2 small-page descriptor byte size (short-descriptor format).
const L2_PAGE_SIZE: u32 = 4 * 1024; // 4KB

// Descriptor bit definitions (VMSAv7 short-descriptor format).

/// Bits[1:0] = 0b01: L1 entry is a page-table (coarse) descriptor pointing
/// at an L2 table.
const L1_DESC_TABLE: u32 = 0b01;
/// Bits[1:0] = 0b10: L1 entry is a 1MB section descriptor.
const L1_DESC_SECTION: u32 = 0b10;
/// Bits[1:0] = 0b10: L2 entry is a 4KB small-page descriptor
const L2_DESC_SMALL_PAGE: u32 = 0b10;

/// AP[1:0] = 0b11, AP[2] = 0: full read/write access for privileged and
/// unprivileged accesses. Section descriptors store AP[1:0] in bits[11:10],
/// while small-page descriptors store them in bits[5:4].
const DESC_AP_FULL_SECTION: u32 = 0b11 << 10;
const DESC_AP_FULL_PAGE: u32 = 0b11 << 4;
/// Shareable (bit16 for sections, bit10 for small pages -- see MemType).
const DESC_S_SECTION: u32 = 1 << 16;
const DESC_S_PAGE: u32 = 1 << 10;

/// TEX=0b001, C=1, B=1: Normal memory, outer & inner write-back
/// write-allocate. TEX lives at bits[14:12] for sections, bits[8:6] for
/// small pages; C/B sit at bits[3:2] for both.
const DESC_NORMAL_SECTION: u32 = (0b001 << 12) | (1 << 3) | (1 << 2);
const DESC_NORMAL_PAGE: u32 = (0b001 << 6) | (1 << 3) | (1 << 2);
/// TEX=0b001, C=0, B=0: Normal memory, non-cacheable (outer & inner).
const DESC_NORMAL_NC_SECTION: u32 = 0b001 << 12;
const DESC_NORMAL_NC_PAGE: u32 = 0b001 << 6;
/// TEX=0, C=0, B=0: Strongly-ordered/Device memory, used for MMIO.
const DESC_DEVICE: u32 = 0;



/// Converts a higher-half kernel address to its physical address.
pub fn va_to_pa(address: u32) -> u32 {
    address.wrapping_sub(KERNEL_VIRTUAL_OFFSET.load(Ordering::Relaxed))
}


/// The fixed L1 translation table: 4096 x 4-byte descriptors, 16KB total,
/// 16KB-aligned. Built at boot time; never bump-allocated.
#[repr(C, align(16384))]
struct L1Table([u32; L1_ENTRIES]);

/// An L2 (coarse) page table: 256 x 4-byte descriptors, 1KB total,
/// 1KB-aligned. Bump-allocated on demand from `.devicetables`.
#[repr(C, align(1024))]
struct L2Table([u32; L2_ENTRIES]);

/// Bump allocator over the `.devicetables` region, handing out L2 tables.
struct TableBumpAllocator {
    next: *mut L2Table,
    end: *mut L2Table,
}

impl TableBumpAllocator {
    fn alloc_table(&mut self) -> Option<*mut L2Table> {
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

/// Memory type for a mapping.
#[derive(Copy, Clone)]
pub enum MemType {
    Normal,
    NormalNonCacheable,
    Device,
}

impl MemType {
    const fn section_bits(self) -> u32 {
        match self {
            MemType::Normal => DESC_NORMAL_SECTION | DESC_S_SECTION,
            MemType::NormalNonCacheable => DESC_NORMAL_NC_SECTION | DESC_S_SECTION,
            MemType::Device => DESC_DEVICE,
        }
    }

    const fn page_bits(self) -> u32 {
        match self {
            MemType::Normal => DESC_NORMAL_PAGE | DESC_S_PAGE,
            MemType::NormalNonCacheable => DESC_NORMAL_NC_PAGE | DESC_S_PAGE,
            MemType::Device => DESC_DEVICE,
        }
    }
}

/// Maps `[phys_start, phys_start + len)` to `[virt_start, virt_start + len)`.
fn map_range(
    root: &mut L1Table,
    alloc: &mut TableBumpAllocator,
    phys_start: u32,
    virt_start: u32,
    len: u32,
    mem_type: MemType,
) -> Result<(), MmuError> {
    let section_attrs = L1_DESC_SECTION | DESC_AP_FULL_SECTION | mem_type.section_bits();
    let page_attrs = L2_DESC_SMALL_PAGE | DESC_AP_FULL_PAGE | mem_type.page_bits();

    let mut offset = 0u32;
    while offset < len {
        let va = virt_start + offset;
        let pa = phys_start + offset;

        let remaining = len - offset;
        let can_use_section =
            (va % L1_SECTION_SIZE == 0) && (pa % L1_SECTION_SIZE == 0) && (remaining >= L1_SECTION_SIZE);

        if can_use_section {
            let l1_idx = (va >> 20) as usize;
            root.0[l1_idx] = pa | section_attrs;
            offset += L1_SECTION_SIZE;
        } else {
            let l1_idx = (va >> 20) as usize;
            let l2_table = l2_table_at_or_alloc(root, l1_idx, alloc)?;
            let l2_idx = ((va >> 12) & 0xFF) as usize;
            l2_table.0[l2_idx] = pa | page_attrs;
            offset += L2_PAGE_SIZE;
        }
    }

    Ok(())
}

/// Returns the L2 table pointed to by `root.0[idx]`, allocating and
/// installing a fresh one if that entry is not yet a valid table descriptor.
///
/// # Errors
/// Returns [`MmuError::OutOfTableSpace`] if the L1 slot already holds a
/// 1MB section (would require splitting - not supported) or the bump
/// allocator is exhausted.
fn l2_table_at_or_alloc<'a>(
    root: &mut L1Table,
    idx: usize,
    alloc: &mut TableBumpAllocator,
) -> Result<&'a mut L2Table, MmuError> {
    let entry = root.0[idx];
    let table_ptr = if entry & 0b11 == L1_DESC_TABLE {
        (entry & 0xFFFF_FC00) as *mut L2Table
    } else if entry & 0b11 == 0 {
        let new_table = alloc.alloc_table().ok_or(MmuError::OutOfTableSpace)?;
        root.0[idx] = va_to_pa(new_table as u32) | L1_DESC_TABLE;
        new_table
    } else {
        // Entry is already a 1MB section descriptor covering this index;
        // splitting an existing section into pages isn't supported.
        return Err(MmuError::OutOfTableSpace);
    };
    Ok(unsafe { &mut *table_ptr })
}

/// Publishes the live `TTBR0` root.
pub fn init() -> Result<(), MmuError> {
    if MMU_STATE_READY.compare_exchange(0, u32::MAX, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err(MmuError::AlreadyInitialized);
    }

    let ttbr0_root: u32;
    let boottables_start: u32;
    let devicetables_start: u32;
    let devicetables_end: u32;
    unsafe {
        asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttbr0_root, options(nostack, nomem));
        boottables_start = &raw const _boottables_start as u32;
        devicetables_start = &raw const _devicetables_start as u32;
        devicetables_end = &raw const _devicetables_end as u32;
    }

    // TTBR0's low bits hold walk attributes (see armv7_boot.s), not part of
    // the base address; the L1 table is 16KB-aligned so bits[13:0] are free.
    let ttbr0_root = ttbr0_root & 0xFFFF_C000;
    TTBR0_ROOT.store(ttbr0_root, Ordering::Relaxed);
    KERNEL_VIRTUAL_OFFSET.store(boottables_start.wrapping_sub(ttbr0_root), Ordering::Relaxed);
    NEXT_FREE_TABLE.store(devicetables_start, Ordering::Relaxed);
    TABLE_REGION_END.store(devicetables_end, Ordering::Relaxed);
    MMU_STATE_READY.store(1, Ordering::Release);

    Ok(())
}

/// Identity-maps `len` bytes of ordinary (cacheable) memory starting at
/// the physical address `phys`
///
/// # Errors
/// Returns [`MmuError::MmuNotReady`] if called before [`init`], or
/// [`MmuError::OutOfTableSpace`] if the reserved `.devicetables`
/// region is exhausted.
pub fn map_normal(phys: u32, len: u32) -> Result<(), MmuError> {
    map_impl(phys, len, MemType::Normal)
}

/// Identity-maps `len` bytes of Normal, non-cacheable memory starting at
/// the physical address `phys`
///
/// # Errors
///Returns [`MmuError::MmuNotReady`] if called before [`init`], or
/// [`MmuError::OutOfTableSpace`] if the reserved `.devicetables`
/// region is exhausted.
pub fn map_normal_nc(phys: u32, len: u32) -> Result<(), MmuError> {
    map_impl(phys, len, MemType::NormalNonCacheable)
}

/// Identity-maps `len` bytes of device (MMIO) memory starting at the
/// physical address `phys`, so that ordinary loads/stores to that
/// range succeed once the MMU is enabled.
///
/// # Errors
/// Returns [`MmuError::MmuNotReady`] if called before [`init`], or
/// [`MmuError::OutOfTableSpace`] if the reserved `.devicetables`
/// region is exhausted.
pub fn map_device(phys: u32, len: u32) -> Result<(), MmuError> {
    map_impl(phys, len, MemType::Device)
}

fn map_impl(phys: u32, len: u32, mem_type: MemType) -> Result<(), MmuError> {
    if MMU_STATE_READY.load(Ordering::Acquire) != 1 {
        return Err(MmuError::MmuNotReady);
    }

    let aligned_phys = phys & !0xFFF;
    let align_adjust = phys - aligned_phys;
    let aligned_len = (len + align_adjust + 0xFFF) & !0xFFF;

    let ttbr0_root_ptr = TTBR0_ROOT.load(Ordering::Relaxed) as *mut L1Table;
    let next = NEXT_FREE_TABLE.load(Ordering::Relaxed) as *mut L2Table;
    let end = TABLE_REGION_END.load(Ordering::Relaxed) as *mut L2Table;

    let result = unsafe {
        let mut alloc = TableBumpAllocator { next, end };
        let ttbr0_root = &mut *ttbr0_root_ptr;
        map_range(ttbr0_root, &mut alloc, aligned_phys, aligned_phys, aligned_len, mem_type).map(|()| alloc.next)
    };

    let new_next = match result {
        Ok(next) => next,
        Err(e) => return Err(e),
    };

    NEXT_FREE_TABLE.store(new_next as u32, Ordering::Relaxed);

    unsafe {
        let mut va = aligned_phys;
        let mapped_end = aligned_phys + aligned_len;
        while va < mapped_end {
            asm!("mcr p15, 0, {}, c8, c7, 1", in(reg) va, options(nostack)); // TLBIMVA
            va += L2_PAGE_SIZE;
        }
        asm!("dsb", "isb", options(nostack));
    }

    Ok(())
}
