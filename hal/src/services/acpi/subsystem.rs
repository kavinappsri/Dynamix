use core::fmt;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use acpica_sys as sys;

use super::madt::Madt;
use super::namespace::Namespace;
use super::raw::check;
use super::sleep::SleepState;
use super::AcpiError;
use crate::services::acpica_osl;
use crate::services::addr::PhysAddr;
use crate::services::cpu;
use crate::services::sync::StaticCell;
use crate::services::acpi::ec::EmbeddedController;

/// How far ACPICA has come. Later levels include the earlier ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AcpiLevel {
    /// The static tables (MADT, FADT, ...) can be read, but the AML namespace
    /// did not load.
    Tables = 1,
    /// The namespace is loaded
    Namespace = 2,
    /// ACPI mode is on, the SCI handler is installed and the `_REG`/`_STA`/`_INI`
    /// methods have run.
    Enabled = 3,
}

impl AcpiLevel {
    fn from_raw(raw: u8) -> Self {
        match raw {
            3 => AcpiLevel::Enabled,
            2 => AcpiLevel::Namespace,
            _ => AcpiLevel::Tables,
        }
    }
}

impl fmt::Display for AcpiLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AcpiLevel::Tables => "tables only",
            AcpiLevel::Namespace => "namespace loaded",
            AcpiLevel::Enabled => "enabled",
        })
    }
}

/// How many table descriptors ACPICA starts with
const INITIAL_TABLE_COUNT: u32 = 16;

static STARTED: AtomicBool = AtomicBool::new(false);
static ENABLING: AtomicBool = AtomicBool::new(false);
static ACPI: StaticCell<Acpi> = StaticCell::uninit();

/// ACPICA, running. There is only ever one (it is C globals), and holding a
/// reference means [`AcpiRoot::init`](super::AcpiRoot::init) worked.
pub struct Acpi {
    rsdp: PhysAddr,
    level: AtomicU8,
}

impl Acpi {
    /// The running ACPICA, if `init` has worked.
    pub fn get() -> Option<&'static Acpi> {
        ACPI.get()
    }

    /// Brings up ACPICA as far as the namespace. See [`AcpiRoot::init`](super::AcpiRoot::init).
    pub(super) fn start(rsdp: PhysAddr) -> Result<(), AcpiError> {
        if STARTED.swap(true, Ordering::AcqRel) {
            return Err(AcpiError::AlreadyStarted);
        }
        acpica_osl::init(rsdp.as_usize() as u64);

        // SAFETY: called exactly once (the flag above), on the boot core,
        // with the heap and the final page tables up
        let level = match unsafe { bring_up() } {
            Ok(level) => level,
            Err(err) => {
                // SAFETY: ACPICA is initialized, or fails cleanly if not.
                let _ = unsafe { sys::AcpiTerminate() };
                return Err(err);
            }
        };
        ACPI.get_or_init(|| Acpi { rsdp, level: AtomicU8::new(level as u8) });
        Ok(())
    }

    /// Physical address of the RSDP ACPICA was started with.
    pub fn rsdp(&self) -> PhysAddr {
        self.rsdp
    }

    /// How far ACPICA has come.
    pub fn level(&self) -> AcpiLevel {
        AcpiLevel::from_raw(self.level.load(Ordering::Acquire))
    }

    fn require(&self, level: AcpiLevel) -> Result<(), AcpiError> {
        if self.level() >= level { Ok(()) } else { Err(AcpiError::NotReady) }
    }

    /// The MADT.
    pub fn madt(&self) -> Result<Madt, AcpiError> {
        Madt::get()
    }

    /// The EC
    pub fn embedded_controller(&self) -> Option<&'static EmbeddedController> {
        super::ec::get()
    }

    /// The namespace, once it is loaded ([`AcpiLevel::Namespace`]).
    pub fn namespace(&self) -> Result<Namespace<'_>, AcpiError> {
        self.require(AcpiLevel::Namespace)?;
        Ok(Namespace::new(self))
    }

    /// The `SLP_TYPa` / `SLP_TYPb` values of a sleep state, from the
    /// firmware's `_Sx` object. `NotFound` if firmware does not offer the state.
    pub fn sleep_type_data(&self, state: SleepState) -> Result<(u8, u8), AcpiError> {
        self.require(AcpiLevel::Namespace)?;
        let (mut typ_a, mut typ_b) = (0u8, 0u8);
        // SAFETY: valid out-pointers.
        check(unsafe { sys::AcpiGetSleepTypeData(state as u8, &mut typ_a, &mut typ_b) })?;
        Ok((typ_a, typ_b))
    }

    /// Puts the machine into `state`. For [`SleepState::S5`] success means
    /// the power is gone, so this only returns when it did not work.
    pub fn enter_sleep(&self, state: SleepState) -> Result<(), AcpiError> {
        self.require(AcpiLevel::Namespace)?;
        // SAFETY: plain ACPICA calls; the second one needs interrupts masked
        // (the caller of a power-off does not expect to be interrupted).
        unsafe {
            check(sys::AcpiEnterSleepStatePrep(state as u8))?;
            let saved = cpu::irq_save();
            let result = check(sys::AcpiEnterSleepState(state as u8));
            // Only reached if the machine did not go to sleep.
            cpu::irq_restore(saved);
            result
        }
    }

    /// Resets the machine through the FADT's reset register. Only returns on failure.
    pub fn reset(&self) -> Result<(), AcpiError> {
        self.require(AcpiLevel::Tables)?;
        // SAFETY: plain ACPICA call.
        check(unsafe { sys::AcpiReset() })
    }

    /// Turns on ACPI mode and everything that needs it. See
    /// [`AcpiRoot::enable`](super::AcpiRoot::enable).
    pub(super) fn enable(&self) -> Result<(), AcpiError> {
        match self.level() {
            AcpiLevel::Enabled => return Ok(()),
            AcpiLevel::Tables => return Err(AcpiError::NotReady),
            AcpiLevel::Namespace => {}
        }
        if ENABLING.swap(true, Ordering::AcqRel) {
            return Err(AcpiError::AlreadyStarted);
        }

        // SAFETY: the namespace is loaded (checked above), single boot core.
        unsafe {
            if let Err(err) = check(sys::AcpiEnableSubsystem(sys::ACPI_FULL_INITIALIZATION)) {
                ENABLING.store(false, Ordering::Release);
                return Err(err);
            }
            self.level.store(AcpiLevel::Enabled as u8, Ordering::Release);

            // Address-space handlers other than memory/I/O/PCI config go in
            // right here: they have to be there before `_REG` methods run.
            if let Ok(namespace) = self.namespace() {
                super::ec::init(&namespace);
            }

            check(sys::AcpiInitializeObjects(sys::ACPI_FULL_INITIALIZATION))
        }
    }
}

/// The ACPICA start-up sequence up to the namespace.
///
/// # Safety
/// Once only, with the heap and final page tables available.
unsafe fn bring_up() -> Result<AcpiLevel, AcpiError> {
    // SAFETY: per this function's contract.
    unsafe {
        check(sys::AcpiInitializeSubsystem())?;
        // No caller storage: ACPICA allocates the table list and grows it.
        check(sys::AcpiInitializeTables(ptr::null_mut(), INITIAL_TABLE_COUNT, 1))?;
        // Parse the DSDT and SSDTs into the namespace. Broken AML lands
        // here; the tables stay usable, so this degrades instead of failing.
        match check(sys::AcpiLoadTables()) {
            Ok(()) => Ok(AcpiLevel::Namespace),
            Err(_) => Ok(AcpiLevel::Tables),
        }
    }
}
