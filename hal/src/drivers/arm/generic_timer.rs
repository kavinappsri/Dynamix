//! The ARM generic timer's physical comparator, the event timer on aarch64
//! and armv7.
//!
//! The free-running counter that `now_ns` reads is architecture code; this
//! driver is the part that raises an interrupt at a deadline. Its interrupt is
//! the second entry of the node's `interrupts` (the non-secure physical
//! timer), which only the interrupt controller can decode, so the controller
//! has to be bound first.

#[cfg(target_arch = "aarch64")]
use crate::aarch64::aarch64_timer as arch;
#[cfg(target_arch = "arm")]
use crate::armv7::armv7_timer as arch;

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::event_timer::{self, EventTimer};
use crate::driver_traits::interrupt_controller::Irq;
use crate::services::device::Device;

/// Which entry of `interrupts` is the non-secure physical timer: secure
/// physical, non-secure physical, virtual, hypervisor.
const NON_SECURE_PHYSICAL: usize = 1;

struct GenericTimer {
    irq: Irq,
}

impl EventTimer for GenericTimer {
    fn init(&self) -> Option<Irq> {
        arch::event_init();
        Some(self.irq)
    }

    fn arm(&self, deadline_ns: u64) {
        arch::event_arm(deadline_ns);
    }

    fn disarm(&self) {
        arch::event_disarm();
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let irq = match dev.irq(NON_SECURE_PHYSICAL) {
        Ok(spec) => spec.irq,
        // The controller is not up yet.
        Err(ProbeError::Defer) => return Err(ProbeError::Defer),
        // A tree that does not say gets the architecture's fixed PPI.
        Err(_) => arch::FALLBACK_TIMER_IRQ,
    };
    event_timer::register(dev, GenericTimer { irq });
    Ok(())
}

driver! {
    name: "arm-generic-timer",
    matches: [Match::Fdt("arm,armv8-timer"), Match::Fdt("arm,armv7-timer")],
    probe: probe,
}
