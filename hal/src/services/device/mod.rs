//! The device model: what the firmware says is in the machine, and which
//! driver took it.
//!
//! [`init`] reads the firmware description once, makes a [`Device`] for each
//! thing it describes, and probes drivers against them (see [`probe`]).
//!
//! Devices are firmware-neutral only where they truly are: a name, a list of
//! ids, and the register windows and interrupt lines they use. Anything else
//! is firmware specific and stays that way. A driver that needs a device tree
//! property asks for [`Device::fdt`], one that needs an ACPI method asks for
//! [`Device::acpi`], and both are `None` when the device came from the other
//! kind of firmware.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::driver_traits::driver::ProbeError;
pub use crate::driver_traits::interrupt_controller::IrqSpec;
pub use fdt::FdtLink;
use crate::services::addr::PhysRange;
use crate::services::boot_info::{HardwareDescription, HardwareError};
use crate::services::mmio::Mmio;
use crate::services::mmu;
use crate::services::registry::AppendList;
use crate::services::sync::SpinLock;

#[cfg(target_arch = "x86_64")]
mod acpi;
mod console;
mod fdt;
mod probe;

pub use probe::Report;

/// Identifies a device for as long as the kernel runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(u32);

impl DeviceId {
    pub const fn number(self) -> u32 {
        self.0
    }
}

/// A window of x86 I/O ports, as the firmware described it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoPorts {
    base: u16,
    count: u16,
}

impl IoPorts {
    // Only ACPI describes I/O ports, and only x86 has ACPI.
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    pub(crate) const fn new(base: u16, count: u16) -> Self {
        Self { base, count }
    }

    pub const fn base(self) -> u16 {
        self.base
    }

    pub const fn count(self) -> u16 {
        self.count
    }
}

/// One thing a device uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// A memory-mapped register window, in CPU physical addresses.
    Mem(PhysRange),
    /// A window of x86 I/O ports.
    Io(IoPorts),
    /// An interrupt line, already in the controller's numbering (ACPI's
    /// global system interrupts). A device tree names its interrupts in cells
    /// only the controller can read, so those are resolved on request by
    /// [`Device::irq`] instead.
    Irq(IrqSpec),
}

/// A register window that has been mapped as device memory. Only
/// [`Device::mem`] makes one, which is what makes [`DeviceMem::mmio`] safe.
#[derive(Clone, Copy, Debug)]
pub struct DeviceMem {
    range: PhysRange,
}

impl DeviceMem {
    pub const fn range(&self) -> PhysRange {
        self.range
    }

    pub const fn base(&self) -> usize {
        self.range.start().as_usize()
    }

    pub const fn len(&self) -> usize {
        self.range.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    /// Register access to the window.
    pub fn mmio(&self) -> Mmio {
        // SAFETY: the window came from the firmware description and was
        // mapped as device memory when this value was made.
        unsafe { Mmio::new(self.base()) }
    }
}

/// A device tree node, with what a walk down from the root learned about it.
#[derive(Clone, Copy)]
pub struct FdtOrigin {
    node: crate::services::dtb::Node<'static>,
    /// The phandle in effect as `interrupt-parent` for this node.
    interrupt_parent: Option<u32>,
}

/// Where a device came from, with that firmware's own handle for it.
#[derive(Clone, Copy)]
pub enum Origin {
    Fdt(FdtOrigin),
    /// An entry in the ACPI namespace.
    #[cfg(target_arch = "x86_64")]
    Acpi(crate::services::acpi::Device),
    /// Hardware ACPI describes in a table instead of the namespace (the
    /// local and I/O APICs in the MADT, power management in the FADT). The
    /// device's only id is the table's signature.
    #[cfg(target_arch = "x86_64")]
    AcpiTable,
    /// Announced by the driver of another device (`by`), because that driver
    /// found something firmware does not describe: the RAM framebuffer
    /// inside QEMU's `fw_cfg`, or an SoC block the board's tree leaves out.
    Spawned { by: &'static Device },
}

/// How far a device is on its way to having a driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Not probed yet.
    Pending,
    /// No driver in this kernel matches it.
    NoDriver,
    /// This driver needs something that was not there; tried again later.
    Deferred(&'static str),
    /// Every matching driver failed; this is the last one's error.
    Failed(&'static str, ProbeError),
    /// This driver took it.
    Bound(&'static str),
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::Pending => f.write_str("pending"),
            State::NoDriver => f.write_str("no driver"),
            State::Deferred(driver) => write!(f, "{driver}: waiting for a dependency"),
            State::Failed(driver, error) => write!(f, "{driver}: {error}"),
            State::Bound(driver) => write!(f, "bound to {driver}"),
        }
    }
}

/// A device the firmware describes.
pub struct Device {
    id: DeviceId,
    name: String,
    ids: Vec<String>,
    origin: Origin,
    resources: SpinLock<Option<&'static [Resource]>>,
    state: SpinLock<State>,
}

impl Device {
    fn new(name: String, ids: Vec<String>, origin: Origin, resources: Option<Vec<Resource>>) -> Device {
        static NEXT_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
        let id = DeviceId(NEXT_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed));
        Device {
            id,
            name,
            ids,
            origin,
            resources: SpinLock::new(resources.map(|list| &*Box::leak(list.into_boxed_slice()))),
            state: SpinLock::new(State::Pending),
        }
    }

    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// A name for logs: the node name, or the `_HID`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the device calls itself, most specific first: the device tree's
    /// `compatible` list, or ACPI's `_HID` followed by its `_CID`s.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    /// The device tree node, if the device came from one.
    pub fn fdt(&self) -> Option<&crate::services::dtb::Node<'static>> {
        match &self.origin {
            Origin::Fdt(origin) => Some(&origin.node),
            #[cfg(target_arch = "x86_64")]
            Origin::Acpi(_) | Origin::AcpiTable => None,
            Origin::Spawned { .. } => None,
        }
    }

