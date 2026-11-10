//! General Kernel Heap
//!
//! Current Problems: Interrupts not masked while lock held. allocating in
//! interrupt handlers will deadlock

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::services::addr::FRAME_SIZE;
use crate::services::pmm::{self, MAX_ORDER};
use crate::services::sync::IrqSpinLock;

/// Block sizes served by small tier
const CLASS_SIZES: [usize; 13] = [16, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 2048];
const CLASS_COUNT: usize = CLASS_SIZES.len();

/// The largest request small tier serves.
pub const MAX_SMALL: usize = CLASS_SIZES[CLASS_COUNT - 1];

/// Byte written over small freed blocks, makes use-after-free cases obvious
const POISON: u8 = 0xDD;

/// Which tier a request is served
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    /// Index into [`CLASS_SIZES`].
    Small(usize),
    /// This many whole frames, taken from a buddy block that is rounded up to a
    /// power of two.
    Pages(usize),
}

impl Route {
    /// Chooses tier for `layout`
    fn of(layout: Layout) -> Option<Route> {
        // Zero size requests not allowed by global alloc
        let size = layout.size().max(1);
        let align = layout.align();

        #[allow(clippy::collapsible_if)] // false positive
        if size <= MAX_SMALL {
            if let Some(index) = CLASS_SIZES.iter().position(|&s| s >= size && s % align == 0) {
                return Some(Route::Small(index));
            }
        }

        if align > FRAME_SIZE {
            return None;
        }

        let frames = size.div_ceil(FRAME_SIZE);
        (order_for(frames) <= MAX_ORDER).then_some(Route::Pages(frames))
    }
}

/// Free-List link stored in free small block
struct FreeNode {
    next: Option<NonNull<FreeNode>>,
}

/// Buddy order of the smallest block holding `frames` frames.
fn order_for(frames: usize) -> usize {
    frames.next_power_of_two().trailing_zeros() as usize
}

/// Returns frames `start..end` of the allocation at `base` to the PMM
fn free_frame_range(base: usize, start: usize, end: usize) {
    let mut at = start;
    while at < end {
        // Biggest power of two that fits
        let mut piece = 1usize << (usize::BITS - 1 - (end - at).leading_zeros());
        if at > 0 {
            piece = piece.min(1 << at.trailing_zeros());
        }
        let frame = pmm::PhysFrame::from_addr(crate::services::addr::PhysAddr::new(base + at * FRAME_SIZE));
        // Failure here means the heap's own bookkeeping is corrupt.
        let result = pmm::free_frames(frame, piece.trailing_zeros() as usize);
        debug_assert!(result.is_ok(), "heap: PMM rejected a free: {result:?}");
        at += piece;
    }
}

/// Address of frame containing `node`
fn frame_base(node: NonNull<FreeNode>) -> usize {
    node.as_ptr() as usize & !(FRAME_SIZE - 1)
}

/// # Safety
/// `node` must be a live free-list node.
unsafe fn next_of(node: NonNull<FreeNode>) -> Option<NonNull<FreeNode>> {
    unsafe { (*node.as_ptr()).next }
}

/// # Safety
/// `node` must be a live free-list node.
unsafe fn set_next(node: NonNull<FreeNode>, next: Option<NonNull<FreeNode>>) {
    unsafe { (*node.as_ptr()).next = next }
}

/// Merges two address-sorted lists into one.
///
/// # Safety
/// Both lists must consist of live free-list nodes and must not share nodes.
unsafe fn merge(mut a: Option<NonNull<FreeNode>>, mut b: Option<NonNull<FreeNode>>) -> Option<NonNull<FreeNode>> {
    let mut head = None;
    let mut tail: Option<NonNull<FreeNode>> = None;
    loop {
        // SAFETY: every node reached here is a live node of one of the lists.
        let pick = unsafe {
            match (a, b) {
                (None, None) => break,
                (Some(x), None) => { a = next_of(x); x }
                (None, Some(y)) => { b = next_of(y); y }
                (Some(x), Some(y)) if x.as_ptr() < y.as_ptr() => { a = next_of(x); x }
                (Some(_), Some(y)) => { b = next_of(y); y }
            }
        };
        match tail {
            // SAFETY: `t` is a node we already picked.
            Some(t) => unsafe { set_next(t, Some(pick)) },
            None => head = Some(pick),
        }
        tail = Some(pick);
    }
    if let Some(t) = tail {
        // SAFETY: `t` is the last picked node.
        unsafe { set_next(t, None) };
    }
    head
}

