//! The embedded controller (EC):
//!
//! # TODO
//!
//! * This driver is polling only.
//!
//! * Burst mode (commands `0x82` / `0x83`)
//! * `_GPE` as a package (a GPE on a GPIO block device)
//! * A full work queue drops the event.
//! * ECDT ports that are not plain I/O
//! * More than one EC. Only the first `PNP0C09` is used.


use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use acpica_sys as sys;

use super::namespace::{Device, Namespace};
use super::raw::check;
use super::resource::Resource;
use super::table::{Pod, Table, TableKind, read_pod};
use super::AcpiError;
use crate::services::klog::{self, Level, Source};
use crate::services::sync::{IrqSpinLock, StaticCell};
use crate::services::timer;
use crate::services::work;
use crate::x86_64::port;

// SAFETY: plain-old-data C struct (a generic address: integers only).
unsafe impl Pod for sys::ACPI_GENERIC_ADDRESS {}

/// ACPI's address space id for the embedded controller (`ACPI_ADR_SPACE_EC`).
const ADDRESS_SPACE_EC: u8 = 3;
/// `SpaceId` of a generic address that is plain I/O ports.
const GAS_SYSTEM_IO: u8 = 1;

/// Handler function codes (`ACPI_READ`, `ACPI_WRITE`); the low bit tells them apart.
const FUNCTION_WRITE: u32 = 1;

// Status register bits.
const STATUS_OBF: u8 = 0x01;
const STATUS_IBF: u8 = 0x02;
const STATUS_SCI_EVT: u8 = 0x20;

// Commands.
const COMMAND_READ: u8 = 0x80;
const COMMAND_WRITE: u8 = 0x81;
const COMMAND_QUERY: u8 = 0x84;

/// `ACPI_GPE_EDGE_TRIGGERED`: ACPICA clears the status before our handler runs.
const GPE_EDGE_TRIGGERED: u32 = 0x00;

/// Most events handled in one pass
const MAX_EVENTS_PER_PASS: usize = 64;

/// How long to wait for the EC to do one step, in nanoseconds. Same as Linux's default.
const WAIT_LIMIT_NS: u64 = 500_000_000;

/// The EC has 256 registers.
const REGISTER_COUNT: u64 = 0x100;

/// Only the first few failures are logged, so a dead EC cannot flood the log.
const MAX_LOGGED_FAILURES: u32 = 8;

/// Work item context: a pass run because the GPE fired (so the GPE must be given back)...
const WORK_FROM_GPE: usize = 1;
/// ...or the one-off pass at start-up (the GPE was never taken).
const WORK_AT_START: usize = 0;

struct EcdtKind;

impl TableKind for EcdtKind {
    const SIGNATURE: &'static [u8; 5] = sys::ACPI_SIG_ECDT;
}

/// What the ECDT says about the EC, if there is one.
struct Ecdt {
    command: Option<u16>,
    data: Option<u16>,
    gpe: u8,
    path: String,
}

impl Ecdt {
    fn get() -> Option<Self> {
        let table = Table::<EcdtKind>::get(1).ok()?;
        let bytes = table.bytes();
        // The table struct ends in a flexible id, so it is not `Pod`; read
        // its fixed fields one by one.
        let control: sys::ACPI_GENERIC_ADDRESS = read_pod(bytes, offset_of!(sys::ACPI_TABLE_ECDT, Control))?;
        let data: sys::ACPI_GENERIC_ADDRESS = read_pod(bytes, offset_of!(sys::ACPI_TABLE_ECDT, Data))?;
        let gpe = *bytes.get(offset_of!(sys::ACPI_TABLE_ECDT, Gpe))?;

        let port = |address: sys::ACPI_GENERIC_ADDRESS| {
            (address.SpaceId == GAS_SYSTEM_IO).then(|| u16::try_from(address.Address).ok()).flatten()
        };
        // The id is a NUL-terminated namespace path right after the fixed part.
        let id = bytes.get(offset_of!(sys::ACPI_TABLE_ECDT, Id)..)?;
        let id = id.split(|byte| *byte == 0).next()?;

        Some(Self {
            command: port(control),
            data: port(data),
            gpe,
            path: String::from(core::str::from_utf8(id).ok()?),
        })
    }
}

/// The embedded controller, ready to be read and written.
pub struct EmbeddedController {
    device: Device,
    command: u16,
    data: u16,
    gpe: Option<u32>,
    lock: IrqSpinLock<()>,
}

