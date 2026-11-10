//! The interrupt service
//!
//! ### Interrupt handling
//! The handler is going to be simply a `fn(Irq, *mut ()) -> IrqStatus`, and a context pointer.
//! Multiple handlers can share one line, all of them will be called when the interrupt fires
//!
//! Handlers will run with interrupts masked on their core. Thus, they have to be short, non-blocking
//! and shouldn't allocate. allocating is _possible_, but I don't think it's a good idea.

use core::sync::atomic::{AtomicU16, AtomicU32, AtomicUsize, Ordering};

pub use crate::driver_traits::interrupt_controller::{
    Claim, CpuId, InterruptController, Irq, IrqError, Polarity, Trigger,
};
use crate::driver_traits::interrupt_controller::{active_interrupt_controller, set_active_interrupt_controller};
#[cfg(not(target_arch = "x86_64"))]
use crate::driver_traits::interrupt_controller::probe_interrupt_controller;
use crate::services::device_query::DeviceQuery;
use crate::services::sync::IrqSpinLock;

/// Highest interrupt number the handler table can hold, plus one.
pub const MAX_IRQS: usize = 1024;
/// Handlers that can be registered at once, across all lines.
pub const MAX_HANDLERS: usize = 64;

const NO_SLOT: u16 = u16::MAX;

/// A handler's verdict on an interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqStatus {
    /// The device behind this handler raised the interrupt, and it was serviced.
    Handled,
    /// Not this handler's device (only meaningful on a shared line).
    NotMine,
}

/// An interrupt handler. `ctx` is whatever was passed to [`register`].
pub type IrqHandler = fn(irq: Irq, ctx: *mut ()) -> IrqStatus;

/// A registered handler; pass to [`unregister`] to remove it.
#[derive(Debug)]
pub struct IrqHandle {
    irq: Irq,
    slot: u16,
}

/// How a legacy (ISA) interrupt line is actually wired on this machine.
#[derive(Clone, Copy, Debug)]
pub struct IrqConfig {
    pub irq: Irq,
    pub trigger: Trigger,
    pub polarity: Polarity,
}

struct Slot {
    handler: AtomicUsize,
    ctx: AtomicUsize,
    next: AtomicU16,
}

struct Line {
    head: AtomicU16,
    /// Dispatches currently walking this line, on any core.
    in_flight: AtomicU32,
}

static SLOTS: [Slot; MAX_HANDLERS] = [const {
    Slot { handler: AtomicUsize::new(0), ctx: AtomicUsize::new(0), next: AtomicU16::new(NO_SLOT) }
}; MAX_HANDLERS];

static LINES: [Line; MAX_IRQS] = [const {
    Line { head: AtomicU16::new(NO_SLOT), in_flight: AtomicU32::new(0) }
}; MAX_IRQS];

/// Which of [`SLOTS`] are taken. Only touched under the lock.
static USED_SLOTS: IrqSpinLock<u64> = IrqSpinLock::new(0);

/// Interrupts that arrived on a line nobody had registered a handler for.
static UNHANDLED: AtomicU32 = AtomicU32::new(0);
/// Times the controller reported nothing pending when an interrupt arrived.
static SPURIOUS: AtomicU32 = AtomicU32::new(0);

const _: () = assert!(MAX_HANDLERS <= 64, "USED_SLOTS is a u64 bitmap");
const _: () = assert!(MAX_HANDLERS < NO_SLOT as usize);

// -------------------------------- Bring Up --------------------------------

/// Finds and initializes the interrupt controller of this device
pub fn init(hardware: &dyn DeviceQuery) -> Result<(), IrqError> {
    if active_interrupt_controller().is_some() {
        return Err(IrqError::AlreadyInitialized);
    }

    #[cfg(target_arch = "x86_64")]
    let controller: &'static dyn InterruptController = crate::x86_64::x64_apic::init(hardware)?;
    #[cfg(not(target_arch = "x86_64"))]
    let controller: &'static dyn InterruptController =
        probe_interrupt_controller(hardware).map_err(|_| IrqError::NoController)?;

    set_active_interrupt_controller(controller);
    controller.init_this_cpu()
}

