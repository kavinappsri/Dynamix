//! The ACPI namespace

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use acpica_sys as sys;

use super::raw::check;
use super::subsystem::Acpi;
use super::AcpiError;

/// Proof that the namespace is loaded; the way to find devices.
pub struct Namespace<'a> {
    _acpi: PhantomData<&'a Acpi>,
}

impl<'a> Namespace<'a> {
    pub(super) fn new(_acpi: &'a Acpi) -> Self {
        Self { _acpi: PhantomData }
    }

    /// Every present device whose `_HID` or `_CID` is `id` (`"PNP0501"`,
    /// `"ACPI0003"`, ...). ACPICA skips devices whose `_STA` says they are
    /// absent.
    pub fn devices(&self, id: &str) -> Result<Vec<Device>, AcpiError> {
        let mut name = [0u8; 16];
        // Short ids only: a PNP/ACPI id is 7 characters, a few firmware
        // oddities go a bit longer. Reject the rest instead of truncating.
        if id.is_empty() || id.len() >= name.len() || id.bytes().any(|byte| byte == 0) {
            return Err(AcpiError::BadParameter);
        }
        name[..id.len()].copy_from_slice(id.as_bytes());
        self.walk_devices(name.as_mut_ptr().cast())
    }

    /// Every present device in the namespace.
    pub fn all_devices(&self) -> Result<Vec<Device>, AcpiError> {
        self.walk_devices(ptr::null_mut())
    }

    fn walk_devices(&self, id: *mut core::ffi::c_char) -> Result<Vec<Device>, AcpiError> {
        let mut found: Vec<Device> = Vec::new();
        // SAFETY: `id` is null or a NUL-terminated string that outlives the
        // call; the callback only touches the `Vec` passed as context, which
        // is alive and not otherwise used for the duration of the call.
        check(unsafe {
            sys::AcpiGetDevices(id, Some(collect_device), ptr::addr_of_mut!(found).cast(), ptr::null_mut())
        })?;
        Ok(found)
    }
}

/// `AcpiGetDevices` callback: pushes every device handle onto the context `Vec`.
unsafe extern "C" fn collect_device(
    object: sys::ACPI_HANDLE,
    _nesting_level: u32,
    context: *mut c_void,
    _return_value: *mut *mut c_void,
) -> sys::ACPI_STATUS {
    // SAFETY: `context` is the `&mut Vec<Device>` from `walk_devices`.
    let found = unsafe { &mut *context.cast::<Vec<Device>>() };
    let Some(handle) = NonNull::new(object) else { return sys::AE_OK };
    if found.try_reserve(1).is_err() {
        return sys::AE_NO_MEMORY;
    }
    found.push(Device { handle });
    sys::AE_OK
}

/// A device in the namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    pub(super) handle: NonNull<c_void>,
}

// SAFETY: an ACPI handle is an opaque id of a namespace node. ACPICA does its
// own locking when it is used, and nodes are never freed here.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

/// What a device says about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// `_HID`
    pub hardware_id: Option<String>,
    /// `_UID`
    pub unique_id: Option<String>,
    /// Every `_CID`.
    pub compatible_ids: Vec<String>,
    /// `_ADR`, for devices on a bus (PCI: device in the high 16 bits, function in the low).
    pub address: Option<u64>,
}

/// An `ACPI_DEVICE_INFO` ACPICA allocated for us, freed on drop.
struct InfoBuffer(NonNull<sys::ACPI_DEVICE_INFO>);

impl Drop for InfoBuffer {
    fn drop(&mut self) {
        // SAFETY: allocated by ACPICA through `AcpiOsAllocate`, freed once.
        unsafe { crate::services::acpica_osl::AcpiOsFree(self.0.as_ptr().cast()) };
    }
}

/// Copies an `ACPI_PNP_DEVICE_ID` into an owned string.
///
/// # Safety
/// `id.String` must be null or point at `id.Length` readable bytes.
unsafe fn id_string(id: &sys::ACPI_PNP_DEVICE_ID) -> Option<String> {
    if id.String.is_null() || id.Length == 0 {
        return None;
    }
    // SAFETY: upheld by the caller.
    let bytes = unsafe { core::slice::from_raw_parts(id.String.cast::<u8>(), id.Length as usize) };
    // `Length` counts the terminating NUL.
    let text = core::str::from_utf8(bytes).ok()?.trim_end_matches('\0');
    Some(String::from(text))
}

impl Device {
    /// Asks ACPICA for the device's `_HID`, `_UID`, `_CID`s and `_ADR`.
    pub fn info(&self) -> Result<DeviceInfo, AcpiError> {
        let mut raw: *mut sys::ACPI_DEVICE_INFO = ptr::null_mut();
        // SAFETY: valid handle and out-pointer.
        check(unsafe { sys::AcpiGetObjectInfo(self.handle.as_ptr(), &mut raw) })?;
        let buffer = InfoBuffer(NonNull::new(raw).ok_or(AcpiError::NotFound)?);

        // SAFETY: ACPICA returned a valid, fully initialized structure whose
        // strings live inside the same allocation; `buffer` keeps it alive
        // until the end of this function.
        unsafe {
            let info = buffer.0.as_ref();
            let valid = u32::from(info.Valid);

            let mut compatible_ids = Vec::new();
            if valid & sys::ACPI_VALID_CID != 0 {
                let list = &info.CompatibleIdList;
                let ids = list.Ids.as_ptr();
                for index in 0..list.Count as usize {
                    if let Some(id) = id_string(&*ids.add(index)) {
                        compatible_ids.push(id);
                    }
                }
            }

            Ok(DeviceInfo {
                hardware_id: if valid & sys::ACPI_VALID_HID != 0 { id_string(&info.HardwareId) } else { None },
                unique_id: if valid & sys::ACPI_VALID_UID != 0 { id_string(&info.UniqueId) } else { None },
                compatible_ids,
                address: if valid & sys::ACPI_VALID_ADR != 0 { Some(info.Address) } else { None },
            })
        }
    }
}
