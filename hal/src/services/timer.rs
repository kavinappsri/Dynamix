//! Busy-waits, the clock, and interrupt-driven timers.
//!
//! * [`delay_us`] / [`delay_ms`] spin on the free-running counter. They work
//!   anywhere (early boot, interrupts masked) but burn the core.
//! * [`sleep`] halts the core until a timer interrupt wakes it. It needs
//!   [`init`] to have run and interrupts to be enabled, and quietly falls back
//!   to a busy-wait when they are not.
//! * [`after`] / [`every`] call a function once, or repeatedly, when a
//!   deadline passes.
//!
//! # Not done yet
//!
//! The table and the hardware arming are for the boot core only. With more
//! cores every core needs its own table and event timer (both backends are
//! per-core hardware already), and [`cancel`] will have to wait for a callback
//! that is running on another core.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use crate::services::cpu;
use crate::services::device_query::DeviceQuery;
use crate::services::interrupts::{self, Irq, IrqError, IrqStatus};
use crate::services::sync::IrqSpinLock;

/// Timers that can be pending at once.
pub const MAX_TIMERS: usize = 128;

/// What a timer calls when it expires. `ctx` is whatever was passed to
/// [`after`] / [`every`]. Runs in interrupt context.
pub type TimerFn = fn(ctx: *mut ());

/// Names one pending timer; pass to [`cancel`].
///
/// A handle goes stale once its timer has fired (one-shot) or been canceled,
/// and [`cancel`] on a stale handle is harmless.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerHandle {
    slot: u16,
    generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    /// [`init`] has not succeeded yet.
    NotReady,
    /// [`init`] already ran.
    AlreadyInitialized,
    /// The interrupt service has no controller to deliver the timer interrupt.
    NoInterruptController,
    /// This machine has no usable event timer.
    NoEventSource,
    /// [`MAX_TIMERS`] timers are already pending.
    Full,
    /// A periodic timer needs an interval greater than zero.
    InvalidInterval,
    /// The interrupt service refused the timer's interrupt line.
    Interrupt(IrqError),
}

/// Counters for diagnostics; see [`stats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerStats {
    /// Timers pending right now.
    pub pending: usize,
    /// When the hardware is armed to fire next (`now_ns` time base), if it is.
    pub armed_for_ns: Option<u64>,
    /// Timers started since boot.
    pub scheduled: u64,
    /// Callbacks that have been called since boot.
    pub fired: u64,
    /// Timers removed by [`cancel`] before they fired.
    pub cancelled: u64,
    /// Timers refused because the table was full.
    pub rejected: u64,
    /// Timer interrupts that found nothing expired yet (the hardware cannot
    /// always cover a long wait in one go, so this is normal).
    pub early_wakeups: u64,
    /// Periods a periodic timer skipped because it was serviced too late.
    pub missed_periods: u64,
}

#[derive(Clone, Copy)]
struct Entry {
    deadline_ns: u64,
    /// 0 for a one-shot timer.
    interval_ns: u64,
    /// `None` marks a free slot.
    func: Option<TimerFn>,
    // Kept as an integer so the table is `Send` without an `unsafe impl`.
    ctx: usize,
    generation: u32,
}

impl Entry {
    const EMPTY: Entry = Entry { deadline_ns: 0, interval_ns: 0, func: None, ctx: 0, generation: 0 };
}

const NOT_ARMED: u64 = u64::MAX;

struct Table {
    entries: [Entry; MAX_TIMERS],
    /// The deadline the hardware is armed for, or [`NOT_ARMED`].
    armed_for: u64,
    next_generation: u32,
    scheduled: u64,
    fired: u64,
    cancelled: u64,
    rejected: u64,
    early_wakeups: u64,
    missed_periods: u64,
}

