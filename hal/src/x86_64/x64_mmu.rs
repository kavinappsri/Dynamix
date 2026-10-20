//! x86_64 paging: table pool, mapping primitives and CPU setup.
//!
//! Locking: mapping entry points take a spin lock. Interrupts are masked in
//! the kernel for now; once they are enabled, callers that can run in
//! interrupt context must not map.

use core::arch::asm;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::services::addr::PhysRange;
use crate::services::mmu::MmuError;
use crate::services::sync::SpinLock;

/// The kernel image lives in virtual memory.
pub const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

pub(crate) const PAGE_SIZE: u64 = 4096;
const HUGE_PAGE_SIZE: u64 = 2 * 1024 * 1024;
const ENTRIES_PER_TABLE: usize = 512;

/// Table pages available.
const POOL_PAGES: usize = 128;

/// Highest address an identity mapping may reach.
pub(crate) const MAX_IDENTITY_ADDRESS: u64 = 1 << 47;

// Page-table entry bits.
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
/// In a PD entry: maps a 2 MiB page. In a PT entry the same bit is PAT.
const HUGE: u64 = 1 << 7;
const PAT_4K: u64 = 1 << 7;
const PAT_2M: u64 = 1 << 12;
const ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const HUGE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFE0_0000;

const PAGE_CACHE_MASK: u64 = WRITE_THROUGH | CACHE_DISABLE | PAT_4K;
const HUGE_CACHE_MASK: u64 = WRITE_THROUGH | CACHE_DISABLE | PAT_2M;

// IA32_PAT. Entry 4 (selected by PAT=1, PCD=0, PWT=0) is reprogrammed to
// write-combining and used for `NormalNc`; entries 0 (WB) and 3 (UC) keep
// their power-on meaning for `Normal` and `Device`.
const IA32_PAT: u32 = 0x277;
const PAT_ENTRY4_SHIFT: u32 = 32;
const PAT_TYPE_WC: u64 = 0x01;
const CPUID_1_EDX_PAT: u32 = 1 << 16;

#[repr(C, align(4096))]
struct Table([u64; ENTRIES_PER_TABLE]);

struct Pool(UnsafeCell<[Table; POOL_PAGES]>);

// SAFETY: every pool page is handed out exactly once (atomic counter).
unsafe impl Sync for Pool {}

static POOL: Pool = Pool(UnsafeCell::new([const { Table([0; ENTRIES_PER_TABLE]) }; POOL_PAGES]));
static POOL_USED: AtomicUsize = AtomicUsize::new(0);
/// Physical address of the PML4.
static ROOT: AtomicU64 = AtomicU64::new(0);
static VIRT_OFFSET: AtomicU64 = AtomicU64::new(0);
static READY: AtomicBool = AtomicBool::new(false);
static PAT_WC: AtomicBool = AtomicBool::new(false);
static MAP_LOCK: SpinLock<()> = SpinLock::new(());

/// Why a paging operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagingError {
    /// Already installed / initialised.
    AlreadyInstalled,
    /// Used before the boot tables were installed.
    NotInstalled,
    /// The static table pool ran out.
    OutOfTableSpace,
    /// A range lies above the lower half.
    AddressTooHigh,
    /// An address that must be page aligned was not.
    Misaligned,
    /// A remap touched memory that is not mapped.
    NotMapped,
    /// The kernel image headers were not what we expect.
    BadImage,
    /// Part of the range is already mapped with a different memory type.
    TypeConflict,
}

impl From<PagingError> for MmuError {
    fn from(error: PagingError) -> Self {
        match error {
            PagingError::OutOfTableSpace => MmuError::OutOfTableSpace,
            PagingError::NotInstalled => MmuError::MmuNotReady,
            PagingError::AlreadyInstalled => MmuError::AlreadyInitialized,
            PagingError::NotMapped => MmuError::NotMapped,
            PagingError::TypeConflict => MmuError::TypeConflict,
            PagingError::AddressTooHigh | PagingError::Misaligned | PagingError::BadImage => MmuError::InvalidRange,
        }
    }
}

/// Memory type of a mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemKind {
    /// Ordinary write-back RAM.
    Normal,
    /// Non-cacheable RAM-like memory
    NormalNc,
    /// Registers and other strongly uncacheable memory.
    Device,
}

