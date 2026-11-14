//! Deferred work queue.

use crate::services::sync::IrqSpinLock;

/// How many items can be waiting at once.
pub const CAPACITY: usize = 64;

/// A queued function. Receives the context pointer it was submitted with.
pub type WorkFn = fn(ctx: *mut ());

/// Why [`submit`] refused an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkError {
    /// [`CAPACITY`] items are already waiting; the new one was dropped.
    Full,
}

/// Counters for diagnostics; see [`stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkStats {
    /// Items waiting right now.
    pub pending: usize,
    /// The most items that were ever waiting at once.
    pub high_water: usize,
    /// Items accepted since boot.
    pub submitted: u64,
    /// Items that have finished running since boot.
    pub completed: u64,
    /// Items refused because the queue was full.
    pub rejected: u64,
}

#[derive(Clone, Copy)]
struct Item {
    func: WorkFn,
    // Kept as an integer so the queue is `Send` without an `unsafe impl`.
    ctx: usize,
}

struct Queue {
    items: [Option<Item>; CAPACITY],
    head: usize,
    len: usize,
    high_water: usize,
    submitted: u64,
    completed: u64,
    rejected: u64,
}

impl Queue {
    const fn new() -> Self {
        Self {
            items: [None; CAPACITY],
            head: 0,
            len: 0,
            high_water: 0,
            submitted: 0,
            completed: 0,
            rejected: 0,
        }
    }
}

static QUEUE: IrqSpinLock<Queue> = IrqSpinLock::new(Queue::new());

/// Queues `func` to be called with `ctx` by a later [`run_pending`].
pub fn submit(func: WorkFn, ctx: *mut ()) -> Result<(), WorkError> {
    let mut queue = QUEUE.lock();
    if queue.len == CAPACITY {
        queue.rejected += 1;
        return Err(WorkError::Full);
    }
    let slot = (queue.head + queue.len) % CAPACITY;
    queue.items[slot] = Some(Item { func, ctx: ctx as usize });
    queue.len += 1;
    queue.submitted += 1;
    if queue.len > queue.high_water {
        queue.high_water = queue.len;
    }
    Ok(())
}

fn take_next() -> Option<Item> {
    let mut queue = QUEUE.lock();
    if queue.len == 0 {
        return None;
    }
    let head = queue.head;
    let item = queue.items[head].take();
    queue.head = (head + 1) % CAPACITY;
    queue.len -= 1;
    item
}

/// Runs the items that were waiting when this was called, oldest first, and
/// returns how many ran.
pub fn run_pending() -> usize {
    let budget = pending();
    let mut ran = 0;
    while ran < budget {
        let Some(item) = take_next() else { break };
        (item.func)(item.ctx as *mut ());
        QUEUE.lock().completed += 1;
        ran += 1;
    }
    ran
}

/// How many items are waiting. An item that is running right now is not counted.
pub fn pending() -> usize {
    QUEUE.lock().len
}

/// A snapshot of the queue's counters.
pub fn stats() -> WorkStats {
    let queue = QUEUE.lock();
    WorkStats {
        pending: queue.len,
        high_water: queue.high_water,
        submitted: queue.submitted,
        completed: queue.completed,
        rejected: queue.rejected,
    }
}
