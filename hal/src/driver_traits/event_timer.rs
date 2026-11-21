//! The one-shot timer that raises an interrupt when the free-running clock
//! reaches a deadline.

use alloc::boxed::Box;

use crate::driver_traits::interrupt_controller::Irq;
use crate::services::device::Device;
use crate::services::registry::{Class, Entry, Iter};

static EVENT_TIMERS: Class<dyn EventTimer> = Class::new();

pub trait EventTimer: Sync {
    /// Prepares the timer, left disarmed, and returns the interrupt it raises.
    /// Called by [`crate::services::timer::init`], once interrupts work.
    /// `None` if the timer turns out not to count.
    fn init(&self) -> Option<Irq>;

    /// Fires the interrupt when the clock (`now_ns`) reaches `deadline_ns`,
    /// replacing any earlier setting. A deadline in the past fires right away.
    fn arm(&self, deadline_ns: u64);

    /// Stops the timer and clears its interrupt.
    fn disarm(&self);
}

/// The first event timer that was bound, if any.
pub fn primary_event_timer() -> Option<&'static dyn EventTimer> {
    EVENT_TIMERS.primary()
}

/// Every event timer that was bound, in the order they were bound.
pub fn event_timers() -> Iter<Entry<dyn EventTimer>> {
    EVENT_TIMERS.iter()
}

/// Called by a driver's probe to hand over the timer it started on `device`.
pub fn register(device: &'static Device, timer: impl EventTimer + 'static) -> &'static dyn EventTimer {
    let timer: &'static dyn EventTimer = Box::leak(Box::new(timer));
    EVENT_TIMERS.register(device, timer);
    timer
}
