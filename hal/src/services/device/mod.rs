//! The deice model for drivers

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::driver_traits::driver::ProbeError;
pub use crate::driver_traits::interrupt_controller::IrqSpec;
use crate::services::addr::PhysRange;
use crate::services::boot_info::{HardwareDescription, HardwareError};
use crate::services::mmio::Mmio;
use crate::services::mmu;
use crate::services::registry::AppendList;
use crate::services::sync::SpinLock;

#[cfg(target_arch = "x86_64")]
mod acpi;
mod fdt;
mod probe;

pub use probe::Report;

/// Device Identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(u32);

impl DeviceId {
    pub const fn number(self) -> u32 {
        self.0
    }
}

/// A window of x86 I/O ports
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoPorts {
    base: u16,
    count: u16,
}

impl IoPorts {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// DMA window
    Mem(PhysRange),
    /// IO ports window
    Io(IoPorts),
    /// An interrupt line, controllers numbersing
    Irq(IrqSpec),
}

/// A register window, mapped as device memory
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

/// A device tree node
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
    /// Hardware ACPI table
    #[cfg(target_arch = "x86_64")]
    AcpiTable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Not probed yet.
    Pending,
    /// No driver
    NoDriver,
    /// waiting for dependency
    Deferred(&'static str),
    /// Every matching driver failed, last one's error attached
    Failed(&'static str, ProbeError),
    /// bound to driver
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

    /// the node name, or the `_HID`
    pub fn name(&self) -> &str {
        &self.name
    }

    /// DT's `compatible` list, or ACPI `_HID`, followed by `_CID`
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
        }
    }

    /// The ACPI namespace entry, if the device came from there.
    #[cfg(target_arch = "x86_64")]
    pub fn acpi(&self) -> Option<&crate::services::acpi::Device> {
        match &self.origin {
            Origin::Acpi(device) => Some(device),
            Origin::Fdt(_) | Origin::AcpiTable => None,
        }
    }

    pub fn state(&self) -> State {
        *self.state.lock()
    }

    fn set_state(&self, state: State) {
        *self.state.lock() = state;
    }

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
        };
        let list: &'static [Resource] = Box::leak(loaded.into_boxed_slice());
        *slot = Some(list);
        Ok(list)
    }

    /// The `index`th register window
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

    /// The `index`th interrupt line, in the numbering of the controller that
    /// serves it.
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
    Ok(probe::probe_all())
}