    /// The ACPI namespace entry, if the device came from there.
    #[cfg(target_arch = "x86_64")]
    pub fn acpi(&self) -> Option<&crate::services::acpi::Device> {
        match &self.origin {
            Origin::Acpi(device) => Some(device),
            Origin::Fdt(_) | Origin::AcpiTable | Origin::Spawned { .. } => None,
        }
    }

    pub fn state(&self) -> State {
        *self.state.lock()
    }

    fn set_state(&self, state: State) {
        *self.state.lock() = state;
    }

    /// Everything the device uses. For ACPI this evaluates `_CRS` the first
    /// time it is asked, so devices no driver wants never run any AML.
    pub fn resources(&self) -> Result<&'static [Resource], ProbeError> {
        let mut slot = self.resources.lock();
        if let Some(list) = *slot {
            return Ok(list);
        }

        let loaded = match &self.origin {
            // Device tree resources are read when the device is made.
            Origin::Fdt(_) => Vec::new(),
            #[cfg(target_arch = "x86_64")]
            Origin::Acpi(device) => acpi::load_resources(device).map_err(|_| ProbeError::Firmware)?,
            // Tables describe their hardware to the driver that knows the
            // table, not as resources.
            #[cfg(target_arch = "x86_64")]
            Origin::AcpiTable => Vec::new(),
            // A spawned device is given its resources when it is made.
            Origin::Spawned { .. } => Vec::new(),
        };
        let list: &'static [Resource] = Box::leak(loaded.into_boxed_slice());
        *slot = Some(list);
        Ok(list)
    }

    /// The `index`th register window (counting only windows, in the order the
    /// firmware lists them), mapped as device memory.
    pub fn mem(&self, index: usize) -> Result<DeviceMem, ProbeError> {
        let range = self
            .resources()?
            .iter()
            .filter_map(|resource| match resource {
                Resource::Mem(range) => Some(*range),
                _ => None,
            })
            .nth(index)
            .ok_or(ProbeError::MissingResource)?;

        mmu::map_device(range).map_err(|_| ProbeError::MappingFailed)?;
        Ok(DeviceMem { range })
    }

    /// The register window that starts at physical address `base`, mapped as
    /// device memory, for drivers that already know the address from a table
    /// of their own and want the device to vouch for it.
    pub fn mem_at(&self, base: usize) -> Result<DeviceMem, ProbeError> {
        let range = self
            .resources()?
            .iter()
            .find_map(|resource| match resource {
                Resource::Mem(range) if range.start().as_usize() == base => Some(*range),
                _ => None,
            })
            .ok_or(ProbeError::MissingResource)?;

        mmu::map_device(range).map_err(|_| ProbeError::MappingFailed)?;
        Ok(DeviceMem { range })
    }

    /// The `index`th I/O port window.
    pub fn io(&self, index: usize) -> Result<IoPorts, ProbeError> {
        self.resources()?
            .iter()
            .filter_map(|resource| match resource {
                Resource::Io(ports) => Some(*ports),
                _ => None,
            })
            .nth(index)
            .ok_or(ProbeError::MissingResource)
    }

    /// The device whose driver announced this one with [`Device::spawn`], if
    /// that is where it came from.
    pub fn parent(&self) -> Option<&'static Device> {
        match &self.origin {
            Origin::Spawned { by } => Some(by),
            _ => None,
        }
    }

    /// Announces a device that no firmware table describes and that this
    /// device's driver found: `ids` are what drivers match it by
    /// ([`Match::Spawned`](crate::driver_traits::driver::Match::Spawned)), and
    /// `resources` what it uses, if anything.
    pub fn spawn(&'static self, name: &str, ids: &[&str], resources: &[Resource]) -> &'static Device {
        register(Device::new(
            String::from(name),
            ids.iter().map(|id| String::from(*id)).collect(),
            Origin::Spawned { by: self },
            Some(resources.to_vec()),
        ))
    }

    /// Like [`spawn`](Self::spawn), unless a device that already has `ids[0]`
    /// exists, whether firmware described it or another driver spawned it:
    /// that one is returned instead. For a block several consumers need, which
    /// the first one to be probed announces and the rest find.
    ///
    /// Probing is single-threaded, so the check and the spawn cannot race.
    pub fn spawn_once(&'static self, name: &str, ids: &[&str], resources: &[Resource]) -> &'static Device {
        let existing = ids
            .first()
            .and_then(|wanted| devices().find(|device| device.ids().iter().any(|id| id == wanted)));
        match existing {
            Some(existing) => existing,
            None => self.spawn(name, ids, resources),
        }
    }

    /// The device that `link` says provides what this device calls `name`
    /// (`clock-names`, say): the device tree way of finding a provider.
    pub fn fdt_link(&self, link: &FdtLink, name: &str) -> Result<&'static Device, ProbeError> {
        #[allow(irrefutable_let_patterns)]
        if let Origin::Fdt(origin) = &self.origin {
            return fdt::link(origin, link, name);
        }
        Err(ProbeError::MissingResource)
    }

    /// The device named by the `index`th entry of this device's `_DEP`: the
    /// ACPI way of saying "I need that device", which makes it the one to ask
    /// for a provider.
    ///
    /// [`ProbeError::MissingResource`] if there is no such entry (or the
    /// device is not from the namespace), [`ProbeError::NotPresent`] if the
    /// device it names is one this kernel made no device for (it has no
    /// `_HID`).
    #[cfg(target_arch = "x86_64")]
    pub fn acpi_dependency(&self, index: usize) -> Result<&'static Device, ProbeError> {
        let Origin::Acpi(device) = &self.origin else {
            return Err(ProbeError::MissingResource);
        };
        let dependencies = match device.dependencies() {
            Ok(list) => list,
            Err(crate::services::acpi::AcpiError::NotFound) => return Err(ProbeError::MissingResource),
            Err(_) => return Err(ProbeError::Firmware),
        };
        let wanted = dependencies.get(index).ok_or(ProbeError::MissingResource)?;
        devices()
            .find(|candidate| matches!(candidate.origin(), Origin::Acpi(device) if device == wanted))
            .ok_or(ProbeError::NotPresent)
    }

    /// The `index`th interrupt line, in the numbering of the controller that
    /// serves it.
    ///
    /// ACPI already numbers its interrupts that way. A device tree writes them
    /// as cells only the interrupt controller can read, so this asks the
    /// controller's driver to decode them: [`ProbeError::Defer`] means that
    /// driver has not bound yet.
    pub fn irq(&self, index: usize) -> Result<IrqSpec, ProbeError> {
        // Only x86 has another kind of origin; ACPI's interrupts are in its
        // resources, below.
        #[allow(irrefutable_let_patterns)]
        if let Origin::Fdt(origin) = &self.origin {
            return fdt::irq(origin, index);
        }

        self.resources()?
            .iter()
            .filter_map(|resource| match resource {
                Resource::Irq(spec) => Some(*spec),
                _ => None,
            })
            .nth(index)
            .ok_or(ProbeError::MissingResource)
    }
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device").field("id", &self.id).field("name", &self.name).field("ids", &self.ids).finish()
    }
}

static DEVICES: AppendList<&'static Device> = AppendList::new();

/// Every device found at boot, in the order the firmware lists them.
pub fn devices() -> impl Iterator<Item = &'static Device> {
    DEVICES.iter().copied()
}

/// Adds a device to the registry.
fn register(device: Device) -> &'static Device {
    let device: &'static Device = Box::leak(Box::new(device));
    DEVICES.push(device);
    device
}

/// Reads the firmware description into devices and probes a driver for each.
///
/// Needs the heap. Call once, after [`HardwareDescription::init`]; the
/// architecture does not matter to the caller.
pub fn init(hardware: &HardwareDescription<'static>) -> Result<Report, HardwareError> {
    match hardware {
        HardwareDescription::Fdt(dtb) => fdt::enumerate(dtb),
        #[cfg(target_arch = "x86_64")]
        HardwareDescription::Acpi(root) => acpi::enumerate(root)?,
        #[cfg(not(target_arch = "x86_64"))]
        HardwareDescription::Acpi(_) => {
            return Err(HardwareError::Acpi(crate::services::acpi::AcpiError::Unsupported));
        }
    }
    let report = probe::probe_all();
    console::select(hardware);
    Ok(report)
}
