//! QEMU firmware configuration (`fw_cfg`) device.

use core::{
    mem::size_of,
    sync::atomic::{Ordering, compiler_fence},
};

use crate::mmio::Mmio;

const DATA: usize = 0x00;
const SELECTOR: usize = 0x08;
const DMA: usize = 0x10;
const FILE_DIRECTORY: u16 = 0x0019;
const DIRECTORY_NAME_LENGTH: usize = 56;
const DMA_CONTROL_SELECT_AND_WRITE: u32 = 0x18;

#[repr(C)]
struct DmaAccess {
    control: u32,
    length: u32,
    address: u64,
}

/// Errors returned by the `fw_cfg` driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwCfgError {
    /// A requested file name cannot fit in an `fw_cfg` directory entry.
    FileNameTooLong,
    /// A Rust object is too large to express in the device's 32-bit DMA length.
    TransferTooLarge,
}

/// A mapped QEMU `fw_cfg` device.
pub struct FwCfg {
    registers: Mmio,
}

impl FwCfg {
    /// Creates an `fw_cfg` device from its mapped register base.
    ///
    /// # Safety
    /// `base` must be a mapped QEMU `fw_cfg` MMIO device.
    /// The device must remain accessible for the value's lifetime.
    pub const unsafe fn new(base: usize) -> Self {
        Self {
            // SAFETY: upheld by this constructor's contract.
            registers: unsafe { Mmio::new(base) },
        }
    }

    /// Finds a file selector in the firmware configuration directory.
    ///
    /// Returns `Ok(None)` if no exact file-name match exists.
    ///
    /// # Errors
    ///
    /// Returns [`FwCfgError::FileNameTooLong`] when `name` cannot fit in an
    /// `fw_cfg` directory entry.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// if let Some(selector) = fw_cfg.find_file("etc/ramfb")? {
    ///     // Use `selector` for a subsequent transfer.
    /// }
    /// # Ok::<(), hal::FwCfgError>(())
    /// ```
    pub fn find_file(&self, name: &str) -> Result<Option<u16>, FwCfgError> {
        let name = name.as_bytes();
        if name.len() >= DIRECTORY_NAME_LENGTH {
            return Err(FwCfgError::FileNameTooLong);
        }

        self.registers.write16(SELECTOR, FILE_DIRECTORY.to_be());
        let count = self.read_be_u32();

        for _ in 0..count {
            self.skip(4); // File size.
            let selector =
                u16::from_be_bytes([self.registers.read8(DATA), self.registers.read8(DATA)]);
            self.skip(2); // Reserved.

            let mut file_name = [0; DIRECTORY_NAME_LENGTH];
            for byte in &mut file_name {
                *byte = self.registers.read8(DATA);
            }

            if file_name.starts_with(name) && file_name[name.len()] == b'\0' {
                return Ok(Some(selector));
            }
        }

        Ok(None)
    }

    /// Writes one plain-old-data object to a selected `fw_cfg` file through DMA.
    ///
    /// The object remains borrowed until the device reports completion, so its
    /// backing memory cannot disappear while the DMA transfer is active.
    ///
    /// # Errors
    ///
    /// Returns [`FwCfgError::TransferTooLarge`] if `T` is larger than the
    /// 32-bit length field accepted by `fw_cfg` DMA.
    pub fn write_object<T>(&self, selector: u16, object: &T) -> Result<(), FwCfgError> {
        let length = u32::try_from(size_of::<T>()).map_err(|_| FwCfgError::TransferTooLarge)?;
        let mut access = DmaAccess {
            control: (((u32::from(selector)) << 16) | DMA_CONTROL_SELECT_AND_WRITE).to_be(),
            length: length.to_be(),
            address: (object as *const T as usize as u64).to_be(),
        };

        compiler_fence(Ordering::SeqCst);
        self.registers
            .write64(DMA, (&mut access as *mut DmaAccess as usize as u64).to_be());

        while u32::from_be(unsafe { core::ptr::read_volatile(&access.control) }) & !1 != 0 {
            core::hint::spin_loop();
        }

        Ok(())
    }

    fn read_be_u32(&self) -> u32 {
        u32::from_be_bytes([
            self.registers.read8(DATA),
            self.registers.read8(DATA),
            self.registers.read8(DATA),
            self.registers.read8(DATA),
        ])
    }

    fn skip(&self, bytes: usize) {
        for _ in 0..bytes {
            self.registers.read8(DATA);
        }
    }
}
