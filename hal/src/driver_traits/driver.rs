//! Driver Abstraction
//!
//! This is at least planned to be a sort of abstraction for all drivers, like
//! framebuffer, serial, etc... so we don't hv repeated code for every driver trait.
//! We'll see how it goes
//!
//!  ____How driver tables are collected, per target:____
//!
//! Im putting that here since its pretty important
//!
//! A driver registers itself with [`register_driver!`], which drops a static
//! [`Driver`] into a per-kind link section. The probe functions made by
//! [`define_probe!`] walk that section. The two sides must agree on where a
//! kind's table starts and ends, and that is the only part that differs by
//! linker:
//!
//! * **ELF targets (aarch64, armv7)**: the kernel linker script gathers the
//!   `.drivers.<kind>` input sections between `__start_<kind>_drivers` and
//!   `__stop_<kind>_drivers`.
//! * **`x86_64-unknown-uefi` (PE/COFF, `lld-link`)**: there is no linker
//!   script. Input sections named `name$suffix` are merged into one output
//!   section `name`, ordered by suffix, so each kind uses three subsections:
//!   `.drv$<k>0` holds a [`TableMarker`], `.drv$<k>1` the entries and
//!   `.drv$<k>2` another marker. The entries are exactly what lies between the
//!   two markers.

use crate::services::dtb::Dtb;
use crate::services::device_query::{DeviceQuery, DeviceResource};


/// Driver struct, Generic over the driver trait
#[repr(C)]
pub struct Driver<T: 'static + ?Sized> {
    pub name: &'static str,
    /// What the device is called: a FDT `compatible`, or an ACPI `_HID`
    pub compatible: &'static str,
    pub init: fn(resource: DeviceResource, tree: &dyn DeviceQuery) -> Result<&'static T, DriverProbeError>,
}


/// Errors that could happen while probing for drivers
#[derive(Debug)]
pub enum DriverProbeError {
    NoCompatibleDriver,
    MappingFailed,
    InitFailed,
    WrongResource,
}


#[doc(hidden)]
pub mod table {
    /// A table boundary on PE targets.
    #[repr(transparent)]
    pub struct TableMarker(pub usize);

    /// Bytes a marker occupies; the first entry starts this far past the
    /// start marker.
    pub const MARKER_SIZE: usize = size_of::<TableMarker>();
}

/// Macro for drivers to compile themselves in the binary
#[macro_export]
macro_rules! register_driver {
    ($name:ident, $compat:literal, $init_fn:expr, Serial) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, Serial, ".drivers.serial", ".drv$s1");
    };
    ($name:ident, $compat:literal, $init_fn:expr, Framebuffer) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, Framebuffer, ".drivers.framebuffer", ".drv$f1");
    };
    ($name:ident, $compat:literal, $init_fn:expr, ClockController) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, ClockController, ".drivers.clock", ".drv$c1");
    };
    ($name:ident, $compat:literal, $init_fn:expr, PowerDomainController) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, PowerDomainController, ".drivers.power_domain", ".drv$p1");
    };

    (@internal $name:ident, $compat:literal, $init_fn:expr, $dtype:path, $elf_section:literal, $pe_section:literal) => {
        #[used]
        #[cfg_attr(not(target_arch = "x86_64"), unsafe(link_section = $elf_section))]
        #[cfg_attr(target_arch = "x86_64", unsafe(link_section = $pe_section))]
        static $name: Driver<dyn $dtype> = Driver {
            name: stringify!($name),
            compatible: $compat,
            init: $init_fn,
        };
    };
}