impl Table {
    const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; MAX_TIMERS],
            armed_for: NOT_ARMED,
            next_generation: 0,
            scheduled: 0,
            fired: 0,
            cancelled: 0,
            rejected: 0,
            early_wakeups: 0,
            missed_periods: 0,
        }
    }

    fn insert(&mut self, deadline_ns: u64, interval_ns: u64, func: TimerFn, ctx: *mut ()) -> Result<TimerHandle, TimerError> {
        let Some(slot) = self.entries.iter().position(|entry| entry.func.is_none()) else {
            self.rejected += 1;
            return Err(TimerError::Full);
        };
        self.next_generation = self.next_generation.wrapping_add(1);
        self.entries[slot] = Entry {
            deadline_ns,
            interval_ns,
            func: Some(func),
            ctx: ctx as usize,
            generation: self.next_generation,
        };
        self.scheduled += 1;
        Ok(TimerHandle { slot: slot as u16, generation: self.next_generation })
    }

    fn earliest(&self) -> Option<u64> {
        self.entries.iter().filter(|entry| entry.func.is_some()).map(|entry| entry.deadline_ns).min()
    }

    fn pending(&self) -> usize {
        self.entries.iter().filter(|entry| entry.func.is_some()).count()
    }

    /// Makes the hardware match the table: armed for the earliest deadline,
    /// or off when nothing is pending.
    fn sync_hardware(&mut self) {
        match self.earliest() {
            Some(deadline) if deadline != self.armed_for => {
                backend::arm(deadline);
                self.armed_for = deadline;
            }
            Some(_) => {}
            None => {
                if self.armed_for != NOT_ARMED {
                    backend::disarm();
                    self.armed_for = NOT_ARMED;
                }
            }
        }
    }

    /// Takes the oldest expired timer: frees it (one-shot) or moves its
    /// deadline forward (periodic), and returns what to call.
    fn take_expired(&mut self, now_ns: u64) -> Option<(TimerFn, usize)> {
        let slot = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.func.is_some() && entry.deadline_ns <= now_ns)
            .min_by_key(|(_, entry)| entry.deadline_ns)
            .map(|(slot, _)| slot)?;

        let entry = &mut self.entries[slot];
        let func = entry.func?;
        let ctx = entry.ctx;

        if entry.interval_ns == 0 {
            entry.func = None;
        } else {
            let mut next = entry.deadline_ns.saturating_add(entry.interval_ns);
            if next <= now_ns {
                // Serviced so late that whole periods went by: skip them
                // instead of firing a burst to catch up.
                self.missed_periods += 1;
                next = now_ns.saturating_add(entry.interval_ns);
            }
            entry.deadline_ns = next;
        }
        self.fired += 1;
        Some((func, ctx))
    }
}

static TABLE: IrqSpinLock<Table> = IrqSpinLock::new(Table::new());
static READY: AtomicBool = AtomicBool::new(false);

// -------------------------------- Bring Up --------------------------------

/// Sets up the event timer and hooks its interrupt. Needs the interrupt
/// service ([`interrupts::init`]) to have found a controller. Interrupts must
/// still be enabled on the CPU ([`cpu::irq_enable`]) before any timer fires.
pub fn init(hardware: &dyn DeviceQuery) -> Result<(), TimerError> {
    if READY.load(Ordering::Acquire) {
        return Err(TimerError::AlreadyInitialized);
    }
    if !interrupts::is_ready() {
        return Err(TimerError::NoInterruptController);
    }

    let irq = backend::init(hardware)?;
    // The handle is dropped on purpose: the timer interrupt is never unhooked.
    interrupts::register(irq, timer_interrupt, core::ptr::null_mut()).map_err(TimerError::Interrupt)?;
    interrupts::enable(irq).map_err(TimerError::Interrupt)?;

    READY.store(true, Ordering::Release);
    Ok(())
}

/// Whether [`init`] has succeeded, i.e. whether [`after`] and [`sleep`] can
/// use the event timer.
pub fn is_ready() -> bool {
    READY.load(Ordering::Acquire)
}

fn timer_interrupt(_irq: Irq, _ctx: *mut ()) -> IrqStatus {
    on_interrupt();
    IrqStatus::Handled
}

