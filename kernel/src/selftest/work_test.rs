//! Boot-time exercise of the deferred work queue.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use hal::services::cpu;
use hal::services::interrupts::{self, Irq, IrqStatus};
use hal::services::work::{self, WorkError};

/// Each item appends its context digit, so the final value shows the order.
static LOG: AtomicU32 = AtomicU32::new(0);
static CHAINED: AtomicU32 = AtomicU32::new(0);
static IRQ_WORK_RAN: AtomicU32 = AtomicU32::new(0);
static IRQ_WORK_CTX: AtomicUsize = AtomicUsize::new(0);
static IRQ_CALLS: AtomicU32 = AtomicU32::new(0);
static IRQ_SUBMIT_OK: AtomicU32 = AtomicU32::new(0);

fn append(ctx: *mut ()) {
    let digit = ctx as usize as u32;
    LOG.store(LOG.load(Ordering::Relaxed) * 10 + digit, Ordering::Relaxed);
}

/// Submits another item while running, to prove the lock is not held.
fn chain(_ctx: *mut ()) {
    let _ = work::submit(chained_tail, 0 as *mut ());
}

fn chained_tail(_ctx: *mut ()) {
    CHAINED.fetch_add(1, Ordering::Relaxed);
}

fn noop(_ctx: *mut ()) {}

fn irq_work(ctx: *mut ()) {
    IRQ_WORK_CTX.store(ctx as usize, Ordering::Relaxed);
    IRQ_WORK_RAN.fetch_add(1, Ordering::Relaxed);
}

fn irq_handler(_irq: Irq, _ctx: *mut ()) -> IrqStatus {
    if work::submit(irq_work, 7 as *mut ()).is_ok() {
        IRQ_SUBMIT_OK.fetch_add(1, Ordering::Relaxed);
    }
    IRQ_CALLS.fetch_add(1, Ordering::Relaxed);
    IrqStatus::Handled
}

fn fifo_and_context() -> Result<(), &'static str> {
    for digit in 1..=3usize {
        work::submit(append, digit as *mut ()).map_err(|_| "submit failed")?;
    }
    if work::pending() != 3 {
        return Err("pending count wrong after submitting three items");
    }
    if LOG.load(Ordering::Relaxed) != 0 {
        return Err("an item ran before run_pending");
    }
    if work::run_pending() != 3 {
        return Err("run_pending did not run all three items");
    }
    if LOG.load(Ordering::Relaxed) != 123 {
        return Err("items ran out of order or with the wrong context");
    }
    if work::pending() != 0 {
        return Err("queue not empty after draining");
    }
    Ok(())
}

fn submit_from_item() -> Result<(), &'static str> {
    work::submit(chain, 0 as *mut ()).map_err(|_| "submit failed")?;
    // Only the item that was waiting runs; the one it queued waits.
    if work::run_pending() != 1 {
        return Err("run_pending ran items submitted during the call");
    }
    if work::pending() != 1 || CHAINED.load(Ordering::Relaxed) != 0 {
        return Err("item submitted from an item was lost or ran early");
    }
    if work::run_pending() != 1 || CHAINED.load(Ordering::Relaxed) != 1 {
        return Err("item submitted from an item never ran");
    }
    Ok(())
}

fn full_queue() -> Result<(), &'static str> {
    let rejected_before = work::stats().rejected;
    for _ in 0..work::CAPACITY {
        work::submit(noop, 0 as *mut ()).map_err(|_| "queue filled up early")?;
    }
    if work::submit(noop, 0 as *mut ()) != Err(WorkError::Full) {
        return Err("a full queue accepted another item");
    }
    if work::stats().rejected != rejected_before + 1 {
        return Err("rejected counter did not move");
    }
    if work::run_pending() != work::CAPACITY {
        return Err("a full queue did not drain completely");
    }
    // The ring wrapped around by now; it must still work.
    fifo_after_wrap()
}

fn fifo_after_wrap() -> Result<(), &'static str> {
    LOG.store(0, Ordering::Relaxed);
    for digit in 4..=6usize {
        work::submit(append, digit as *mut ()).map_err(|_| "submit failed after wrap")?;
    }
    work::run_pending();
    if LOG.load(Ordering::Relaxed) != 456 {
        return Err("order wrong after the ring wrapped");
    }
    Ok(())
}

/// The real use case: an interrupt handler submits, the main context runs it.
/// Skipped if the interrupt controller has no software interrupts.
fn from_interrupt() -> Result<(), &'static str> {
    let Some(irq) = interrupts::software_irq(0) else {
        return Ok(());
    };
    let handle = interrupts::register(irq, irq_handler, core::ptr::null_mut())
        .map_err(|_| "could not register the test handler")?;
    interrupts::enable(irq).map_err(|_| "could not enable the test interrupt")?;

    let was_enabled = cpu::irqs_enabled();
    cpu::irq_enable();

    let mut arrived = false;
    if interrupts::raise_self(irq).is_ok() {
        for _ in 0..50_000_000u32 {
            if IRQ_CALLS.load(Ordering::Relaxed) >= 1 {
                arrived = true;
                break;
            }
            core::hint::spin_loop();
        }
    }

    if !was_enabled {
        cpu::irq_disable();
    }
    let _ = interrupts::unregister(handle);

    if !arrived {
        return Err("software interrupt never arrived");
    }
    if IRQ_SUBMIT_OK.load(Ordering::Relaxed) != 1 {
        return Err("submit from interrupt context failed");
    }
    if IRQ_WORK_RAN.load(Ordering::Relaxed) != 0 || work::pending() != 1 {
        return Err("work ran inside the interrupt instead of waiting");
    }
    if work::run_pending() != 1
        || IRQ_WORK_RAN.load(Ordering::Relaxed) != 1
        || IRQ_WORK_CTX.load(Ordering::Relaxed) != 7
    {
        return Err("work submitted from an interrupt did not run correctly");
    }
    Ok(())
}

pub fn run() -> Result<(), &'static str> {
    fifo_and_context()?;
    submit_from_item()?;
    full_queue()?;
    from_interrupt()
}