/// Sorts a free list by ascending address.
///
/// # Safety
/// The list must consist of live free-list nodes.
unsafe fn sort_by_address(head: Option<NonNull<FreeNode>>) -> Option<NonNull<FreeNode>> {
    unsafe {
        let first = head?;
        if next_of(first).is_none() {
            return head;
        }

        // Split after the middle node.
        let mut slow = first;
        let mut fast = next_of(first);
        while let Some(f) = fast {
            fast = next_of(f);
            if let Some(f2) = fast {
                slow = next_of(slow).expect("slow trails fast");
                fast = next_of(f2);
            }
        }
        let second = next_of(slow);
        set_next(slow, None);

        merge(sort_by_address(Some(first)), sort_by_address(second))
    }
}

/// Takes a `2^order` frame block from the PMM and returns it as a pointer, or
/// null when the PMM has nothing suitable.
fn alloc_pages(order: usize) -> *mut u8 {
    for _ in 0..2 {
        let Ok(frame) = pmm::alloc_frames(order) else {
            return ptr::null_mut();
        };

        let addr = frame.addr().as_usize();

        if addr != 0 {
            return addr as *mut u8;
        }
    }
    ptr::null_mut()
}

/// State of the small tier, protected by the heap lock.
struct SmallHeap {
    free: [Option<NonNull<FreeNode>>; CLASS_COUNT],
    /// Frames taken from the PMM for the small tier.
    frames: usize,
    /// Bytes currently handed out, counted in whole blocks.
    live_bytes: usize,
}

unsafe impl Send for SmallHeap {}

impl SmallHeap {
    const fn new() -> Self {
        Self {
            free: [None; CLASS_COUNT],
            frames: 0,
            live_bytes: 0,
        }
    }

    /// Carves a fresh frame into blocks of class `index` and links them in.
    /// Returns `false` when the PMM is out of frames.
    fn refill(&mut self, index: usize) -> bool {
        let page = alloc_pages(0);
        if page.is_null() {
            return false;
        }
        self.frames += 1;

        let size = CLASS_SIZES[index];
        // Link back to front so the list hands blocks out in address order.
        let mut head = self.free[index];
        for i in (0..FRAME_SIZE / size).rev() {
            unsafe {
                let block = page.add(i * size).cast::<FreeNode>();
                block.write(FreeNode { next: head });
                head = NonNull::new(block);
            }
        }
        self.free[index] = head;
        true
    }

    /// Returns every completely free frame to the PMM and reports how many.
    fn trim(&mut self) -> usize {
        let mut released = 0;
        for index in 0..CLASS_COUNT {
            let per_frame = FRAME_SIZE / CLASS_SIZES[index];
            // With the list sorted, all free blocks of one frame are adjacent.
            // SAFETY: the list holds live nodes
            let mut cursor = unsafe { sort_by_address(self.free[index].take()) };
            let mut kept_head: Option<NonNull<FreeNode>> = None;
            let mut kept_tail: Option<NonNull<FreeNode>> = None;

            while let Some(first) = cursor {
                // Collect the run of blocks that share `first`'s frame.
                let base = frame_base(first);
                let mut last = first;
                let mut count = 1;
                loop {
                    // SAFETY: `last` is a live node of the sorted list.
                    let next = unsafe { next_of(last) };
                    match next {
                        Some(n) if frame_base(n) == base => {
                            last = n;
                            count += 1;
                        }
                        _ => {
                            cursor = next;
                            break;
                        }
                    }
                }

                if count == per_frame {
                    let frame = pmm::PhysFrame::from_addr(crate::services::addr::PhysAddr::new(base));
                    if pmm::free_frames(frame, 0).is_ok() {
                        self.frames -= 1;
                        released += 1;
                        continue;
                    }
                    // PMM failed; keep the blocks
                }

                // SAFETY: `first..=last` is a chain of live nodes, `tail` is the
                // last node kept so far.
                unsafe {
                    set_next(last, None);
                    match kept_tail {
                        Some(t) => set_next(t, Some(first)),
                        None => kept_head = Some(first),
                    }
                }
                kept_tail = Some(last);
            }
            self.free[index] = kept_head;
        }
        released
    }

