//! Device-tree based discovery of the small set of drivers supported at boot.

use crate::{Serial, sync::StaticCell};

static ACTIVE_SERIAL: StaticCell<&'static dyn Serial> = StaticCell::uninit();

/// The portion of a device tree needed by HAL driver discovery.
///
/// Keeping this narrow leaves DTB parsing and policy in the kernel, where they
/// can evolve independently from device drivers.
pub trait DeviceTree {
    /// Returns the mapped register base for the first matching compatible node.
    ///
    /// Implementations must return only addresses valid for MMIO access.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let uart_base = tree.compatible_address("arm,pl011");
    /// ```
    fn compatible_address(&self, compatible: &str) -> Option<(usize, usize)>;

    type Node<'a>: DeviceTreeNode where Self: 'a;

    fn compatible_node(&self, compatible: &str) -> Option<Self::Node<'_>>;
}

pub trait DeviceTreeNode {
    fn get_prop_u32(&self, prop_name: &str) -> Option<u32>;
}

/// A serial driver that can be selected from a device tree compatible string.
#[repr(C)]
pub struct SerialDriver {
    /// Human-readable identifier used when inspecting the driver table.
    pub name: &'static str,
    /// Device-tree `compatible` value handled by this driver.
    pub compatible: &'static str,
    /// Initializes the driver for the supplied mapped MMIO base address.
    pub init: fn(base: usize, node: Option<&dyn DeviceTreeNode>) -> &'static dyn Serial,
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
        static $ident: $crate::device_tree::SerialDriver = $crate::device_tree::SerialDriver {
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
/// Returns [`ProbeError::NoCompatibleSerialDriver`] when no registered driver's
/// compatible string occurs in `tree`.
///
/// # Examples
///
/// ```ignore
/// let serial = hal::probe_serial(&tree)?;
/// serial.write_str("hello\n");
/// # Ok::<(), hal::ProbeError>(())
/// ```
pub fn probe_serial(tree: &impl DeviceTree) -> Result<&'static dyn Serial, ProbeError> {
    for driver in serial_drivers() {
        if let Some((base, size)) = tree.compatible_address(driver.compatible) {
            crate::mmu::map_device(base, size).map_err(|_| ProbeError::MappingFailed)?;

            let node = tree.compatible_node(driver.compatible);
            let node_ref = node.as_ref().map(|n| n as &dyn DeviceTreeNode);

            return Ok(*ACTIVE_SERIAL.get_or_init(|| (driver.init)(base, node_ref)));
        }
    }

    Err(ProbeError::NoCompatibleSerialDriver)
}

/// Returns the serial device selected by [`probe_serial`], if boot discovery
/// completed successfully.
///
/// Exception and panic reporting can use this without coupling the kernel to a
/// particular UART implementation.
pub fn active_serial() -> Option<&'static dyn Serial> {
    ACTIVE_SERIAL.get().copied()
}
