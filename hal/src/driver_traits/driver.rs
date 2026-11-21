//! How drivers describe themselves, and how the kernel finds them.
//!
//! A driver is one [`DriverInfo`]: a name, the firmware ids it handles, and a
//! `probe` function. [`driver!`] drops it into the driver table, a single link
//! section that every driver in the crate adds to. [`crate::services::device`]
//! then matches devices against it.
//!
//! Matches are per firmware on purpose. A device tree names hardware with
//! `compatible` strings and ACPI names it with `_HID`/`_CID`s; A
//! driver that has both just lists both.
//!
//! ### How the table is collected, per target
//!
//! * **ELF (aarch64, armv7)**: the kernel linker script gathers `.drivers`
//!   between `__start_drivers` and `__stop_drivers`.
//! * **`x86_64-unknown-uefi` (PE/COFF, `lld-link`)**: no linker script, but
//!   input sections named `name$suffix` merge into `name`, ordered by suffix.
//!   `.drv$a` holds a [`table::TableMarker`], `.drv$m` the entries and
//!   `.drv$z` another marker; the entries are what lies between the markers.

use core::fmt;
use crate::services::device::Device;

/// What a driver says it can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    /// A device tree `compatible` string.
    Fdt(&'static str),
    /// An ACPI `_HID` or `_CID`.
    Acpi(&'static str),
    /// Hardware ACPI table
    AcpiTable(&'static str),
}

#[derive(Debug)]
pub struct DriverInfo {
    pub name: &'static str,
    pub matches: &'static [Match],
    /// Called once for each device this driver matches and wins. On `Ok` the
    /// driver has registered whatever it provides, e.g. with
    /// [`crate::driver_traits::serial::register`].
    pub probe: fn(&'static Device) -> Result<(), ProbeError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// Waiting for dependency
    Defer,
    /// The device lacks a resource the driver needs.
    MissingResource,
    /// A register window could not be mapped.
    MappingFailed,
    /// The firmware description could not be read, or says something this
    /// driver cannot use.
    Firmware,
    /// The hardware is not there, or did not answer.
    NotPresent,
    /// The hardware did not come up.
    InitFailed,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ProbeError::Defer => "waiting for a dependency",
            ProbeError::MissingResource => "missing resource",
            ProbeError::MappingFailed => "mapping failed",
            ProbeError::Firmware => "bad firmware description",
            ProbeError::NotPresent => "hardware not present",
            ProbeError::InitFailed => "initialization failed",
        })
    }
}

#[doc(hidden)]
pub mod table {
    /// A table boundary on PE targets.
    #[repr(transparent)]
    pub struct TableMarker(pub usize);

    /// Bytes a marker occupies; the first entry starts this far past the
    /// start marker.
    pub const MARKER_SIZE: usize = size_of::<TableMarker>();

    #[cfg(target_arch = "x86_64")]
    #[used]
    #[unsafe(link_section = ".drv$a")]
    pub static START: TableMarker = TableMarker(0x4452_5653);

    #[cfg(target_arch = "x86_64")]
    #[used]
    #[unsafe(link_section = ".drv$z")]
    pub static STOP: TableMarker = TableMarker(0x4452_5645);
}

/// Adds a driver to the driver table.
///
/// ```ignore
/// driver! {
///     name: "pl011",
///     matches: [Match::Fdt("arm,pl011")],
///     probe: probe,
/// }
/// ```
#[macro_export]
macro_rules! driver {
    (name: $name:literal, matches: [$($matched:expr),+ $(,)?], probe: $probe:path $(,)?) => {
        const _: () = {
            #[used]
            #[cfg_attr(not(target_arch = "x86_64"), unsafe(link_section = ".drivers"))]
            #[cfg_attr(target_arch = "x86_64", unsafe(link_section = ".drv$m"))]
            static DRIVER: $crate::driver_traits::driver::DriverInfo = $crate::driver_traits::driver::DriverInfo {
                name: $name,
                matches: &[$($matched),+],
                probe: $probe,
            };
        };
    };
}

/// Every driver linked into this kernel.
pub fn table() -> &'static [DriverInfo] {
    #[cfg(not(target_arch = "x86_64"))]
    let (start, end) = {
        unsafe extern "C" {
            static __start_drivers: u8;
            static __stop_drivers: u8;
        }
        (core::ptr::addr_of!(__start_drivers), core::ptr::addr_of!(__stop_drivers))
    };

    #[cfg(target_arch = "x86_64")]
    let (start, end) = (
        (core::ptr::addr_of!(table::START) as *const u8).wrapping_add(table::MARKER_SIZE),
        core::ptr::addr_of!(table::STOP) as *const u8,
    );

    let count = (end as usize).saturating_sub(start as usize) / size_of::<DriverInfo>();
    // SAFETY: the linker placed `count` consecutive `DriverInfo`s between the
    // two boundaries, and nothing else lives there.
    unsafe { core::slice::from_raw_parts(start.cast::<DriverInfo>(), count) }
}