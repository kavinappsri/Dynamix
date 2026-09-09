//! Driver Abstraction
//!
//! This is at least planned to be a sort of abstraction for all drivers, like
//! framebuffer, serial, etc... so we don't hv repeated code for every driver trait.
//! We'll see how it goes

use crate::services::dtb::Dtb;

/// Driver struct, Generic over the driver trait
#[repr(C)]
pub struct Driver<T: 'static + ?Sized> {
    pub name: &'static str,
    pub compatible: &'static str,
    pub init: fn(base: usize, node: &Dtb) -> Result<&'static T, DriverProbeError>,
}


/// Errors that could happen while probing for drivers
#[derive(Debug)]
pub enum DriverProbeError {
    NoCompatibleDriver,
    MappingFailed,
    InitFailed,
}

/// Macro for drivers to compile themselves in the binary
#[macro_export]
macro_rules! register_driver {
    (@internal $name:ident, $compat:literal, $init_fn:expr, $dtype:path, $section:literal) => {
        #[used]
        #[unsafe(link_section = $section)]
        static $name: Driver<dyn $dtype> = Driver {
            name: stringify!($name),
            compatible: $compat,
            init: $init_fn,
        };
    };

    ($name:ident, $compat:literal, $init_fn:expr, Serial) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, Serial, ".drivers.serial");
    };
    ($name:ident, $compat:literal, $init_fn:expr, Framebuffer) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, Framebuffer, ".drivers.framebuffer");
    };
    ($name:ident, $compat:literal, $init_fn:expr, ClockController) => {
        $crate::register_driver!(@internal $name, $compat, $init_fn, ClockController, ".drivers.clock");
    };
}
/// Internal implementation for all probe sriver functions
pub fn probe_driver<D: 'static + ?Sized>(start_tag: *const u8, stop_tag: *const u8, tree: &Dtb) -> Result<&'static D, DriverProbeError> {
    let start = start_tag as *const Driver<D>;
    let stop = stop_tag as *const Driver<D>;
    let count = (stop as usize - start as usize) / size_of::<Driver<D>>();
    let drivers = unsafe{ core::slice::from_raw_parts(start, count) };

    for driver in drivers {
        if let Some((base, size)) = tree.find_compatible_address(driver.compatible) {
            crate::services::mmu::map_device(base, size).map_err(|_| DriverProbeError::MappingFailed)?;
            let initialized_driver = (driver.init)(base, tree)?;
            return Ok(initialized_driver);
        }
    }

    Err(DriverProbeError::NoCompatibleDriver)
}

/// Defines a new probe driver implementation
#[macro_export]
macro_rules! define_probe {
    ($start:literal, $stop:literal, $fn_name:ident, $dtype:path, $post_init:expr) => {
        unsafe extern "C" {
            #[link_name = $start]
            static __start_tag: u8;
            #[link_name = $stop]
            static __stop_tag: u8;
        }

        pub fn $fn_name(tree: &Dtb) -> Result<&'static dyn $dtype, $crate::driver_traits::driver::DriverProbeError> {
            let driver = $crate::driver_traits::driver::probe_driver::<dyn $dtype>(
                core::ptr::addr_of!(__start_tag),
                core::ptr::addr_of!(__stop_tag),
                tree
            )?;
            $post_init(driver);
            Ok(driver)
        }
    };
}

// TODO: Make an active dtb thing nd remove most of the dtb handing from kernel