fn on_interrupt() {
    // Silence the hardware first: on ARM the timer's line stays asserted
    // until it is disabled.
    backend::disarm();
    let now = now_ns();

    let mut any = false;
    loop {
        // The lock is not held while the callback runs.
        let next = TABLE.lock().take_expired(now);
        let Some((func, ctx)) = next else { break };
        any = true;
        func(ctx as *mut ());
    }

    let mut table = TABLE.lock();
    if !any {
        table.early_wakeups += 1;
    }
    // The hardware was just disarmed, whatever the table believed.
    table.armed_for = NOT_ARMED;
    table.sync_hardware();
}

// -------------------------------- Timers --------------------------------

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn schedule(first: Duration, interval: Duration, func: TimerFn, ctx: *mut ()) -> Result<TimerHandle, TimerError> {
    if !is_ready() {
        return Err(TimerError::NotReady);
    }
    let deadline_ns = now_ns().saturating_add(duration_ns(first));
    let mut table = TABLE.lock();
    let handle = table.insert(deadline_ns, duration_ns(interval), func, ctx)?;
    table.sync_hardware();
    Ok(handle)
}

/// Calls `func(ctx)` once, `delay` from now (a little later, never earlier).
pub fn after(delay: Duration, func: TimerFn, ctx: *mut ()) -> Result<TimerHandle, TimerError> {
    schedule(delay, Duration::ZERO, func, ctx)
}

/// Calls `func(ctx)` every `interval`, the first time `interval` from now,
/// until [`cancel`]led. If the callback is serviced so late that periods were
/// missed, those periods are skipped rather than made up.
pub fn every(interval: Duration, func: TimerFn, ctx: *mut ()) -> Result<TimerHandle, TimerError> {
    if interval.is_zero() {
        return Err(TimerError::InvalidInterval);
    }
    schedule(interval, interval, func, ctx)
}

/// Stops a timer. Returns whether it was still pending: `false` means it had
/// already fired (one-shot) or been canceled.
pub fn cancel(handle: TimerHandle) -> bool {
    let mut table = TABLE.lock();
    let Some(entry) = table.entries.get_mut(handle.slot as usize) else { return false };
    if entry.func.is_none() || entry.generation != handle.generation {
        return false;
    }
    entry.func = None;
    table.cancelled += 1;
    table.sync_hardware();
    true
}

/// A snapshot of the timer table's counters.
pub fn stats() -> TimerStats {
    let table = TABLE.lock();
    TimerStats {
        pending: table.pending(),
        armed_for_ns: (table.armed_for != NOT_ARMED).then_some(table.armed_for),
        scheduled: table.scheduled,
        fired: table.fired,
        cancelled: table.cancelled,
        rejected: table.rejected,
        early_wakeups: table.early_wakeups,
        missed_periods: table.missed_periods,
    }
}

fn wake(ctx: *mut ()) {
    // SAFETY: `ctx` is the address of the flag in `sleep`, which does not
    // return until this has run.
    unsafe { &*(ctx as *const AtomicBool) }.store(true, Ordering::Release);
}

/// Halts the core for at least `duration`.
pub fn sleep(duration: Duration) {
    if duration.is_zero() {
        return;
    }
    if !is_ready() || !cpu::irqs_enabled() {
        busy_wait(duration);
        return;
    }

    let woke = AtomicBool::new(false);
    if after(duration, wake, &woke as *const AtomicBool as *mut ()).is_err() {
        busy_wait(duration);
        return;
    }

    loop {
        // Masked while checking, so the wake-up cannot slip in between the
        // check and the idle: the idle re-enables interrupts atomically.
        cpu::irq_disable();
        if woke.load(Ordering::Acquire) {
            cpu::irq_enable();
            return;
        }
        cpu::irq_enable_and_wait();
    }
}

fn busy_wait(duration: Duration) {
    delay_us(u64::try_from(duration.as_micros()).unwrap_or(u64::MAX));
}

// -------------------------------- Hardware --------------------------------

/// What the architecture provides: a one-shot event timer that raises an
/// interrupt when the free-running clock reaches a deadline.
mod backend {
    use super::*;

