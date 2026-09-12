//! Physical memory manager
//!
//! this is currently a list allocator, it hands out ram pages
//! to the kernel and drivers



use crate::services::mmu::{self, MmuError};
use crate::services::sync::SpinLock;

/// Frame size in bytes (4KB).
pub const FRAME_SIZE: usize = 4096;
const FRAME_SHIFT: usize = 12;

/// Highest supported buddy order.
pub const MAX_ORDER: usize = 10;

/// Maximum number of caller-supplied [`ReservedRange`]s [`init`] accepts
/// no heap for a vec.
pub const MAX_RESERVED_RANGES: usize = 15;

/// A physical frame address. must be [`FRAME_SIZE`]-aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysFrame(usize);

impl PhysFrame {
    #[inline]
    pub const fn from_addr(addr: usize) -> Self {
        debug_assert!(addr % FRAME_SIZE == 0);
        Self(addr)
    }

    #[inline]
    pub const fn addr(self) -> usize {
        self.0
    }

    #[inline]
    const fn index(self) -> usize {
        self.0 >> FRAME_SHIFT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmmError {
    /// [`init`] was called more than once.
    AlreadyInitialized,
    /// A PMM function other than [`init`] was called before [`init`].
    NotReady,
    /// The reserved region computed for kernel/boot/metadata did not fit
    /// within the RAM range reported by the DTB.
    ReservedRegionOutOfRange,
    /// Mapping the usable RAM range as identity-mapped normal memory failed.
    MappingFailed(MmuError),
    /// `order` exceeded [`MAX_ORDER`].
    OrderTooLarge,
    /// More than [`MAX_RESERVED_RANGES`] ranges were passed to [`init`].
    TooManyReservedRanges,
    /// A frame passed to [`free_frames`] was not aligned to its claimed
    /// order, or was already free (double-free).
    InvalidFree,
}

/// Per-frame bookkeeping
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameState {
    Allocated,
    Free(u8),
    FreeTail,
}

struct Pmm {
    /// One entry per frame in the managed range, indexed by
    /// `frame.index() - base_index`.
    frame_state: &'static mut [FrameState],
    /// Physical address corresponding to `frame_state[0]`.
    base_addr: usize,
    /// Physical frame index corresponding to `frame_state[0]`.
    base_index: usize,
    /// Head of each order's free list.
    free_lists: [Option<PhysFrame>; MAX_ORDER + 1],
    frames_free: usize,
    frames_total: usize,
}

impl Pmm {
    /// Physical address of the free-list link word stored in `frame`'s
    /// first `size_of::<Option<PhysFrame>>()` bytes.
    ///
    /// # Safety
    /// `frame` must currently be identity-mapped and not concurrently
    /// aliased (i.e. the caller holds the PMM lock and `frame` is not
    /// handed out to anyone else).
    unsafe fn node_ptr(frame: PhysFrame) -> *mut Option<PhysFrame> {
        frame.addr() as *mut Option<PhysFrame>
    }

    fn state_mut(&mut self, frame: PhysFrame) -> &mut FrameState {
        &mut self.frame_state[frame.index() - self.base_index]
    }

    fn state(&self, frame: PhysFrame) -> FrameState {
        self.frame_state[frame.index() - self.base_index]
    }

    fn in_range(&self, frame: PhysFrame) -> bool {
        let idx = frame.index();
        idx >= self.base_index && idx < self.base_index + self.frame_state.len()
    }

    fn push_free(&mut self, order: usize, frame: PhysFrame) {
        // SAFETY: frame is RAM under our management, and we
        // hold `&mut self` (i.e. the PMM lock), so nothing else can change it
        unsafe { core::ptr::write(Self::node_ptr(frame), self.free_lists[order]) };
        self.free_lists[order] = Some(frame);
        *self.state_mut(frame) = FrameState::Free(order as u8);
        for i in 1..(1usize << order) {
            let tail = PhysFrame::from_addr(frame.addr() + i * FRAME_SIZE);
            *self.state_mut(tail) = FrameState::FreeTail;
        }
    }

    /// Removes and returns the head of `free_lists[order]`, if any.
    fn pop_free(&mut self, order: usize) -> Option<PhysFrame> {
        let head = self.free_lists[order]?;
        // SAFETY: head is a frame we ourselves linked in `push_free`.
        let next = unsafe { core::ptr::read(Self::node_ptr(head)) };
        self.free_lists[order] = next;
        Some(head)
    }

    /// Removes a specific frame from `free_lists[order]`.
    fn remove_free(&mut self, order: usize, target: PhysFrame) -> bool {
        let mut cursor = self.free_lists[order];
        let mut prev: Option<PhysFrame> = None;

        while let Some(cur) = cursor {
            // SAFETY: cur is a frame we linked ourselves.
            let next = unsafe { core::ptr::read(Self::node_ptr(cur)) };
            if cur == target {
                match prev {
                    // SAFETY: prev is a live free-list node we linked.
                    Some(p) => unsafe { core::ptr::write(Self::node_ptr(p), next) },
                    None => self.free_lists[order] = next,
                }
                return true;
            }
            prev = Some(cur);
            cursor = next;
        }
        false
    }

    /// Returns `frame`'s buddy at `order`
    fn buddy_of(&self, frame: PhysFrame, order: usize) -> PhysFrame {
        let block_bytes = FRAME_SIZE << order;
        let offset = frame.addr() - self.base_addr;
        let buddy_offset = offset ^ block_bytes;
        PhysFrame::from_addr(self.base_addr + buddy_offset)
    }

    fn alloc(&mut self, order: usize) -> Option<PhysFrame> {
        // Find the smallest available order >= order with a free block.
        let mut found_order = None;
        for o in order..=MAX_ORDER {
            if self.free_lists[o].is_some() {
                found_order = Some(o);
                break;
            }
        }
        let found_order = found_order?;
        let block = self.pop_free(found_order)?;

        // Split the block down to the requested order
        let mut cur_order = found_order;
        let mut cur_frame = block;
        while cur_order > order {
            cur_order -= 1;
            let half_bytes = FRAME_SIZE << cur_order;
            let buddy = PhysFrame::from_addr(cur_frame.addr() + half_bytes);
            self.push_free(cur_order, buddy);
        }

        *self.state_mut(cur_frame) = FrameState::Allocated;
        for i in 1..(1usize << order) {
            let tail = PhysFrame::from_addr(cur_frame.addr() + i * FRAME_SIZE);
            *self.state_mut(tail) = FrameState::Allocated;
        }

        self.frames_free -= 1usize << order;
        Some(cur_frame)
    }

    fn free(&mut self, frame: PhysFrame, order: usize) -> Result<(), PmmError> {
        if order > MAX_ORDER {
            return Err(PmmError::OrderTooLarge);
        }
        let offset = frame.addr().wrapping_sub(self.base_addr);
        if !self.in_range(frame) || offset % (FRAME_SIZE << order) != 0 {
            return Err(PmmError::InvalidFree);
        }
        if matches!(self.state(frame), FrameState::Free(_) | FrameState::FreeTail) {
            return Err(PmmError::InvalidFree);
        }

        let mut cur_order = order;
        let mut cur_frame = frame;

        // Coalesce with the buddy while it is free, itself at the same
        // order, and merging still fits within the managed range.
        while cur_order < MAX_ORDER {
            let buddy = self.buddy_of(cur_frame, cur_order);
            if !self.in_range(buddy) {
                break;
            }
            let buddy_is_free_head = matches!(
                self.state(buddy),
                FrameState::Free(o) if o as usize == cur_order
            );
            if !buddy_is_free_head {
                break;
            }

            self.remove_free(cur_order, buddy);
            // The merged block's base is whichever of the two came first.
            cur_frame = PhysFrame::from_addr(cur_frame.addr().min(buddy.addr()));
            cur_order += 1;
        }

        self.frames_free += 1usize << order;
        self.push_free(cur_order, cur_frame);
        Ok(())
    }
}


unsafe impl Send for Pmm {}

static PMM: SpinLock<Option<Pmm>> = SpinLock::new(None);

/// Ram excluded from the free-list pool
#[derive(Clone, Copy)]
pub struct ReservedRange {
    pub start: usize,
    pub end: usize,
}

/// Initializes the physical memory manager.
pub fn init(ram_base: usize, ram_size: usize, reserved: &[ReservedRange]) -> Result<(), PmmError> {
    let mut guard = PMM.lock();
    if guard.is_some() {
        return Err(PmmError::AlreadyInitialized);
    }
    if reserved.len() > MAX_RESERVED_RANGES {
        return Err(PmmError::TooManyReservedRanges);
    }

    let ram_base = align_down(ram_base, FRAME_SIZE);
    let ram_end = align_up(ram_base + ram_size, FRAME_SIZE);

    mmu::map_normal(ram_base, ram_end - ram_base).map_err(PmmError::MappingFailed)?;

    // Metadata: one FrameState byte per frame in the managed range
    let frame_count = (ram_end - ram_base) / FRAME_SIZE;
    let highest_reserved_end = reserved.iter().map(|r| r.end).max().unwrap_or(ram_base);
    let meta_start = align_up(highest_reserved_end.max(ram_base), 8);
    let meta_bytes = frame_count; // 1 byte per FrameState
    let meta_end = align_up(meta_start + meta_bytes, FRAME_SIZE);

    if meta_end > ram_end {
        return Err(PmmError::ReservedRegionOutOfRange);
    }

    // SAFETY: [meta_start, meta_end) is within the freshly identity-mapped
    // RAM range, is above every caller-supplied reserved range, and is not
    // otherwise referenced by anyone at this point in boot.
    let frame_state: &'static mut [FrameState] = unsafe {
        let ptr = meta_start as *mut FrameState;
        core::ptr::write_bytes(ptr, 0, frame_count);
        core::slice::from_raw_parts_mut(ptr, frame_count)
    };
    for s in frame_state.iter_mut() {
        *s = FrameState::Allocated;
    }

    let mut pmm = Pmm {
        frame_state,
        base_addr: ram_base,
        base_index: ram_base >> FRAME_SHIFT,
        free_lists: [None; MAX_ORDER + 1],
        frames_free: 0,
        frames_total: frame_count,
    };

    // Build the free lists
    let mut all_reserved: [ReservedRange; MAX_RESERVED_RANGES + 1] =
        [ReservedRange { start: 0, end: 0 }; MAX_RESERVED_RANGES + 1];
    let reserved_count = reserved.len();
    all_reserved[..reserved_count].copy_from_slice(reserved);
    all_reserved[reserved_count] = ReservedRange {
        start: meta_start,
        end: meta_end,
    };
    let all_reserved = &all_reserved[..=reserved_count];

    free_unreserved_ranges(&mut pmm, ram_base, ram_end, all_reserved);

    *guard = Some(pmm);
    Ok(())
}

/// Walks `[ram_base, ram_end)`, inserting every maximal aligned
/// power-of-two-sized free run into the buddy free lists, skipping bytes
/// covered by any range in `reserved`.
fn free_unreserved_ranges(pmm: &mut Pmm, ram_base: usize, ram_end: usize, reserved: &[ReservedRange]) {
    let mut cursor = ram_base;
    while cursor < ram_end {
        // Find how far the current free run extends before hitting a
        // reserved range (or the end of RAM).
        let mut run_end = ram_end;
        let mut in_reserved = false;
        for r in reserved {
            if cursor >= r.start && cursor < r.end {
                in_reserved = true;
                run_end = r.end;
                break;
            }
            if r.start > cursor && r.start < run_end {
                run_end = r.start;
            }
        }

        if in_reserved {
            cursor = run_end;
            continue;
        }

        // Insert the largest aligned power-of-two block(s) covering
        // [cursor, run_end), largest-order-first, buddy style. Alignment is
        // measured relative to `ram_base` (== `pmm.base_addr`), matching
        // the offset-based buddy math in `Pmm::buddy_of` -- a block's raw
        // address need not itself be power-of-two aligned, only its offset
        // from the region base.
        let mut addr = cursor;
        while addr < run_end {
            let remaining = run_end - addr;
            let rel_offset = addr - ram_base;
            let align_order = if rel_offset == 0 {
                MAX_ORDER
            } else {
                (rel_offset.trailing_zeros().saturating_sub(FRAME_SHIFT as u32)) as usize
            };
            let size_order = usize::BITS as usize - 1 - remaining.leading_zeros() as usize;
            let size_order = size_order.saturating_sub(FRAME_SHIFT);
            let order = align_order.min(size_order).min(MAX_ORDER);
            let block_bytes = FRAME_SIZE << order;

            pmm.push_free(order, PhysFrame::from_addr(addr));
            pmm.frames_free += 1usize << order;
            addr += block_bytes;
        }

        cursor = run_end;
    }
}

/// Allocates `2^order` contiguous, [`FRAME_SIZE`]-aligned frames.
///
/// Returns the base [`PhysFrame`] of the block, or `None` if no block of
/// that order is currently available (RAM is fragmented or exhausted at
/// that size).
///
/// # Errors
/// Returns [`PmmError::NotReady`] if called before [`init`], or
/// [`PmmError::OrderTooLarge`] if `order > `[`MAX_ORDER`].
pub fn alloc_frames(order: usize) -> Result<Option<PhysFrame>, PmmError> {
    if order > MAX_ORDER {
        return Err(PmmError::OrderTooLarge);
    }
    let mut guard = PMM.lock();
    let pmm = guard.as_mut().ok_or(PmmError::NotReady)?;
    Ok(pmm.alloc(order))
}

/// Allocates a single [`FRAME_SIZE`] frame, variation of
/// [`pmm::alloc_frames()`]
pub fn alloc_frame() -> Result<Option<PhysFrame>, PmmError> {
    alloc_frames(0)
}

/// Frees a block of `2^order` frames previously returned by
/// [`alloc_frames`] with the same `order`.
///
/// Coalesces with the buddy block(s) where possible, up to [`MAX_ORDER`].
///
/// # Errors
/// Returns [`PmmError::NotReady`] before [`init`], [`PmmError::OrderTooLarge`]
/// if `order > `[`MAX_ORDER`], or [`PmmError::InvalidFree`] if `frame` is
/// misaligned for `order`, out of the managed range, or already free
/// (double-free).
pub fn free_frames(frame: PhysFrame, order: usize) -> Result<(), PmmError> {
    let mut guard = PMM.lock();
    let pmm = guard.as_mut().ok_or(PmmError::NotReady)?;
    pmm.free(frame, order)
}

/// Frees a single frame, variation of [`pmm::free_frames()`]
pub fn free_frame(frame: PhysFrame) -> Result<(), PmmError> {
    free_frames(frame, 0)
}

/// Returns `(frames_free, frames_total)` for diagnostics/telemetry.
pub fn stats() -> Result<(usize, usize), PmmError> {
    let guard = PMM.lock();
    let pmm = guard.as_ref().ok_or(PmmError::NotReady)?;
    Ok((pmm.frames_free, pmm.frames_total))
}

#[inline]
const fn align_down(addr: usize, align: usize) -> usize {
    addr & !(align - 1)
}

#[inline]
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}