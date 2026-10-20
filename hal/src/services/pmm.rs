//! Physical memory manager

use crate::services::addr::{FRAME_SHIFT, PhysAddr, PhysRange};
pub use crate::services::addr::{FRAME_SIZE, PhysFrame};
use crate::services::mmu::{self, MmuError};
use crate::services::sync::SpinLock;

/// Highest supported buddy order.
pub const MAX_ORDER: usize = 10;

/// Maximum number of caller-supplied reserved ranges [`init`] accepts
pub const MAX_RESERVED_RANGES: usize = 15;

/// The frame starting at the raw address `addr`.
#[inline]
fn frame_at(addr: usize) -> PhysFrame {
    PhysFrame::from_addr(PhysAddr::new(addr))
}

/// The raw start address of `frame`.
#[inline]
fn addr_of(frame: PhysFrame) -> usize {
    frame.addr().as_usize()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmmError {
    /// [`init`] was called more than once.
    AlreadyInitialized,
    /// A PMM function other than [`init`] was called before [`init`].
    NotReady,
    /// No usable region had room for the PMM's metadata after excluding the
    /// reserved ranges.
    NoRoomForMetadata,
    /// Every frame is in use (or no free block of the requested order exists).
    OutOfMemory,
    /// [`init`] was given no usable memory at all.
    NoMemory,
    /// Mapping the usable RAM range as identity-mapped normal memory failed.
    MappingFailed(MmuError),
    /// `order` exceeded [`MAX_ORDER`].
    OrderTooLarge,
    /// More than [`MAX_RESERVED_RANGES`] ranges were passed to [`init`].
    TooManyReserveds,
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
    frame_state: &'static mut [FrameState],
    base_addr: usize,
    base_index: usize,
    free_lists: [Option<PhysFrame>; MAX_ORDER + 1],
    frames_free: usize,
    frames_total: usize,
}

impl Pmm {
    /// Physical address of the free-list link word stored in `frame`'s
    /// first `size_of::<Option<PhysFrame>>()` bytes.
    unsafe fn node_ptr(frame: PhysFrame) -> *mut Option<PhysFrame> {
        addr_of(frame) as *mut Option<PhysFrame>
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
        // SAFETY: frame is identity-mapped RAM under our management, and we
        // hold `&mut self` (i.e. the PMM lock), so no one else touches it.
        unsafe { core::ptr::write(Self::node_ptr(frame), self.free_lists[order]) };
        self.free_lists[order] = Some(frame);
        *self.state_mut(frame) = FrameState::Free(order as u8);
        for i in 1..(1usize << order) {
            let tail = frame_at(addr_of(frame) + i * FRAME_SIZE);
            *self.state_mut(tail) = FrameState::FreeTail;
        }
    }

    /// Removes and returns the head of `free_lists[order]`
    fn pop_free(&mut self, order: usize) -> Option<PhysFrame> {
        let head = self.free_lists[order]?;
        // SAFETY: head is a frame we ourselves linked in `push_free`.
        let next = unsafe { core::ptr::read(Self::node_ptr(head)) };
        self.free_lists[order] = next;
        Some(head)
    }

    /// Removes a specific frame from `free_lists[order]`
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


    fn buddy_of(&self, frame: PhysFrame, order: usize) -> PhysFrame {
        let block_bytes = FRAME_SIZE << order;
        let offset = addr_of(frame) - self.base_addr;
        let buddy_offset = offset ^ block_bytes;
        frame_at(self.base_addr + buddy_offset)
    }

    fn alloc(&mut self, order: usize) -> Option<PhysFrame> {
        // Find the smallest available order >= `order` with a free block.
        let mut found_order = None;
        for o in order..=MAX_ORDER {
            if self.free_lists[o].is_some() {
                found_order = Some(o);
                break;
            }
        }
        let found_order = found_order?;
        let block = self.pop_free(found_order)?;

        // Split the block down to the requested order, pushing the unused
        // upper halves back onto their respective free lists.
        let mut cur_order = found_order;
        let cur_frame = block;
        while cur_order > order {
            cur_order -= 1;
            let half_bytes = FRAME_SIZE << cur_order;
            let buddy = frame_at(addr_of(cur_frame) + half_bytes);
            self.push_free(cur_order, buddy);
        }

        *self.state_mut(cur_frame) = FrameState::Allocated;
        for i in 1..(1usize << order) {
            let tail = frame_at(addr_of(cur_frame) + i * FRAME_SIZE);
            *self.state_mut(tail) = FrameState::Allocated;
        }

        self.frames_free -= 1usize << order;
        Some(cur_frame)
    }

    /// Checks that `frame`/`order` is eligible to be freed
    fn validate_free(&self, frame: PhysFrame, order: usize) -> Result<(), PmmError> {
        if order > MAX_ORDER {
            return Err(PmmError::OrderTooLarge);
        }
        let offset = addr_of(frame).wrapping_sub(self.base_addr);
        if !self.in_range(frame) || !offset.is_multiple_of(FRAME_SIZE << order)  {
            return Err(PmmError::InvalidFree);
        }
        if matches!(self.state(frame), FrameState::Free(_) | FrameState::FreeTail) {
            return Err(PmmError::InvalidFree);
        }
        Ok(())
    }

    fn free(&mut self, frame: PhysFrame, order: usize) -> Result<(), PmmError> {
        self.validate_free(frame, order)?;

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
            cur_frame = frame_at(addr_of(cur_frame).min(addr_of(buddy)));
            cur_order += 1;
        }

        self.frames_free += 1usize << order;
        self.push_free(cur_order, cur_frame);
        Ok(())
    }
}

