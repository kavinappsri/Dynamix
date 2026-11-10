//! Boot-time exercise of the interrupt service.

use core::sync::atomic::{AtomicU32, Ordering};

use hal::services::cpu;
use hal::services::interrupts::{self, Irq, IrqError, IrqStatus};

const CTX_A: usize = 0xA11C_E000;
const CTX_B: usize = 0xB0B0_0000;

static A_CALLS: AtomicU32 = AtomicU32::new(0);
static B_CALLS: AtomicU32 = AtomicU32::new(0);
static BAD_CTX: AtomicU32 = AtomicU32::new(0);

fn handler_a(_irq: Irq, ctx: *mut ()) -> IrqStatus {
    if ctx as usize != CTX_A {
        BAD_CTX.fetch_add(1, Ordering::Relaxed);
    }
    A_CALLS.fetch_add(1, Ordering::Relaxed);
    IrqStatus::Handled
}

/// Shares the line and declines the interrupt, as a handler for a different
/// device on the same line would.
fn handler_b(_irq: Irq, ctx: *mut ()) -> IrqStatus {
    if ctx as usize != CTX_B {
        BAD_CTX.fetch_add(1, Ordering::Relaxed);
    }
    B_CALLS.fetch_add(1, Ordering::Relaxed);
    IrqStatus::NotMine
}

fn fail(_: IrqError) -> &'static str {
    "interrupt service call failed"
}

/// Raises the software interrupt and waits (bounded) for `counter` to reach `expected`.
fn raise_and_wait(irq: Irq, counter: &AtomicU32, expected: u32) -> Result<(), &'static str> {
    interrupts::raise_self(irq).map_err(fail)?;
    for _ in 0..50_000_000u32 {
        if counter.load(Ordering::Relaxed) >= expected {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err("software interrupt never arrived")
}

pub fn run() -> Result<(), &'static str> {
    let irq = interrupts::software_irq(0).ok_or("controller has no software interrupts")?;

    let a = interrupts::register(irq, handler_a, CTX_A as *mut ()).map_err(fail)?;
    let b = interrupts::register(irq, handler_b, CTX_B as *mut ()).map_err(fail)?;
    interrupts::enable(irq).map_err(fail)?;

    let was_enabled = cpu::irqs_enabled();
    cpu::irq_enable();

    let result = (|| {
        raise_and_wait(irq, &A_CALLS, 1)?;
        // A second one proves end-of-interrupt was signalled correctly.
        raise_and_wait(irq, &A_CALLS, 2)?;
        if B_CALLS.load(Ordering::Relaxed) != A_CALLS.load(Ordering::Relaxed) {
            return Err("shared line did not call every handler");
        }
        if BAD_CTX.load(Ordering::Relaxed) != 0 {
            return Err("handler received the wrong context pointer");
        }
        Ok(())
    })();

    // Unregister both, then check the line is really cut off. A further
    // software interrupt now reaches nobody (it is counted as unhandled).
    interrupts::unregister(a).map_err(fail)?;
    interrupts::unregister(b).map_err(fail)?;
    let unhandled_before = interrupts::unhandled_count();
    let calls_before = A_CALLS.load(Ordering::Relaxed);
    let _ = interrupts::raise_self(irq);
    for _ in 0..50_000_000u32 {
        if interrupts::unhandled_count() != unhandled_before {
            break;
        }
        core::hint::spin_loop();
    }
    let after = (interrupts::unhandled_count() != unhandled_before
        && A_CALLS.load(Ordering::Relaxed) == calls_before)
        .then_some(())
        .ok_or("handler still called after unregister");

    if !was_enabled {
        cpu::irq_disable();
    }
    result.and(after)
}