pub fn init_this_cpu() -> Result<(), IrqError> {
    controller()?.init_this_cpu()
}

/// Whether [`init`] has found a controller.
pub fn is_ready() -> bool {
    active_interrupt_controller().is_some()
}

fn controller() -> Result<&'static dyn InterruptController, IrqError> {
    active_interrupt_controller().ok_or(IrqError::NoController)
}

fn check_range(irq: Irq) -> Result<(), IrqError> {
    if irq.0 as usize >= MAX_IRQS {
        return Err(IrqError::OutOfRange);
    }

    #[allow(clippy::collapsible_if)] //false positive
    if let Some(controller) = active_interrupt_controller() {
        if irq.0 >= controller.irq_count() {
            return Err(IrqError::OutOfRange);
        }
    }
    Ok(())
}

// -------------------------------- Registration --------------------------------

/// Adds `handler` to `irq`. The line stays masked until [`enable`] is called.
///
/// `ctx` is handed back to the handler untouched; it is up to the caller to
/// keep whatever it points to alive until [`unregister`] returns.
pub fn register(irq: Irq, handler: IrqHandler, ctx: *mut ()) -> Result<IrqHandle, IrqError> {
    check_range(irq)?;

    let mut used = USED_SLOTS.lock();
    let slot = (!*used).trailing_zeros() as usize;
    if slot >= MAX_HANDLERS {
        return Err(IrqError::NoFreeSlots);
    }
    *used |= 1 << slot;

    let line = &LINES[irq.0 as usize];
    SLOTS[slot].handler.store(handler as usize, Ordering::Relaxed);
    SLOTS[slot].ctx.store(ctx as usize, Ordering::Relaxed);
    SLOTS[slot].next.store(line.head.load(Ordering::Relaxed), Ordering::Relaxed);
    // Publishes the slot's fields to any core walking the line.
    line.head.store(slot as u16, Ordering::Release);

    Ok(IrqHandle { irq, slot: slot as u16 })
}

/// Removes a handler. When this returns, the handler is not running on any
/// core and will not be called again.
///
/// Must not be called from a handler of the same line (it would wait for
/// itself).
pub fn unregister(handle: IrqHandle) -> Result<(), IrqError> {
    let line = &LINES[handle.irq.0 as usize];

    {
        let _used = USED_SLOTS.lock();
        let slot = handle.slot;
        let next = SLOTS[slot as usize].next.load(Ordering::Relaxed);

        if line.head.load(Ordering::Relaxed) == slot {
            line.head.store(next, Ordering::Release);
        } else {
            let mut cursor = line.head.load(Ordering::Relaxed);
            loop {
                if cursor == NO_SLOT {
                    return Err(IrqError::NotRegistered);
                }
                let following = SLOTS[cursor as usize].next.load(Ordering::Relaxed);
                if following == slot {
                    SLOTS[cursor as usize].next.store(next, Ordering::Release);
                    break;
                }
                cursor = following;
            }
        }
    }

    // Unlinked: new dispatches cannot reach the slot, so wait out the ones
    // that already can. (Lock released: a dispatch must not be stuck behind it.)
    while line.in_flight.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }

    let mut used = USED_SLOTS.lock();
    *used &= !(1 << handle.slot);
    Ok(())
}

/// Registers `handler`, programs the line's trigger and polarity, and enables
/// it: the usual sequence for a driver.
pub fn request(
    irq: Irq,
    trigger: Trigger,
    polarity: Polarity,
    handler: IrqHandler,
    ctx: *mut (),
) -> Result<IrqHandle, IrqError> {
    let controller = controller()?;
    controller.configure(irq, trigger, polarity)?;
    let handle = register(irq, handler, ctx)?;
    if let Err(err) = controller.enable(irq) {
        let _ = unregister(handle);
        return Err(err);
    }
    Ok(handle)
}

// -------------------------------- Line Control --------------------------------

/// Unmasks `irq` at the controller.
pub fn enable(irq: Irq) -> Result<(), IrqError> {
    check_range(irq)?;
    controller()?.enable(irq)
}