impl EmbeddedController {
    /// The command/status port (usually 0x66).
    pub fn command_port(&self) -> u16 {
        self.command
    }

    /// The data port (usually 0x62).
    pub fn data_port(&self) -> u16 {
        self.data
    }

    /// The GPE the EC raises its events on.
    pub fn gpe(&self) -> Option<u32> {
        self.gpe
    }

    /// Reads one of the EC's 256 registers.
    pub fn read(&self, register: u8) -> Result<u8, AcpiError> {
        let _serialised = self.lock.lock();
        self.flush_output();
        self.wait_input_empty()?;
        self.send_command(COMMAND_READ);
        self.wait_input_empty()?;
        self.send_data(register);
        self.wait_output_full()?;
        Ok(self.receive_data())
    }

    /// Writes one of the EC's 256 registers.
    pub fn write(&self, register: u8, value: u8) -> Result<(), AcpiError> {
        let _serialised = self.lock.lock();
        self.flush_output();
        self.wait_input_empty()?;
        self.send_command(COMMAND_WRITE);
        self.wait_input_empty()?;
        self.send_data(register);
        self.wait_input_empty()?;
        self.send_data(value);
        // The EC has taken the value once it empties the input buffer.
        self.wait_input_empty()
    }

    /// Asks the EC which event it wants attention for. `0` means none.
    fn query(&self) -> Result<u8, AcpiError> {
        let _serialised = self.lock.lock();
        self.flush_output();
        self.wait_input_empty()?;
        self.send_command(COMMAND_QUERY);
        self.wait_output_full()?;
        Ok(self.receive_data())
    }

    /// Handles every event the EC has queued.
    fn service_events(&self) {
        for _ in 0..MAX_EVENTS_PER_PASS {
            if self.status() & STATUS_SCI_EVT == 0 {
                break;
            }
            let event = match self.query() {
                Ok(0) => break,
                Ok(event) => event,
                Err(err) => {
                    klog::log(Source::Ec, Level::Warn, format_args!("event query failed: {err}"));
                    break;
                }
            };

            // `_Qxx`, with xx the event number in uppercase hex.
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            let name = [b'_', b'Q', HEX[usize::from(event >> 4)], HEX[usize::from(event & 0xF)]];
            let Ok(name) = core::str::from_utf8(&name) else { break };
            match self.device.evaluate(name) {
                Ok(()) => klog::log(Source::Ec, Level::Info, format_args!("event {event:#04x}: ran {name}")),
                Err(AcpiError::NotFound) => {
                    klog::log(Source::Ec, Level::Warn, format_args!("event {event:#04x}: firmware has no {name}"))
                }
                Err(err) => {
                    klog::log(Source::Ec, Level::Warn, format_args!("event {event:#04x}: {name} failed: {err}"))
                }
            }
        }
    }

    fn status(&self) -> u8 {
        // SAFETY: the EC's own status port, as firmware described it.
        unsafe { port::inb(self.command) }
    }

    fn send_command(&self, command: u8) {
        // SAFETY: as above.
        unsafe { port::outb(self.command, command) }
    }

    fn send_data(&self, byte: u8) {
        // SAFETY: the EC's data port.
        unsafe { port::outb(self.data, byte) }
    }

    fn receive_data(&self) -> u8 {
        // SAFETY: as above.
        unsafe { port::inb(self.data) }
    }

    /// A byte left in the output buffer (an unclaimed event answer, say)
    /// would be mistaken for our reply, so it goes first.
    fn flush_output(&self) {
        for _ in 0..8 {
            if self.status() & STATUS_OBF == 0 {
                break;
            }
            let _ = self.receive_data();
        }
    }

    fn wait_input_empty(&self) -> Result<(), AcpiError> {
        self.wait(|status| status & STATUS_IBF == 0)
    }

    fn wait_output_full(&self) -> Result<(), AcpiError> {
        self.wait(|status| status & STATUS_OBF != 0)
    }

    /// Polls the status register until `done` likes it, or time runs out.
    fn wait(&self, done: impl Fn(u8) -> bool) -> Result<(), AcpiError> {
        let start = timer::now_ns();
        loop {
            if done(self.status()) {
                return Ok(());
            }
            if timer::now_ns().wrapping_sub(start) > WAIT_LIMIT_NS {
                return Err(AcpiError::Timeout);
            }
            core::hint::spin_loop();
        }
    }
}

static EC: StaticCell<EmbeddedController> = StaticCell::uninit();

/// The embedded controller, if one was found and its handler installed.
pub fn get() -> Option<&'static EmbeddedController> {
    EC.get()
}

