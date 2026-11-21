//! Interrupt Controller interface

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::registry::{Class, Entry, Iter};
use crate::services::sync::StaticCell;

/// The controller the interrupt service chose to use.
static ACTIVE_INTERRUPT_CONTROLLER: StaticCell<&'static dyn InterruptController> = StaticCell::uninit();

/// Every interrupt controller that was bound.
static CONTROLLERS: Class<dyn InterruptController> = Class::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Irq(pub u32);

impl Irq {
    #[inline]
    pub const fn number(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CpuId(pub u32);

impl CpuId {
    pub const BOOT: CpuId = CpuId(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Edge,
    Level,
}

/// Which signal level (or edge direction) counts as asserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow,
}

/// An interrupt line of a device, with how it is signalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqSpec {
    pub irq: Irq,
    pub trigger: Trigger,
    pub polarity: Polarity,
}

/// An interrupt the controller has handed to the CPU, to be given back to
/// [`InterruptController::end_of_interrupt`] once it has been serviced.
#[derive(Clone, Copy, Debug)]
pub struct Claim {
    pub irq: Irq,
    /// Controller-private acknowledge value (the GIC needs its full IAR
    /// value, source CPU included, to signal end of interrupt).
    pub raw: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqError {
    /// The interrupt number is outside what the controller supports.
    OutOfRange,
    /// The controller (or this interrupt) cannot do what was asked.
    Unsupported,
    /// No interrupt controller was found or initialised.
    NoController,
    /// The interrupt service is already initialised.
    AlreadyInitialized,
    /// The handler table is full.
    NoFreeSlots,
    /// No such registration.
    NotRegistered,
    /// Out of interrupt vectors (x86).
    NoFreeVectors,
    /// The controller hardware did not respond as expected.
    InitFailed,
}

/// An interrupt controller.
pub trait InterruptController: Sync {
    /// One past the highest interrupt number this controller handles.
    fn irq_count(&self) -> u32;

    /// Brings up this controller's per-core part on the calling core. Called
    /// once on the boot core by [`crate::services::interrupts::init`], and
    /// by every secondary core as it starts.
    fn init_this_cpu(&self) -> Result<(), IrqError>;

    /// Unmasks `irq` at the controller.
    fn enable(&self, irq: Irq) -> Result<(), IrqError>;

    /// Masks `irq` at the controller.
    fn disable(&self, irq: Irq) -> Result<(), IrqError>;

    /// Sets how `irq` is signalled. Call before enabling it.
    fn configure(&self, irq: Irq, trigger: Trigger, polarity: Polarity) -> Result<(), IrqError>;

    /// Decodes one interrupt specifier as a device tree writes it (the cells
    /// of one `interrupts` entry, whose length is this controller's
    /// `#interrupt-cells`). Only the controller knows the format, which is why
    /// this is its job. Controllers that no device tree device can name
    /// (the x86 APIC) leave it unsupported.
    fn translate(&self, _cells: &[u32]) -> Result<IrqSpec, IrqError> {
        Err(IrqError::Unsupported)
    }

    /// Routes `irq` to `cpu`. Controllers that cannot route return
    /// [`IrqError::Unsupported`].
    fn set_affinity(&self, _irq: Irq, _cpu: CpuId) -> Result<(), IrqError> {
        Err(IrqError::Unsupported)
    }

    /// Acknowledges the interrupt that is being delivered to the calling
    /// core. `None` means nothing is pending (a spurious interrupt) and no
    /// end-of-interrupt must follow.
    ///
    /// `cpu_vector` is the vector the CPU reported, for architectures where
    /// the core rather than the controller names the interrupt
    /// It is `None` where the controller has an acknowledge
    /// register (the GIC).
    fn acknowledge(&self, cpu_vector: Option<u32>) -> Option<Claim>;

    /// Tells the controller the interrupt in `claim` has been serviced.
    fn end_of_interrupt(&self, claim: Claim);

    /// The interrupt number of software-triggered line `n` (SGI `n` on the
    /// GIC, a reserved self-IPI vector on x86), if there is one.
    fn software_irq(&self, n: u8) -> Option<Irq>;

    /// Triggers the software interrupt `irq` on the calling core.
    fn raise_self(&self, irq: Irq) -> Result<(), IrqError>;
}

/// The controller selected at boot, if any.
pub fn active_interrupt_controller() -> Option<&'static dyn InterruptController> {
    ACTIVE_INTERRUPT_CONTROLLER.get().copied()
}

/// Records `controller` as the active one. Only the first call counts.
pub(crate) fn set_active_interrupt_controller(controller: &'static dyn InterruptController) {
    ACTIVE_INTERRUPT_CONTROLLER.get_or_init(|| controller);
}

/// Every interrupt controller that was bound, in the order they were bound.
pub fn interrupt_controllers() -> Iter<Entry<dyn InterruptController>> {
    CONTROLLERS.iter()
}

/// The first interrupt controller that was bound, if any.
pub fn primary_interrupt_controller() -> Option<&'static dyn InterruptController> {
    CONTROLLERS.primary()
}

/// Called by a driver's probe to hand over the controller it started on `device`.
pub fn register(
    device: &'static Device,
    controller: impl InterruptController + 'static,
) -> &'static dyn InterruptController {
    let controller: &'static dyn InterruptController = Box::leak(Box::new(controller));
    CONTROLLERS.register(device, controller);
    controller
}

/// Like [`register`], for a controller that already lives in a static because
/// architecture code reaches it directly too (the x86 APIC).
pub fn register_static(device: &'static Device, controller: &'static dyn InterruptController) {
    CONTROLLERS.register(device, controller);
}