impl MemKind {
    fn bits(self, huge: bool) -> u64 {
        let pat = if huge { PAT_2M } else { PAT_4K };
        match self {
            MemKind::Normal => 0,
            MemKind::Device => WRITE_THROUGH | CACHE_DISABLE,
            MemKind::NormalNc => {
                if PAT_WC.load(Ordering::Relaxed) {
                    pat
                } else {
                    WRITE_THROUGH | CACHE_DISABLE
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// State shared with the handoff code
// ---------------------------------------------------------------------------

/// Physical address of the PML4 once installed, `None` before.
pub fn root() -> Option<u64> {
    match ROOT.load(Ordering::Acquire) {
        0 => None,
        root => Some(root),
    }
}

/// Records the PML4 built by `handoff::paging::prepare`.
pub(crate) fn install_root(root: u64) -> Result<(), PagingError> {
    ROOT.compare_exchange(0, root, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| PagingError::AlreadyInstalled)
}

/// Records the virtual-minus-physical offset of the higher-half image alias.
pub(crate) fn set_virt_offset(offset: u64) {
    VIRT_OFFSET.store(offset, Ordering::Release);
}

/// Converts a kernel address to physical (lower-half addresses are already physical).
pub fn va_to_pa(address: usize) -> usize {
    let address = address as u64;
    if address >= KERNEL_VIRT_BASE {
        address.wrapping_sub(VIRT_OFFSET.load(Ordering::Relaxed)) as usize
    } else {
        address as usize
    }
}

// ---------------------------------------------------------------------------
// Table primitives
// ---------------------------------------------------------------------------

/// Virtual address of the pool table at physical address `phys`.
fn table_ptr(phys: u64) -> *mut Table {
    phys.wrapping_add(VIRT_OFFSET.load(Ordering::Relaxed)) as *mut Table
}

/// Hands out the next zeroed table page from the pool; returns its physical address.
pub(crate) fn alloc_table() -> Result<u64, PagingError> {
    let index = POOL_USED.fetch_add(1, Ordering::Relaxed);
    if index >= POOL_PAGES {
        return Err(PagingError::OutOfTableSpace);
    }
    // SAFETY: `index` is in bounds, and each index is handed out only once.
    let virt = unsafe {
        let table = (POOL.0.get() as *mut Table).add(index);
        core::ptr::write_bytes(table, 0, 1);
        table
    };
    Ok((virt as u64).wrapping_sub(VIRT_OFFSET.load(Ordering::Relaxed)))
}

/// Returns the physical address of the table `parent[index]` points at,
/// creating it if absent.
unsafe fn next_table(parent: u64, index: usize) -> Result<u64, PagingError> {
    let parent = table_ptr(parent);
    // SAFETY: upheld by the caller; `index` is masked to 9 bits by every caller.
    let entry = unsafe { (*parent).0[index] };
    if entry & PRESENT != 0 {
        return Ok(entry & ADDRESS_MASK);
    }
    let child = alloc_table()?;
    unsafe { (*parent).0[index] = child | PRESENT | WRITABLE };
    Ok(child)
}

/// Maps `[va, va + len)` to `[pa, pa + len)` as `kind`. Pages that are already
/// mapped with the same memory type are left alone; ones mapped with a
/// different type fail with [`PagingError::TypeConflict`] (use `remap_range`
/// to change a type on purpose).
pub(crate) unsafe fn map_range(root: u64, va: u64, pa: u64, len: u64, kind: MemKind) -> Result<(), PagingError> {
    if (va | pa) & (PAGE_SIZE - 1) != 0 {
        return Err(PagingError::Misaligned);
    }
    let len = len.checked_add(PAGE_SIZE - 1).ok_or(PagingError::AddressTooHigh)? & !(PAGE_SIZE - 1);

    let mut offset = 0u64;
    while offset < len {
        let v = va + offset;
        let p = pa + offset;
        let remaining = len - offset;

        // SAFETY: `root` and every table reached from it are pool tables.
        unsafe {
            let pdpt = next_table(root, ((v >> 39) & 0x1FF) as usize)?;
            let pd = next_table(pdpt, ((v >> 30) & 0x1FF) as usize)?;
            let pd_index = ((v >> 21) & 0x1FF) as usize;
            let pd_entry = (*table_ptr(pd)).0[pd_index];

            if pd_entry & PRESENT != 0 && pd_entry & HUGE != 0 {
                if pd_entry & HUGE_CACHE_MASK != kind.bits(true) {
                    return Err(PagingError::TypeConflict);
                }
                // Already covered by a 2 MiB page: skip to the next boundary.
                offset += core::cmp::min(HUGE_PAGE_SIZE - (v & (HUGE_PAGE_SIZE - 1)), remaining);
                continue;
            }

            let block_aligned = (v | p) & (HUGE_PAGE_SIZE - 1) == 0;
            if pd_entry & PRESENT == 0 && block_aligned && remaining >= HUGE_PAGE_SIZE {
                (*table_ptr(pd)).0[pd_index] = p | PRESENT | WRITABLE | HUGE | kind.bits(true);
                offset += HUGE_PAGE_SIZE;
                continue;
            }

            let pt = next_table(pd, pd_index)?;
            let slot = &mut (*table_ptr(pt)).0[((v >> 12) & 0x1FF) as usize];
            if *slot & PRESENT == 0 {
                *slot = p | PRESENT | WRITABLE | kind.bits(false);
            } else if *slot & PAGE_CACHE_MASK != kind.bits(false) {
                return Err(PagingError::TypeConflict);
            }
        }
        offset += PAGE_SIZE;
    }
    Ok(())
}

#[inline]
fn invlpg(address: u64) {
    // SAFETY: invalidating a TLB entry has no memory effects.
    unsafe { asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags)) };
}

/// Changes the memory type of the already-mapped, page-aligned range
/// `[start, start + len)`. Pages that already have the wanted type are not
/// touched.
unsafe fn remap_range(root: u64, start: u64, len: u64, kind: MemKind) -> Result<(), PagingError> {
    let end = start + len;
    let mut v = start;

    while v < end {
        // SAFETY: `root` and every table reached from it are pool tables.
        unsafe {
            let pml4e = (*table_ptr(root)).0[((v >> 39) & 0x1FF) as usize];
            if pml4e & PRESENT == 0 {
                return Err(PagingError::NotMapped);
            }
            let pdpte = (*table_ptr(pml4e & ADDRESS_MASK)).0[((v >> 30) & 0x1FF) as usize];
            if pdpte & PRESENT == 0 || pdpte & HUGE != 0 {
                return Err(PagingError::NotMapped);
            }
            let pd_slot = &mut (*table_ptr(pdpte & ADDRESS_MASK)).0[((v >> 21) & 0x1FF) as usize];
            if *pd_slot & PRESENT == 0 {
                return Err(PagingError::NotMapped);
            }

            if *pd_slot & HUGE != 0 {
                let entry = *pd_slot;
                let block = v & !(HUGE_PAGE_SIZE - 1);

                if entry & HUGE_CACHE_MASK == kind.bits(true) {
                    v = core::cmp::min(block + HUGE_PAGE_SIZE, end);
                    continue;
                }
                if v == block && end - v >= HUGE_PAGE_SIZE {
                    *pd_slot = (entry & !HUGE_CACHE_MASK) | kind.bits(true);
                    invlpg(v);
                    v += HUGE_PAGE_SIZE;
                    continue;
                }

                // Only part of the 2 MiB page changes: split it into 4 KiB pages.
                let pt = alloc_table()?;
                let base = entry & HUGE_ADDRESS_MASK;
                let mut attrs = entry & !(HUGE_ADDRESS_MASK | HUGE | PAT_2M);
                if entry & PAT_2M != 0 {
                    attrs |= PAT_4K;
                }
                let table = table_ptr(pt);
                for i in 0..ENTRIES_PER_TABLE {
                    (*table).0[i] = (base + i as u64 * PAGE_SIZE) | attrs;
                }
                *pd_slot = pt | PRESENT | WRITABLE;
                invlpg(block);
            }

            let pt = *pd_slot & ADDRESS_MASK;
            let slot = &mut (*table_ptr(pt)).0[((v >> 12) & 0x1FF) as usize];
            if *slot & PRESENT == 0 {
                return Err(PagingError::NotMapped);
            }
            if *slot & PAGE_CACHE_MASK != kind.bits(false) {
                *slot = (*slot & !PAGE_CACHE_MASK) | kind.bits(false);
                invlpg(v);
            }
        }
        v += PAGE_SIZE;
    }
    Ok(())
}

fn page_span(phys: u64, len: u64) -> Result<(u64, u64), PagingError> {
    let start = phys & !(PAGE_SIZE - 1);
    let end = phys.checked_add(len).ok_or(PagingError::AddressTooHigh)?;
    let end = end.checked_add(PAGE_SIZE - 1).ok_or(PagingError::AddressTooHigh)? & !(PAGE_SIZE - 1);
    if end > MAX_IDENTITY_ADDRESS {
        return Err(PagingError::AddressTooHigh);
    }
    Ok((start, end - start))
}

/// Identity-maps `len` bytes at physical address `phys` as `kind`, skipping
/// anything already mapped with that type.
fn map_identity(phys: u64, len: u64, kind: MemKind) -> Result<(), PagingError> {
    let root = root().ok_or(PagingError::NotInstalled)?;
    let (start, len) = page_span(phys, len)?;
    let _guard = MAP_LOCK.lock();
    // SAFETY: `root` is the live PML4. Only previously unmapped pages change,
    // so no TLB invalidation is required.
    unsafe { map_range(root, start, start, len, kind) }
}

/// Changes the memory type of an existing identity mapping.
fn remap_identity(phys: u64, len: u64, kind: MemKind) -> Result<(), PagingError> {
    let root = root().ok_or(PagingError::NotInstalled)?;
    let (start, len) = page_span(phys, len)?;
    let _guard = MAP_LOCK.lock();
    // SAFETY: `root` is the live PML4; changed entries are invalidated.
    unsafe { remap_range(root, start, len, kind) }
}

// ---------------------------------------------------------------------------
// Service-facing API (same names and signatures as the other architectures)
// ---------------------------------------------------------------------------

/// Whether the boot tables are installed, i.e. whether `map_*` can work.
pub fn is_ready() -> bool {
    root().is_some()
}

fn raw(range: PhysRange) -> (u64, u64) {
    (range.start().as_usize() as u64, range.len() as u64)
}

/// Identity-maps `range` as strongly uncacheable device memory.
pub fn map_device(range: PhysRange) -> Result<(), MmuError> {
    let (phys, len) = raw(range);
    map_identity(phys, len, MemKind::Device).map_err(Into::into)
}

/// Identity-maps `range` as ordinary write-back memory.
pub fn map_normal(range: PhysRange) -> Result<(), MmuError> {
    let (phys, len) = raw(range);
    map_identity(phys, len, MemKind::Normal).map_err(Into::into)
}

/// Identity-maps `range` as non-cacheable memory (write-combining once
/// [`init`] has programmed the PAT, uncached before that).
pub fn map_normal_nc(range: PhysRange) -> Result<(), MmuError> {
    let (phys, len) = raw(range);
    map_identity(phys, len, MemKind::NormalNc).map_err(Into::into)
}

/// Changes an already mapped range to ordinary write-back memory.
pub fn remap_normal(range: PhysRange) -> Result<(), MmuError> {
    let (phys, len) = raw(range);
    remap_identity(phys, len, MemKind::Normal).map_err(Into::into)
}

/// Changes an already mapped range to non-cacheable memory.
pub fn remap_normal_nc(range: PhysRange) -> Result<(), MmuError> {
    let (phys, len) = raw(range);
    remap_identity(phys, len, MemKind::NormalNc).map_err(Into::into)
}

/// Physical range of the kernel image (empty before the boot tables exist).
pub fn kernel_image_range() -> PhysRange {
    super::handoff::paging::image_range().unwrap_or(PhysRange::EMPTY)
}

// ---------------------------------------------------------------------------
// CPU setup
// ---------------------------------------------------------------------------

unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags)) };
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32,
        options(nomem, nostack, preserves_flags))
    };
}

/// CPU-side setup on top of the boot tables: programs the PAT so
/// `NormalNc` mappings become write-combining.
pub fn init() -> Result<(), MmuError> {
    if root().is_none() {
        return Err(MmuError::MmuNotReady);
    }
    if READY.swap(true, Ordering::AcqRel) {
        return Err(MmuError::AlreadyInitialized);
    }

    // SAFETY: CPUID is always available in long mode.
    let has_pat = unsafe { core::arch::x86_64::__cpuid(1) }.edx & CPUID_1_EDX_PAT != 0;
    if has_pat {
        // SAFETY: PAT is supported; entry 4 is unused by any existing mapping
        // (all of them use entries 0 and 3).
        unsafe {
            let pat = rdmsr(IA32_PAT);
            let pat = (pat & !(0xFFu64 << PAT_ENTRY4_SHIFT)) | (PAT_TYPE_WC << PAT_ENTRY4_SHIFT);
            wrmsr(IA32_PAT, pat);
            // Flush the TLB so no stale entry keeps the old interpretation.
            let cr3: u64;
            asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
            asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags));
        }
        PAT_WC.store(true, Ordering::Release);
    }
    Ok(())
}
