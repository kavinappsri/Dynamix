//! Physical address types shared by the MMU, the PMM and the boot code.
//!
//! All of them wrap a `usize`: every supported target identity-maps RAM and
//! device windows, so a physical address is also a pointer-sized integer.

/// Size of a physical frame (and of the smallest page) in bytes.
pub const FRAME_SIZE: usize = 4096;
/// `log2(FRAME_SIZE)`.
pub const FRAME_SHIFT: usize = 12;

/// A physical address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysAddr(usize);

impl PhysAddr {
    #[inline]
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// `align` must be a power of two.
    #[inline]
    pub const fn align_down(self, align: usize) -> Self {
        Self(self.0 & !(align - 1))
    }

    /// `align` must be a power of two. Saturates at the top of the address space.
    #[inline]
    pub const fn align_up(self, align: usize) -> Self {
        Self(self.0.saturating_add(align - 1) & !(align - 1))
    }

    #[inline]
    pub const fn is_aligned(self, align: usize) -> bool {
        self.0 & (align - 1) == 0
    }
}

/// A contiguous physical range `[start, start + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysRange {
    start: PhysAddr,
    len: usize,
}

impl PhysRange {
    pub const EMPTY: PhysRange = PhysRange { start: PhysAddr(0), len: 0 };

    #[inline]
    pub const fn new(start: PhysAddr, len: usize) -> Self {
        Self { start, len }
    }

    /// The range `[start, end)`; empty if `end <= start`.
    #[inline]
    pub const fn from_bounds(start: PhysAddr, end: PhysAddr) -> Self {
        Self { start, len: end.0.saturating_sub(start.0) }
    }

    #[inline]
    pub const fn start(self) -> PhysAddr {
        self.start
    }

    /// One past the last byte (saturating).
    #[inline]
    pub const fn end(self) -> PhysAddr {
        PhysAddr(self.start.0.saturating_add(self.len))
    }

    #[inline]
    pub const fn len(self) -> usize {
        self.len
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn contains(self, addr: PhysAddr) -> bool {
        addr.0 >= self.start.0 && addr.0 < self.end().0
    }

    /// Whether the two ranges share at least one byte.
    #[inline]
    pub const fn overlaps(self, other: PhysRange) -> bool {
        !self.is_empty() && !other.is_empty() && self.start.0 < other.end().0 && other.start.0 < self.end().0
    }

    /// The smallest `align`-aligned range containing `self`.
    #[inline]
    pub const fn round_out(self, align: usize) -> PhysRange {
        PhysRange::from_bounds(self.start.align_down(align), self.end().align_up(align))
    }

    /// The largest `align`-aligned range contained in `self` (possibly empty).
    #[inline]
    pub const fn round_in(self, align: usize) -> PhysRange {
        PhysRange::from_bounds(self.start.align_up(align), self.end().align_down(align))
    }
}

/// A physical frame: a [`FRAME_SIZE`]-aligned address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysFrame(PhysAddr);

impl PhysFrame {
    /// `addr` must be frame aligned.
    #[inline]
    pub const fn from_addr(addr: PhysAddr) -> Self {
        debug_assert!(addr.is_aligned(FRAME_SIZE));
        Self(addr)
    }

    #[inline]
    pub const fn addr(self) -> PhysAddr {
        self.0
    }

    /// The frame number (`addr / FRAME_SIZE`).
    #[inline]
    pub const fn index(self) -> usize {
        self.0.as_usize() >> FRAME_SHIFT
    }

    /// The frame `count` frames further on.
    #[inline]
    pub const fn offset(self, count: usize) -> Self {
        Self(PhysAddr(self.0.as_usize() + count * FRAME_SIZE))
    }
}
