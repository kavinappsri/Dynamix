//! A fixed-size queue

use crate::services::cpu;
use crate::services::sync::IrqSpinLock;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueStats {
    /// Items waiting right now.
    pub pending: usize,
    /// Items pushed since the queue was made, including the dropped ones.
    pub pushed: u64,
    /// Items thrown away because the queue was full.
    pub dropped: u64,
}

struct Ring<T, const N: usize> {
    items: [T; N],
    head: usize,
    len: usize,
    pushed: u64,
    dropped: u64,
}

/// A queue of up to `N` items.
pub struct EventQueue<T: Copy, const N: usize> {
    ring: IrqSpinLock<Ring<T, N>>,
}

impl<T: Copy, const N: usize> EventQueue<T, N> {
    /// `filler` only pads the unused slots; it is never handed out.
    pub const fn new(filler: T) -> Self {
        Self { ring: IrqSpinLock::new(Ring { items: [filler; N], head: 0, len: 0, pushed: 0, dropped: 0 }) }
    }

    /// Queues `item`. Returns false (and counts a drop) if the queue is full.
    pub fn push(&self, item: T) -> bool {
        let mut ring = self.ring.lock();
        ring.pushed += 1;
        if ring.len == N {
            ring.dropped += 1;
            return false;
        }
        let slot = (ring.head + ring.len) % N;
        ring.items[slot] = item;
        ring.len += 1;
        true
    }

    /// The oldest item, if there is one.
    pub fn pop(&self) -> Option<T> {
        let mut ring = self.ring.lock();
        if ring.len == 0 {
            return None;
        }
        let item = ring.items[ring.head];
        ring.head = (ring.head + 1) % N;
        ring.len -= 1;
        Some(item)
    }

    /// Blocks until there is an item.
    ///
    /// If `polled`, nothing will interrupt to say an item arrived: `poll` is
    /// called in a spin loop and is expected to do the pushing. Otherwise the
    /// core sleeps until an interrupt (which is what pushes) wakes it.
    pub fn wait_pop(&self, polled: bool, mut poll: impl FnMut()) -> T {
        loop {
            if polled {
                poll();
            }
            if let Some(item) = self.pop() {
                return item;
            }
            if polled {
                core::hint::spin_loop();
                continue;
            }
            
            let state = cpu::irq_save();
            if self.ring.lock().len == 0 {
                cpu::irq_enable_and_wait();
            }
            cpu::irq_restore(state);
        }
    }

    /// A snapshot of the counters.
    pub fn stats(&self) -> QueueStats {
        let ring = self.ring.lock();
        QueueStats { pending: ring.len, pushed: ring.pushed, dropped: ring.dropped }
    }
}