static FAILURES: AtomicU32 = AtomicU32::new(0);

/// Set once the GPE handler is installed, so `start_events` knows there is something to enable.
static EVENTS_INSTALLED: AtomicBool = AtomicBool::new(false);

/// ACPICA's address-space handler for `EmbeddedControl` regions.
///
/// `address` is the EC register, `bit_width` how much to move (firmware
/// declares multi-byte fields; they go byte by byte, lowest register first,
/// as the spec says), `value` the data in or out.
///
/// # Safety
/// Called by ACPICA with the `context` we registered (the `&'static
/// EmbeddedController`) and a `value` valid to read and write.
unsafe extern "C" fn space_handler(
    function: u32,
    address: u64,
    bit_width: u32,
    value: *mut u64,
    context: *mut c_void,
    _region_context: *mut c_void,
) -> sys::ACPI_STATUS {
    if value.is_null() || context.is_null() || bit_width == 0 || bit_width % 8 != 0 || bit_width > 64 {
        return sys::AE_BAD_PARAMETER;
    }
    let bytes = u64::from(bit_width / 8);
    if address.checked_add(bytes).is_none_or(|end| end > REGISTER_COUNT) {
        return sys::AE_BAD_ADDRESS;
    }

    // SAFETY: per this function's contract.
    let ec = unsafe { &*context.cast::<EmbeddedController>() };

    let result = (|| -> Result<(), AcpiError> {
        if function & 1 == FUNCTION_WRITE {
            // SAFETY: `value` is valid per the contract.
            let data = unsafe { *value };
            for index in 0..bytes {
                ec.write((address + index) as u8, (data >> (8 * index)) as u8)?;
            }
        } else {
            let mut data = 0u64;
            for index in 0..bytes {
                data |= u64::from(ec.read((address + index) as u8)?) << (8 * index);
            }
            // SAFETY: `value` is valid per the contract.
            unsafe { *value = data };
        }
        Ok(())
    })();

    match result {
        Ok(()) => sys::AE_OK,
        Err(err) => {
            if FAILURES.fetch_add(1, Ordering::Relaxed) < MAX_LOGGED_FAILURES {
                klog::log(
                    Source::Ec,
                    Level::Warn,
                    format_args!("register {address:#04x} ({bit_width} bits) access failed: {err}"),
                );
            }
            if err == AcpiError::Timeout { sys::AE_TIME } else { sys::AE_ERROR }
        }
    }
}

/// ACPICA calls this, in the interrupt, when the EC's GPE fires. ACPICA has
/// already disabled the GPE; it stays that way until [`events_work`] gives it
/// back, so the handler only has to queue that.
unsafe extern "C" fn gpe_handler(_device: sys::ACPI_HANDLE, _gpe: u32, _context: *mut c_void) -> u32 {
    if work::submit(events_work, WORK_FROM_GPE as *mut ()).is_err() {
        klog::log(Source::Ec, Level::Error, format_args!("Work queue full, EC events are off until reboot"));
    }
    sys::ACPI_INTERRUPT_HANDLED
}

/// The work item behind the GPE: drains the EC's events.
fn events_work(context: *mut ()) {
    let Some(ec) = EC.get() else { return };
    ec.service_events();

    if context as usize == WORK_FROM_GPE {
        if let Some(gpe) = ec.gpe {
            // SAFETY: a GPE on the FADT blocks (null device) that ACPICA handed us.
            if let Err(err) = check(unsafe { sys::AcpiFinishGpe(core::ptr::null_mut(), gpe) }) {
                klog::log(Source::Ec, Level::Error, format_args!("Could not re-enable the EC's GPE {gpe:#x}: {err}"));
            }
        }
    }
}

/// Turns on the EC's GPE and queues one pass for events that were already
/// waiting. Call it once the namespace is initialized (`_REG` has run).
pub(super) fn start_events() {
    if !EVENTS_INSTALLED.load(Ordering::Acquire) {
        return;
    }
    let Some(ec) = EC.get() else { return };
    let Some(gpe) = ec.gpe else { return };

    // SAFETY: a GPE on the FADT blocks (null device) with a handler installed.
    if let Err(err) = check(unsafe { sys::AcpiEnableGpe(core::ptr::null_mut(), gpe) }) {
        klog::log(Source::Ec, Level::Warn, format_args!("Could not enable the EC's GPE {gpe:#x}: {err}"));
        return;
    }
    if work::submit(events_work, WORK_AT_START as *mut ()).is_err() {
        klog::log(Source::Ec, Level::Warn, format_args!("Work queue full, skipping the first EC event pass"));
    }
}