// SAFETY: Pmm's only non-Sync-by-default field is the frame_state slice,
// which is exclusively accessed through the SpinLock below.
unsafe impl Send for Pmm {}

static PMM: SpinLock<Option<Pmm>> = SpinLock::new(None);

/// A frame-aligned `[start, end)` byte range excluded from the free pool --
/// the kernel image, boot/device page tables, the firmware's tables, or the
/// PMM's own metadata array.
#[derive(Clone, Copy)]
struct Reserved {
    start: usize,
    end: usize,
}

impl Reserved {
    /// `range` rounded outwards to whole frames.
    fn covering(range: PhysRange) -> Self {
        let range = range.round_out(FRAME_SIZE);
        Self { start: range.start().as_usize(), end: range.end().as_usize() }
    }
}

/// Returns the first frame-aligned address at which `len` bytes fit inside one
/// of `regions` without touching any of `reserved`.
fn find_metadata_area(regions: &[PhysRange], reserved: &[Reserved], len: usize) -> Option<usize> {
    for region in regions {
        let start = align_up(region.start().as_usize(), FRAME_SIZE);
        let end = align_down(region.end().as_usize(), FRAME_SIZE);

        let mut candidate = start;
        'search: while candidate + len <= end {
            for r in reserved {
                if candidate < r.end && r.start < candidate + len {
                    candidate = align_up(r.end, FRAME_SIZE);
                    continue 'search;
                }
            }
            return Some(candidate);
        }
    }
    None
}