    /// Prepares the event timer, left disarmed, and returns its interrupt.
    pub fn init(hardware: &dyn DeviceQuery) -> Result<Irq, TimerError> {
        #[cfg(target_arch = "x86_64")]
        {
            let _ = hardware;
            return crate::x86_64::x64_timer::event_init().ok_or(TimerError::NoEventSource);
        }
        #[cfg(target_arch = "aarch64")]
        return Ok(crate::aarch64::aarch64_timer::event_init(hardware));
        #[cfg(target_arch = "arm")]
        return Ok(crate::armv7::armv7_timer::event_init(hardware));
    }

    /// Arms the event timer for `deadline_ns` (the [`now_ns`] time base),
    /// replacing any earlier setting. A deadline in the past fires right away.
    pub fn arm(deadline_ns: u64) {
        #[cfg(target_arch = "x86_64")]
        crate::x86_64::x64_timer::event_arm(deadline_ns);
        #[cfg(target_arch = "aarch64")]
        crate::aarch64::aarch64_timer::event_arm(deadline_ns);
        #[cfg(target_arch = "arm")]
        crate::armv7::armv7_timer::event_arm(deadline_ns);
    }

    /// Stops the event timer and clears its interrupt.
    pub fn disarm() {
        #[cfg(target_arch = "x86_64")]
        crate::x86_64::x64_timer::event_disarm();
        #[cfg(target_arch = "aarch64")]
        crate::aarch64::aarch64_timer::event_disarm();
        #[cfg(target_arch = "arm")]
        crate::armv7::armv7_timer::event_disarm();
    }
}

/// Converts nanoseconds to counter ticks at `hz`, rounding down.
pub fn ns_to_ticks(ns: u64, hz: u64) -> u64 {
    let ticks = (ns as u128 * hz as u128) / 1_000_000_000;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The interrupt of the ARM generic timer's non-secure physical timer, from
/// the device tree's `interrupts` property (the second of its four entries),
/// as an id the GIC understands.
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
pub(crate) fn arm_timer_irq_from_dt(hardware: &dyn DeviceQuery) -> Option<Irq> {
    let dtb = hardware.as_dtb()?;
    let node = dtb.find_compatible("arm,armv8-timer").or_else(|| dtb.find_compatible("arm,armv7-timer"))?;
    let property = node.property("interrupts")?;
    // Each entry is three cells: type (0 = SPI, 1 = PPI), number, flags.
    let entry = property.value().get(12..24)?;
    let cell = |at: usize| u32::from_be_bytes([entry[at], entry[at + 1], entry[at + 2], entry[at + 3]]);
    match cell(0) {
        1 => Some(Irq(cell(4) + 16)),
        0 => Some(Irq(cell(4) + 32)),
        _ => None,
    }
}

// -------------------------------- Clock --------------------------------

pub fn ticks_to_ns(ticks: u64, hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    let secs = ticks / hz;
    let rem = ticks % hz;
    secs.wrapping_mul(1_000_000_000)
        .wrapping_add(rem * 1_000_000_000 / hz)
}

pub fn delay_ms(ms: u64) {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_timer::delay_ms(ms);

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_timer::delay_ms(ms);

    #[cfg(target_arch = "x86_64")]
    {
        crate::x86_64::x64_timer::delay_ms(ms);
    }
}

/// Busy-waits for at least `us` microseconds.
pub fn delay_us(us: u64) {
    #[cfg(target_arch = "aarch64")]
    crate::aarch64::aarch64_timer::delay_us(us);

    #[cfg(target_arch = "arm")]
    crate::armv7::armv7_timer::delay_us(us);

    #[cfg(target_arch = "x86_64")]
    crate::x86_64::x64_timer::delay_us(us);
}

/// Nanoseconds since the platform's free-running counter started.
pub fn now_ns() -> u64 {
    #[cfg(target_arch = "aarch64")]
    return crate::aarch64::aarch64_timer::now_ns();

    #[cfg(target_arch = "arm")]
    return crate::armv7::armv7_timer::now_ns();

    #[cfg(target_arch = "x86_64")]
    return crate::x86_64::x64_timer::now_ns();
}
