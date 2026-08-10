// fw_cfg.rs

const FW_CFG_FILE_DIR_SELECTOR: u16 = 0x0019;

#[repr(C)]
struct FwCfgDmaAccess {
    control: u32,
    length: u32,
    address: u64,
}

pub struct FwCfg {
    base: usize,
}

impl FwCfg {
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    /// Searches the fw_cfg directory for a specific file and returns its selector key.
    pub fn find_file(&self, name: &str) -> Option<u16> {
        let data_reg = self.base as *mut u8;
        let selector_reg = (self.base + 0x08) as *mut u16;
        let name_bytes = name.as_bytes();

        unsafe {
            // Select the file directory catalog
            selector_reg.write_volatile(FW_CFG_FILE_DIR_SELECTOR.to_be());

            // Read file count (u32, big-endian)
            let mut count_bytes = [0u8; 4];
            for b in count_bytes.iter_mut() {
                *b = data_reg.read_volatile();
            }
            let count = u32::from_be_bytes(count_bytes);

            // Loop through directory entries (each entry is 64 bytes)
            for _ in 0..count {
                for _ in 0..4 { data_reg.read_volatile(); } // Skip size

                // Read selector key (2 bytes, big-endian)
                let key_hi = data_reg.read_volatile();
                let key_lo = data_reg.read_volatile();
                let select_key = u16::from_be_bytes([key_hi, key_lo]);

                data_reg.read_volatile(); // Skip reserved
                data_reg.read_volatile(); // Skip reserved

                // Read 56-byte filename string
                let mut name_buf = [0u8; 56];
                for b in name_buf.iter_mut() {
                    *b = data_reg.read_volatile();
                }

                // Check for exact match including null terminator
                if name_buf.starts_with(name_bytes) && name_buf[name_bytes.len()] == b'\0' {
                    return Some(select_key);
                }
            }
        }
        None
    }

    /// Triggers a DMA write to the fw_cfg device.
    ///
    /// # Safety
    /// `data_ptr` must point to a valid, initialized structure of `length` bytes.
    pub unsafe fn do_dma_write(&self, selector: u16, data_ptr: u64, length: u32) {
        let dma_reg = (self.base + 0x10) as *mut u64;

        // Control flags: FW_CFG_DMA_CTL_SELECT (1 << 3) | FW_CFG_DMA_CTL_WRITE (1 << 4) = 0x18
        let control = ((selector as u32) << 16) | 0x18;

        let mut dma = FwCfgDmaAccess {
            control: control.to_be(),
            length: length.to_be(),
            address: data_ptr.to_be(),
        };

        let dma_addr = &mut dma as *mut _ as u64;

        // Ensure struct is fully written to memory before triggering DMA
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

        // Trigger transfer
        unsafe {
            dma_reg.write_volatile(dma_addr.to_be());
        }

        // Wait for DMA completion (QEMU clears control to 0 on success)
        while unsafe { u32::from_be(core::ptr::read_volatile(&dma.control)) & !1 != 0 } {
            core::hint::spin_loop();
        }
    }
}