/// Internal implementation for all probe sriver functions
pub fn probe_driver<D: 'static + ?Sized>(start_tag: *const u8, stop_tag: *const u8, tree: &dyn DeviceQuery) -> Result<&'static D, DriverProbeError> {
    let start = start_tag as *const Driver<D>;
    let stop = stop_tag as *const Driver<D>;
    let count = (stop as usize - start as usize) / size_of::<Driver<D>>();
    let drivers = unsafe{ core::slice::from_raw_parts(start, count) };

    for driver in drivers {
        if let Some(resource) = tree.find_compatible_resource(driver.compatible) {
            // Only memory needs mapping
            if let DeviceResource::Mmio { base, size } = resource {
                crate::services::mmu::map_device(base, size).map_err(|_| DriverProbeError::MappingFailed)?;
            }
            let initialized_driver = (driver.init)(resource, tree)?;
            return Ok(initialized_driver);
        }
    }

    Err(DriverProbeError::NoCompatibleDriver)
}

/// Defines a new probe driver implementation
///
/// `define_probe!(Kind, probe_fn, |driver| post_init)` for one of the kinds
/// [`register_driver!`] knows. Once per module, next to the trait.
#[macro_export]
macro_rules! define_probe {
    (Serial, $fn_name:ident, $post_init:expr) => {
        $crate::define_probe!(@internal Serial, $fn_name, $post_init,
            "__start_serial_drivers", "__stop_serial_drivers", ".drv$s0", ".drv$s2", 0x5345_5230);
    };
    (Framebuffer, $fn_name:ident, $post_init:expr) => {
        $crate::define_probe!(@internal Framebuffer, $fn_name, $post_init,
            "__start_framebuffer_drivers", "__stop_framebuffer_drivers", ".drv$f0", ".drv$f2", 0x4642_5230);
    };
    (ClockController, $fn_name:ident, $post_init:expr) => {
        $crate::define_probe!(@internal ClockController, $fn_name, $post_init,
            "__start_clock_drivers", "__stop_clock_drivers", ".drv$c0", ".drv$c2", 0x434C_4B30);
    };
    (PowerDomainController, $fn_name:ident, $post_init:expr) => {
        $crate::define_probe!(@internal PowerDomainController, $fn_name, $post_init,
            "__start_power_domain_drivers", "__stop_power_domain_drivers", ".drv$p0", ".drv$p2", 0x5044_4330);
    };

    (@internal $dtype:path, $fn_name:ident, $post_init:expr,
     $elf_start:literal, $elf_stop:literal, $pe_start:literal, $pe_stop:literal, $label:literal) => {
        #[cfg(not(target_arch = "x86_64"))]
        unsafe extern "C" {
            #[link_name = $elf_start]
            static __start_tag: u8;
            #[link_name = $elf_stop]
            static __stop_tag: u8;
        }

        #[cfg(target_arch = "x86_64")]
        #[used]
        #[allow(non_upper_case_globals)]
        #[unsafe(link_section = $pe_start)]
        static __start_tag: $crate::driver_traits::driver::table::TableMarker =
            $crate::driver_traits::driver::table::TableMarker($label);

        #[cfg(target_arch = "x86_64")]
        #[used]
        #[allow(non_upper_case_globals)]
        #[unsafe(link_section = $pe_stop)]
        static __stop_tag: $crate::driver_traits::driver::table::TableMarker =
            $crate::driver_traits::driver::table::TableMarker($label + 1);

        pub fn $fn_name(tree: &dyn $crate::services::device_query::DeviceQuery) -> Result<&'static dyn $dtype, $crate::driver_traits::driver::DriverProbeError> {
            // The table's first entry: the start symbol itself on ELF, one
            // marker past the start marker on PE.
            #[cfg(not(target_arch = "x86_64"))]
            let first = core::ptr::addr_of!(__start_tag);
            #[cfg(target_arch = "x86_64")]
            let first = (core::ptr::addr_of!(__start_tag) as *const u8)
                .wrapping_add($crate::driver_traits::driver::table::MARKER_SIZE);

            #[cfg(not(target_arch = "x86_64"))]
            let end = core::ptr::addr_of!(__stop_tag);
            #[cfg(target_arch = "x86_64")]
            let end = core::ptr::addr_of!(__stop_tag) as *const u8;

            let driver = $crate::driver_traits::driver::probe_driver::<dyn $dtype>(first, end, tree)?;
            $post_init(driver);
            Ok(driver)
        }
    };
}