    fn alloc(&mut self, index: usize) -> *mut u8 {
        if self.free[index].is_none() && !self.refill(index) {
            // Out of frames: other classes may be sitting on empty ones.
            if self.trim() == 0 || !self.refill(index) {
                return ptr::null_mut();
            }
        }
        let node = self.free[index].expect("refill left the free list empty");
        // SAFETY: every node on a free list is a live block that we linked.
        self.free[index] = unsafe { node.as_ref().next };
        self.live_bytes += CLASS_SIZES[index];
        node.as_ptr().cast()
    }

    /// # Safety
    /// `ptr` must have been returned by [`SmallHeap::alloc`] for the same
    /// `index` and not be freed already.
    unsafe fn dealloc(&mut self, ptr: *mut u8, index: usize) {
        let size = CLASS_SIZES[index];
        if cfg!(debug_assertions) {
            // SAFETY: the caller owns the whole block.
            unsafe { ptr.write_bytes(POISON, size) };
        }
        let block = ptr.cast::<FreeNode>();
        // SAFETY: the block is at least 16 bytes and aligned for a FreeNode.
        unsafe { block.write(FreeNode { next: self.free[index] }) };
        self.free[index] = NonNull::new(block);
        self.live_bytes -= size;
    }
}

/// A snapshot of the heap's bookkeeping
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapStats {
    /// Frames the small tier currently holds from the PMM.
    pub small_frames: usize,
    /// Bytes currently allocated from the small tier
    pub small_live_bytes: usize,
    /// Frames currently held by large allocations.
    pub large_frames: usize,
}

/// The kernel's general purpose allocator i.e. `global_alloc`
pub struct KernelHeap {
    small: IrqSpinLock<SmallHeap>,
    large_frames: AtomicUsize,
}

impl KernelHeap {
    pub const fn new() -> Self {
        Self { small: IrqSpinLock::new(SmallHeap::new()), large_frames: AtomicUsize::new(0) }
    }

    /// Gives every completely free small-tier frame back to the PMM and returns
    /// how many were released.
    pub fn trim(&self) -> usize {
        self.small.lock().trim()
    }

    pub fn stats(&self) -> HeapStats {
        let small = self.small.lock();
        HeapStats {
            small_frames: small.frames,
            small_live_bytes: small.live_bytes,
            large_frames: self.large_frames.load(Ordering::Relaxed),
        }
    }
}

impl Default for KernelHeap {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match Route::of(layout) {
            Some(Route::Small(index)) => self.small.lock().alloc(index),
            Some(Route::Pages(frames)) => {
                let order = order_for(frames);
                let mut ptr = alloc_pages(order);
                if ptr.is_null() && self.trim() > 0 {
                    ptr = alloc_pages(order);
                }
                if !ptr.is_null() {
                    free_frame_range(ptr as usize, frames, 1 << order);
                    self.large_frames.fetch_add(frames, Ordering::Relaxed);
                }
                ptr
            }
            None => ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        match Route::of(layout) {
            Some(Route::Small(index)) => unsafe { self.small.lock().dealloc(ptr, index) },
            Some(Route::Pages(frames)) => {
                self.large_frames.fetch_sub(frames, Ordering::Relaxed);
                free_frame_range(ptr as usize, 0, frames);
            }
            None => debug_assert!(false, "heap: dealloc of a layout that alloc cannot serve"),
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };

        let old_route = Route::of(layout);
        let new_route = Route::of(new_layout);

        // Same block (size class or frame count): nothing to move.
        if old_route.is_some() && old_route == new_route {
            return ptr;
        }
        // Fewer frames: give the tail back and keep the block where it is.
        #[allow(clippy::collapsible_if)] // false positive
        if let (Some(Route::Pages(old)), Some(Route::Pages(new))) = (old_route, new_route) {
            if new < old {
                free_frame_range(ptr as usize, new, old);
                self.large_frames.fetch_sub(old - new, Ordering::Relaxed);
                return ptr;
            }
        }

        // SAFETY: standard grow/shrink-by-copy, upholding the caller's contract.
        unsafe {
            let new_ptr = self.alloc(new_layout);
            if !new_ptr.is_null() {
                ptr::copy_nonoverlapping(ptr, new_ptr, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
            new_ptr
        }
    }
}