/// Masks `irq` at the controller.
pub fn disable(irq: Irq) -> Result<(), IrqError> {
    check_range(irq)?;
    controller()?.disable(irq)
}

/// Sets how `irq` is signalled. Call before [`enable`].
pub fn configure(irq: Irq, trigger: Trigger, polarity: Polarity) -> Result<(), IrqError> {
    check_range(irq)?;
    controller()?.configure(irq, trigger, polarity)
}

/// Routes `irq` to `cpu`, where the controller can.
pub fn set_affinity(irq: Irq, cpu: CpuId) -> Result<(), IrqError> {
    check_range(irq)?;
    controller()?.set_affinity(irq, cpu)
}

/// The interrupt number of software line `n` (0..16), if the controller has it.
pub fn software_irq(n: u8) -> Option<Irq> {
    active_interrupt_controller()?.software_irq(n)
}

// Triggers software interrupt `irq` on the calling core. Interrupts still
/// have to be enabled on the core (and the line registered and enabled) for
/// the handler to run.
pub fn raise_self(irq: Irq) -> Result<(), IrqError> {
    check_range(irq)?;
    controller()?.raise_self(irq)
}

/// How legacy ISA line `line` (0..16) is wired on this machine, after any
/// firmware overrides. `None` on machines without ISA interrupts.
pub fn isa_irq(line: u8) -> Option<IrqConfig> {
    #[cfg(target_arch = "x86_64")]
    return crate::x86_64::x64_apic::isa_irq(line);
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = line;
        None
    }
}

// -------------------------------- Dispatch --------------------------------

/// Calls every handler registered on `irq`. `None` means there was no handler.
pub fn dispatch(irq: Irq) -> Option<IrqStatus> {
    let line = LINES.get(irq.0 as usize)?;

    line.in_flight.fetch_add(1, Ordering::Acquire);
    let mut cursor = line.head.load(Ordering::Acquire);
    let mut any = false;
    let mut handled = false;

    while cursor != NO_SLOT {
        let slot = &SLOTS[cursor as usize];
        let handler = slot.handler.load(Ordering::Relaxed);
        let ctx = slot.ctx.load(Ordering::Relaxed);
        cursor = slot.next.load(Ordering::Acquire);

        // SAFETY: `handler` was stored from an `IrqHandler` by `register`, and
        // the slot cannot be recycled while `in_flight` is nonzero.
        let handler: IrqHandler = unsafe { core::mem::transmute::<usize, IrqHandler>(handler) };
        any = true;
        if handler(irq, ctx as *mut ()) == IrqStatus::Handled {
            handled = true;
        }
    }
    line.in_flight.fetch_sub(1, Ordering::Release);

    match (any, handled) {
        (false, _) => None,
        (true, true) => Some(IrqStatus::Handled),
        (true, false) => Some(IrqStatus::NotMine),
    }
}

/// What the architecture entry code calls once it has saved the interrupted
/// context: asks the controller what fired, runs its handlers, signals end of
/// interrupt.
///
/// `cpu_vector` is the vector the CPU reported (x86), or `None` where the
/// controller identifies the interrupt itself (the GIC), in which case this
/// drains every pending interrupt before returning.
pub fn handle_interrupt(cpu_vector: Option<u32>) {
    let Some(controller) = active_interrupt_controller() else { return };

    loop {
        let Some(claim) = controller.acknowledge(cpu_vector) else {
            SPURIOUS.fetch_add(1, Ordering::Relaxed);
            return;
        };

        if dispatch(claim.irq).is_none() {
            // Nobody is listening. Mask it, or a level-triggered line would
            // fire again the instant it is acknowledged.
            UNHANDLED.fetch_add(1, Ordering::Relaxed);
            let _ = controller.disable(claim.irq);
        }
        controller.end_of_interrupt(claim);

        if cpu_vector.is_some() {
            return;
        }
    }
}

/// Interrupts that fired with no handler registered (the line was masked).
pub fn unhandled_count() -> u32 {
    UNHANDLED.load(Ordering::Relaxed)
}

/// Interrupts the controller reported nothing pending for.
pub fn spurious_count() -> u32 {
    SPURIOUS.load(Ordering::Relaxed)
}