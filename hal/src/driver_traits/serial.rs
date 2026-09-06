//! Interfaces shared by serial devices.

use crate::services::dtb::Dtb;
use crate::services::sync::StaticCell;

static ACTIVE_SERIAL: StaticCell<&'static dyn Serial> = StaticCell::uninit();

/// A polling serial device.
///
/// Interrupt and configuration support belong in the driver once the kernel has
/// the infrastructure to use them.
pub trait Serial: Sync {
    /// Blocks until `byte` has been transmitted.
    fn write_byte(&self, byte: u8);

    /// Blocks until a byte has been received.
    fn read_byte(&self) -> u8;

    /// Writes a UTF-8 string without allocating.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// uart.write_str("booting...\n");
    /// ```
    fn write_str(&self, value: &str) {
        for byte in value.bytes() {
            self.write_byte(byte);
        }
    }
}

#[repr(C)]
pub struct SerialDriver {
    /// Human-readable identifier used when inspecting the driver table.
    pub name: &'static str,
    /// Device-tree `compatible` value handled by this driver.
    pub compatible: &'static str,
    /// Initializes the driver for the supplied mapped MMIO base address.
    pub init: fn(base: usize, node: &Dtb) -> &'static dyn Serial,
}

/// Failure to locate a serial driver for the supplied device tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// No registered serial driver matched a node in the supplied tree.
    NoCompatibleSerialDriver,
    MappingFailed
}

/// Registers a serial driver in the linker-retained discovery table.
///
/// The macro is for HAL driver implementations. Its initializer must return a
/// static serial-device reference and its compatible string must match the
/// platform device tree.
///
/// # Examples
///
/// ```ignore
/// register_serial_driver!(PL011_UART, "arm,pl011", init_pl011_uart);
/// ```
#[macro_export]
macro_rules! register_serial_driver {
    ($ident:ident, $compat:expr, $init_fn:path) => {
        #[used]
        #[unsafe(link_section = ".drivers.serial")]
        static $ident: $crate::driver_traits::serial::SerialDriver = $crate::driver_traits::serial::SerialDriver {
            name: stringify!($ident),
            compatible: $compat,
            init: $init_fn,
        };
    };
}

unsafe extern "C" {
    static __start_serial_drivers: u8;
    static __stop_serial_drivers: u8;
}

fn serial_drivers() -> &'static [SerialDriver] {
    // SAFETY: the linker script emits these symbols around a contiguous sequence
    // of `SerialDriver` values retained by `KEEP`.
    unsafe {
        let start = core::ptr::addr_of!(__start_serial_drivers) as *const SerialDriver;
        let stop = core::ptr::addr_of!(__stop_serial_drivers) as *const SerialDriver;
        let count = (stop as usize - start as usize) / core::mem::size_of::<SerialDriver>();
        core::slice::from_raw_parts(start, count)
    }
}

/// Finds and initializes the first registered serial driver present in `tree`.
///
/// # Errors
///
/// Returns [`crate::ProbeError::NoCompatibleSerialDriver`] when no registered driver's
/// compatible string occurs in `tree`.
///
/// # Examples
///
/// ```ignore
/// let serial = hal::probe_serial(&tree)?;
/// serial.write_str("hello\n");
/// # Ok::<(), hal::ProbeError>(())
/// ```
pub fn probe_serial(tree: &Dtb) -> Result<&'static dyn Serial, crate::ProbeError> {
    for driver in serial_drivers() {
        if let Some((base, size)) = tree.find_compatible_address(driver.compatible) {
            crate::services::mmu::map_device(base, size).map_err(|_| crate::ProbeError::MappingFailed)?;

            return Ok(*ACTIVE_SERIAL.get_or_init(|| (driver.init)(base, tree)));
        }
    }

    Err(crate::ProbeError::NoCompatibleSerialDriver)
}

/// Returns the serial device selected by [`crate::probe_serial`], if boot discovery
/// completed successfully.
///
/// Exception and panic reporting can use this without coupling the kernel to a
/// particular UART implementation.
pub fn active_serial() -> Option<&'static dyn Serial> {
    ACTIVE_SERIAL.get().copied()
}

