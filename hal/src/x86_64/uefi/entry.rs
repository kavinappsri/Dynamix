//! `efi_main`: the first Rust code to run, called by firmware.
//!
//! Discovers what the kernel needs (framebuffer, ACPI, memory map), leaves
//! Boot Services, and hands over to [`crate::x86_64::handoff`].

use core::fmt::Write;

use super::config_table::{self, RsdpError};
use super::console::ConOut;
use super::exit::{self, ExitBootServicesError};
use super::gop;
use super::memory_map::{MemoryMap, MemoryMapError};
use super::types::{self, EfiHandle, EfiStatus, EfiSystemTable};
use crate::services::acpi::AcpiQuery;
use crate::services::boot_info::{BootInfo, HardwareDescription, RegionList};
use crate::x86_64::handoff;

/// Color used to prove the GOP framebuffer works before the kernel runs.
const GOP_SANITY_COLOR: u32 = 0x00_1E_90_FF;

/// Entry point
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image_handle: EfiHandle, system_table: *mut EfiSystemTable) -> EfiStatus {
    // SAFETY: firmware hands `efi_main` a valid system table.
    let system_table_ref = unsafe { &*system_table };
    let boot_services = system_table_ref.boot_services;
    // SAFETY: valid until Boot Services exit, which happens below; `out` is
    // not used afterwards.
    let mut out = unsafe { ConOut::new(system_table_ref.con_out) };

    let _ = writeln!(out, "Dynamix UEFI boot stage reached.");

    let mut boot = BootInfo::empty();

    // --- Framebuffer: GOP, if present. ---
    // SAFETY: still inside Boot Services.
    match unsafe { gop::locate_gop(boot_services) } {
        Ok(fb) => {
            fb.fill(GOP_SANITY_COLOR);
            let _ = writeln!(out, "[+] GOP found, framebuffer filled (base={:#x})", fb.base());
            boot.framebuffer = Some(fb);
        }
        Err(e) => {
            let _ = writeln!(out, "[ ] {}", e.describe());
        }
    }

    // --- ACPI: RSDP from the configuration table, then the tables behind it. ---
    // SAFETY: `system_table` is the valid table firmware gave us.
    match unsafe { config_table::locate_rsdp(system_table) } {
        Ok(rsdp) => {
            let _ = writeln!(out, "[+] RSDP found (addr={:#x})", rsdp as usize);
            // SAFETY: firmware identity-maps the RSDP while Boot Services are alive.
            match unsafe { AcpiQuery::from_rsdp(rsdp as usize) } {
                Ok(query) => {
                    let _ = writeln!(out, "[+] ACPI tables parsed (revision={}, tables={})", query.revision(), query.headers().count());
                    boot.hardware = Some(HardwareDescription::Acpi(query));
                }
                Err(_) => {
                    let _ = writeln!(out, "[ ] ACPI table parsing failed");
                }
            }
        }
        Err(RsdpError::NotFound) => {
            let _ = writeln!(out, "[ ] RSDP not found in configuration table");
        }
    }

    let _ = writeln!(out, "[+] Exiting boot services, falling into rust_main");

    let Some(mut memory_map) = MemoryMap::take() else {
        let _ = writeln!(out, "[ ] Memory map buffer already in use");
        return types::EFI_LOAD_ERROR;
    };

    // SAFETY: `image_handle` is what firmware gave us; Boot Services are still valid.
    if let Err(e) = unsafe { exit::exit_boot_services(image_handle, boot_services, &mut memory_map) } {
        // Boot services survive a failed exit, so report and hand the failure
        // back to firmware instead of falling into the kernel.
        let status = match e {
            ExitBootServicesError::MemoryMap(MemoryMapError::GetMemoryMapFailed(status)) | ExitBootServicesError::Exit(status) => {
                let _ = writeln!(out, "[ ] {} (status {:#x})", describe_exit_error(e), status);
                status
            }
            ExitBootServicesError::MemoryMap(_) => {
                let _ = writeln!(out, "[ ] {}", describe_exit_error(e));
                types::EFI_LOAD_ERROR
            }
        };
        return status;
    }

    // --- From here on there is no firmware console and no boot services. ---

    boot.memory_regions = RegionList::from_fill(|regions| memory_map.fill_usable_regions(regions));

    // SAFETY: interrupts stay masked until the kernel installs its own IDT.
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };

    // SAFETY: Boot Services have exited and `boot` is complete.
    unsafe { handoff::enter_kernel(&mut boot) }
}

fn describe_exit_error(error: ExitBootServicesError) -> &'static str {
    match error {
        ExitBootServicesError::MemoryMap(MemoryMapError::BufferTooSmall { .. }) => "Memory map buffer too small",
        ExitBootServicesError::MemoryMap(MemoryMapError::DescriptorSizeTooSmall { .. }) => "Memory map descriptor size too small",
        ExitBootServicesError::MemoryMap(MemoryMapError::GetMemoryMapFailed(_)) => "GetMemoryMap failed",
        ExitBootServicesError::Exit(_) => "ExitBootServices failed",
    }
}