/// Where the EC's ports and device came from.
enum Found {
    /// The namespace's `PNP0C09` device and its `_CRS`.
    Namespace { device: Device, command: u16, data: u16 },
    /// The ECDT (and the device at its path).
    Ecdt { device: Device, command: u16, data: u16, gpe: u8 },
}

/// The first two I/O ports of a `_CRS`: data first, then command/status.
fn crs_ports(device: &Device) -> Option<(u16, u16)> {
    let ports: Vec<u16> = device
        .resources()
        .ok()?
        .into_iter()
        .filter_map(|resource| match resource {
            Resource::Io { min, .. } => Some(min),
            Resource::FixedIo { base, .. } => Some(base),
            _ => None,
        })
        .collect();
    match ports.as_slice() {
        [data, command, ..] => Some((*data, *command)),
        _ => None,
    }
}

fn find(namespace: &Namespace<'_>) -> Option<Found> {
    let device = namespace.devices("PNP0C09").ok().and_then(|devices| devices.first().copied());

    if let Some(device) = device {
        if let Some((data, command)) = crs_ports(&device) {
            return Some(Found::Namespace { device, command, data });
        }
        klog::log(Source::Ec, Level::Warn, format_args!("PNP0C09 has no usable _CRS, trying the ECDT"));
    }

    let ecdt = Ecdt::get()?;
    let (Some(command), Some(data)) = (ecdt.command, ecdt.data) else {
        klog::log(Source::Ec, Level::Warn, format_args!("ECDT does not describe I/O ports, ignoring it"));
        return None;
    };
    let device = device.or_else(|| namespace.device_at(&ecdt.path).ok())?;
    Some(Found::Ecdt { device, command, data, gpe: ecdt.gpe })
}

/// Finds the EC and registers the address-space handler for it.
pub(super) fn init(namespace: &Namespace<'_>) {
    let Some(found) = find(namespace) else {
        klog::log(Source::Ec, Level::Info, format_args!("No embedded controller found"));
        return;
    };
    let (device, command, data, gpe, origin) = match found {
        Found::Namespace { device, command, data } => {
            // A plain integer `_GPE` is a GPE on the FADT blocks. A package
            // would be a GPE on a GPIO block device, which is not handled.
            let gpe = device.evaluate_integer("_GPE").ok().and_then(|gpe| u32::try_from(gpe).ok());
            (device, command, data, gpe, "_CRS")
        },
        Found::Ecdt { device, command, data, gpe } => (device, command, data, Some(u32::from(gpe)), "ECDT"),
    };

    let ec = EC.get_or_init(|| EmbeddedController { device, command, data, gpe, lock: IrqSpinLock::new(()) });

    // SAFETY: the device handle is a valid namespace node; `space_handler`
    // matches the handler signature and only uses the context we pass,
    // which is `'static`.
    let installed = check(unsafe {
        sys::AcpiInstallAddressSpaceHandler(
            device.handle.as_ptr(),
            ADDRESS_SPACE_EC,
            Some(space_handler),
            None,
            core::ptr::from_ref(ec).cast_mut().cast(),
        )
    });
    match installed {
        Ok(()) => klog::log(
            Source::Ec,
            Level::Info,
            format_args!("Embedded controller: command port {command:#x}, data port {data:#x} (from {origin})"),
        ),
        Err(err) => klog::log(Source::Ec, Level::Warn, format_args!("Could not install the EC handler: {err}")),
    }

    let Some(gpe) = gpe else {
        klog::log(Source::Ec, Level::Info, format_args!("no plain _GPE number, so no EC events"));
        return;
    };
    // SAFETY: a GPE on the FADT blocks (null device); the handler matches the
    // signature ACPICA wants and uses no context.
    let handler = check(unsafe {
        sys::AcpiInstallGpeHandler(core::ptr::null_mut(), gpe, GPE_EDGE_TRIGGERED, Some(gpe_handler), core::ptr::null_mut())
    });
    match handler {
        Ok(()) => {
            EVENTS_INSTALLED.store(true, Ordering::Release);
            klog::log(Source::Ec, Level::Info, format_args!("Event GPE {gpe:#x}"));
        }
        Err(err) => klog::log(Source::Ec, Level::Warn, format_args!("Could not install the EC's GPE handler: {err}")),
    }
}

const _: () = assert!(size_of::<sys::ACPI_GENERIC_ADDRESS>() == 12);