/// Initializes the physical memory manager over the usable RAM `regions`
/// (which may be several, with holes between them), excluding `reserved`.
pub fn init(regions: &[PhysRange], reserved: &[PhysRange]) -> Result<(), PmmError> {
    let mut guard = PMM.lock();
    if guard.is_some() {
        return Err(PmmError::AlreadyInitialized);
    }
    if reserved.len() > MAX_RESERVED_RANGES {
        return Err(PmmError::TooManyReserveds);
    }

    // Frame-aligned view of the usable regions and the span they cover.
    let mut ram_base = usize::MAX;
    let mut ram_end = 0usize;
    let mut frames_total = 0usize;
    for region in regions {
        let start = align_up(region.start().as_usize(), FRAME_SIZE);
        let end = align_down(region.end().as_usize(), FRAME_SIZE);
        if start >= end {
            continue;
        }
        mmu::map_normal(PhysRange::from_bounds(PhysAddr::new(start), PhysAddr::new(end))).map_err(PmmError::MappingFailed)?;
        ram_base = ram_base.min(start);
        ram_end = ram_end.max(end);
        frames_total += (end - start) / FRAME_SIZE;
    }
    if frames_total == 0 {
        return Err(PmmError::NoMemory);
    }

    // Reserved ranges are rounded outwards to whole frames.
    let mut all_reserved: [Reserved; MAX_RESERVED_RANGES + 1] =
        [Reserved { start: 0, end: 0 }; MAX_RESERVED_RANGES + 1];
    for (slot, r) in all_reserved.iter_mut().zip(reserved) {
        *slot = Reserved::covering(*r);
    }
    let reserved_count = reserved.len();

    // One state entry per frame of the whole span (holes included).
    let span_frames = (ram_end - ram_base) / FRAME_SIZE;
    let meta_len = align_up(span_frames * core::mem::size_of::<FrameState>(), FRAME_SIZE);
    let meta_start = find_metadata_area(regions, &all_reserved[..reserved_count], meta_len)
        .ok_or(PmmError::NoRoomForMetadata)?;
    all_reserved[reserved_count] = Reserved { start: meta_start, end: meta_start + meta_len };
    let all_reserved = &all_reserved[..=reserved_count];

    // SAFETY: [meta_start, meta_start + meta_len) lies inside a region that was
    // just identity-mapped, clear of every reserved range, and nobody else
    // references it at this point in boot.
    let frame_state: &'static mut [FrameState] = unsafe {
        let ptr = meta_start as *mut FrameState;
        for i in 0..span_frames {
            ptr.add(i).write(FrameState::Allocated);
        }
        core::slice::from_raw_parts_mut(ptr, span_frames)
    };

    let mut pmm = Pmm {
        frame_state,
        base_addr: ram_base,
        base_index: ram_base >> FRAME_SHIFT,
        free_lists: [None; MAX_ORDER + 1],
        frames_free: 0,
        frames_total,
    };

    for region in regions {
        let start = align_up(region.start().as_usize(), FRAME_SIZE);
        let end = align_down(region.end().as_usize(), FRAME_SIZE);
        if start < end {
            free_unreserved_ranges(&mut pmm, start, end, all_reserved);
        }
    }

    *guard = Some(pmm);
    Ok(())
}

/// Walks `[start, end)`, inserting every maximal aligned power-of-two-sized
/// free run into the buddy free lists, skipping bytes covered by any range in
/// `reserved`. Block alignment is relative to the PMM's base address.
fn free_unreserved_ranges(pmm: &mut Pmm, start: usize, end: usize, reserved: &[Reserved]) {
    let ram_base = pmm.base_addr;
    let mut cursor = start;
    while cursor < end {
        // Find how far the current free run extends before hitting a
        // reserved range (or the end of the region).
        let mut run_end = end;
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

            pmm.push_free(order, frame_at(addr));
            pmm.frames_free += 1usize << order;
            addr += block_bytes;
        }

        cursor = run_end;
    }
}

/// Allocates `2^order` contiguous, [`FRAME_SIZE`]-aligned frames.
pub fn alloc_frames(order: usize) -> Result<PhysFrame, PmmError> {
    if order > MAX_ORDER {
        return Err(PmmError::OrderTooLarge);
    }
    let mut guard = PMM.lock();
    let pmm = guard.as_mut().ok_or(PmmError::NotReady)?;
    pmm.alloc(order).ok_or(PmmError::OutOfMemory)
}

/// Allocates a single [`FRAME_SIZE`] frame. Convenience for `alloc_frames(0)`.
pub fn alloc_frame() -> Result<PhysFrame, PmmError> {
    alloc_frames(0)
}

/// Frees a block of `2^order` frames previously returned by
/// [`alloc_frames`] with the same `order`.
pub fn free_frames(frame: PhysFrame, order: usize) -> Result<(), PmmError> {
    // One lock for the whole operation, so the frame cannot be freed or
    // reallocated by someone else between validating it and releasing it.
    // (The MMU's own lock is separate and never calls back into the PMM.)
    let mut guard = PMM.lock();
    let pmm = guard.as_mut().ok_or(PmmError::NotReady)?;
    pmm.validate_free(frame, order)?;

    // A caller may have remapped the block (e.g. non-cacheable for DMA); hand
    // it back as ordinary memory.
    let block = PhysRange::new(frame.addr(), FRAME_SIZE << order);
    mmu::remap_normal(block).map_err(PmmError::MappingFailed)?;

    pmm.free(frame, order)
}

/// Frees a single frame previously returned by [`alloc_frame`].
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