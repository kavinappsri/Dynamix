//! The HAL part of the boot path, the one that happens before jumping to `rust_main`

use crate::services::boot_info::BootInfo;
use crate::services::boot_info::DeviceTree;


unsafe extern "Rust" {
    fn rust_main(boot: BootInfo<'static>) -> !;
}

#[unsafe(no_mangle)]
pub extern "C" fn fdt_entry(dtb_ptr: usize) -> ! {
    use crate::services::dtb::Dtb;

    let dtb = match unsafe { Dtb::from_ptr(dtb_ptr) } {
        Ok(tree) => tree,
        Err(_) => panic!("invalid DTB"),
    };

    let mut boot = BootInfo::empty();

    boot.memory_regions = crate::services::boot_info::RegionList::from_fill(|out| {
        use crate::services::device_query::DeviceQuery;
        dtb.memory_regions(out)
    });
    boot.device_tree_reserved_range = Some(dtb.physical_range());
    boot.device_tree = Some(DeviceTree::Fdt(dtb));

    unsafe { rust_main(boot) };
}

#[cfg(target_arch = "x86_64")]
mod uefi_boot_entry {
    use core::cell::UnsafeCell;
    use core::mem::{MaybeUninit, size_of};

    use super::rust_main;
    use crate::acpi as acpi_crate;
    use crate::services::acpi::AcpiQuery;
    use crate::services::boot_info::{BootInfo, DeviceTree, RegionList};
    use crate::services::cpu;
    use crate::x86_64::boot_paging;
    use crate::x86_64::boot_stack;
    use crate::x86_64::gop::{self, GopError};
    use crate::x86_64::rsdp::{self, RsdpError};
    use crate::x86_64::uefi_boot::{self, ExitBootServicesError, format_hex, println_ascii};
    use crate::x86_64::memory_map::MemoryMapError;
    use crate::x86_64::uefi_types::{self, EfiHandle, EfiStatus, EfiSystemTable};

    /// Where `BootInfo` waits while the kernel changes address spaces: inside
    /// the image, so it is reachable at its load address before the jump and
    /// at its higher-half alias after it. (The old firmware stack is neither
    /// mapped afterwards nor safe to keep.)
    struct BootInfoSlot(UnsafeCell<MaybeUninit<BootInfo<'static>>>);

    // SAFETY: written once by the boot core before the jump, read once after.
    unsafe impl Sync for BootInfoSlot {}

    static BOOT_INFO: BootInfoSlot = BootInfoSlot(UnsafeCell::new(MaybeUninit::uninit()));

    /// Stage 2: first code on the kernel's own stack, still at the load
    /// address under firmware's page tables. `boot` points into the firmware
    /// stack we just left, so it is moved into [`BOOT_INFO`] before anything
    /// else; then the page tables are built and the higher-half jump is made.
    unsafe extern "efiapi" fn stage2(boot: *mut BootInfo<'static>) -> ! {
        let boot = unsafe { core::ptr::read(boot) };
        unsafe { (*BOOT_INFO.0.get()).write(boot) };


        match unsafe { boot_paging::prepare() } {
            Ok(prepared) => unsafe { boot_paging::activate(prepared, higher_half_entry) },
            Err(_) => {
                let boot = unsafe { (*BOOT_INFO.0.get()).assume_init_ref() };
                if let Some(framebuffer) = boot.framebuffer {
                    framebuffer.fill(0x00_FF_00_00);
                }
                cpu::hlt_frvr()
            }
        }
    }

    /// Stage 3: the first code to run in the higher half, on the higher-half
    /// alias of the boot stack. Hands `BootInfo` to the kernel by value, from
    /// a frame of its own.
    unsafe extern "efiapi" fn higher_half_entry() -> ! {
        // SAFETY: `stage2` initialised the slot before the jump.
        let mut boot = unsafe { (*BOOT_INFO.0.get()).assume_init_read() };

        // The ACPI query was built under firmware's page tables; its cached root
        // table pointer is not mapped in ours. Re-read it before the kernel uses it.
        let acpi_unusable = match &mut boot.device_tree {
            Some(DeviceTree::Acpi(acpi)) => unsafe { acpi.remap() }.is_err(),
            _ => false,
        };
        if acpi_unusable {
            boot.device_tree = None;
        }

        // SAFETY: `rust_main` is the kernel's entry point and never returns.
        unsafe { rust_main(boot) }
    }

    /// Formats `num` as decimal into `buf`, returning the written `&str`.
    fn usize_to_str(mut num: usize, buf: &mut [u8; 20]) -> &str {
        if num == 0 {
            return "0";
        }
        let mut i = buf.len();
        while num > 0 {
            i -= 1;
            buf[i] = b'0' + (num % 10) as u8;
            num /= 10;
        }
        // SAFETY: every byte written above is an ASCII digit.
        unsafe { core::str::from_utf8_unchecked(&buf[i..]) }
    }

    /// Entry point the firmware calls directly. This exact symbol name
    /// matters: the `x86_64-unknown-uefi` target's default linker
    /// invocation sets `/entry:efi_main`, so the PE loader finds this
    /// function by that name, not by scanning for a particular signature or
    /// attribute.
    #[unsafe(no_mangle)]
    pub extern "efiapi" fn efi_main(
        image_handle: EfiHandle,
        system_table: *mut EfiSystemTable,
    ) -> EfiStatus {
        let system_table_ref = unsafe { &*system_table };
        let con_out = system_table_ref.con_out;
        let boot_services = system_table_ref.boot_services;

        unsafe { println_ascii(con_out, "Dynamix UEFI boot stage reached.") };

        let mut hex_buf = [0u8; 2 + size_of::<usize>() * 2];
        let mut boot = BootInfo::empty();

        // --- Framebuffer: GOP, if present. ---
        match unsafe { gop::locate_gop(boot_services) } {
            Ok(fb) => {

                fb.fill(0x00_1E_90_FF) ; // sanity-check color

                // SAFETY: still inside Boot Services.
                unsafe {
                    println_ascii(con_out, "[+] GOP found, framebuffer filled");
                    println_ascii(con_out, "    base=");
                    println_ascii(con_out, format_hex(fb.base as usize, &mut hex_buf));
                }

                boot.framebuffer = Some(fb);
            }
            Err(e) => {
                let msg = match e {
                    GopError::NotFound => "[ ] GOP not found",
                    GopError::LocateFailed => "[ ] GOP LocateProtocol failed",
                    GopError::NullMode => "[ ] GOP reported a null mode",
                    GopError::SetModeFailed => "[ ] GOP SetMode failed",
                    GopError::NoLinearFramebuffer => "[ ] GOP has no linear framebuffer",
                    GopError::UnsupportedPixelFormat => "[ ] GOP pixel format unsupported",
                };
                // SAFETY: still inside Boot Services.
                unsafe { println_ascii(con_out, msg) };
            }
        }


        match unsafe { rsdp::locate_rsdp(system_table) } {
            Ok(rsdp_ptr) => {
                // SAFETY: still inside Boot Services.
                unsafe {
                    println_ascii(con_out, "[+] RSDP found");
                    println_ascii(con_out, "    addr=");
                    println_ascii(con_out, format_hex(rsdp_ptr as usize, &mut hex_buf));
                }

                match unsafe { AcpiQuery::from_rsdp(rsdp_ptr as usize) } {
                    Ok(query) => {
                        let mut count_buf = [0u8; 20];
                        let table_count = query.headers().count();

                        // SAFETY: still inside Boot Services.
                        unsafe {
                            println_ascii(con_out, "[+] ACPI tables parsed");
                            println_ascii(con_out, "    revision=");
                            println_ascii(con_out, usize_to_str(query.revision() as usize, &mut count_buf));
                            println_ascii(con_out, "    tables=");
                            println_ascii(con_out, usize_to_str(table_count, &mut count_buf));

                            println_ascii(
                                con_out,
                                if query.find_table::<acpi_crate::fadt::Fadt>().is_some() {
                                    "[+] FADT present"
                                } else {
                                    "[ ] FADT not present"
                                },
                            );
                            println_ascii(
                                con_out,
                                if query.find_table::<acpi_crate::madt::Madt>().is_some() {
                                    "[+] MADT present"
                                } else {
                                    "[ ] MADT not present"
                                },
                            );
                        }

                        boot.device_tree = Some(DeviceTree::Acpi(query));
                    }
                    Err(_) => {
                        // SAFETY: still inside Boot Services.
                        unsafe { println_ascii(con_out, "[ ] ACPI table parsing failed") };
                    }
                }
            }
            Err(RsdpError::NotFound) => {
                // SAFETY: still inside Boot Services.
                unsafe { println_ascii(con_out, "[ ] RSDP not found in configuration table") };
            }
        }

        unsafe { println_ascii(con_out, "[+] Exiting boot services, falling into rust_main") };


        let memory_map = match unsafe { uefi_boot::exit_boot_services(image_handle, boot_services) } {
            Ok(map) => map,
            Err(e) => {
                // Boot services survive a failed exit, so report and hand the
                // failure back to firmware instead of falling into the kernel.
                let (msg, status) = match e {
                    ExitBootServicesError::MemoryMap(MemoryMapError::BufferTooSmall { .. }) => {
                        ("[ ] Memory map buffer too small", uefi_types::EFI_LOAD_ERROR)
                    }
                    ExitBootServicesError::MemoryMap(MemoryMapError::DescriptorSizeTooSmall { .. }) => {
                        ("[ ] Memory map descriptor size too small", uefi_types::EFI_LOAD_ERROR)
                    }
                    ExitBootServicesError::MemoryMap(MemoryMapError::GetMemoryMapFailed(status)) => {
                        ("[ ] GetMemoryMap failed", status)
                    }
                    ExitBootServicesError::Exit(status) => ("[ ] ExitBootServices failed", status),
                };
                // SAFETY: still inside Boot Services.
                unsafe { println_ascii(con_out, msg) };
                return status;
            }
        };

        // --- From here on there is no firmware console and no boot services. ---

        boot.memory_regions = RegionList::from_fill(|out| memory_map.fill_usable_regions(out));


        unsafe { core::arch::asm!("cli", options(nomem, nostack)) };

        // Take over the machine:
        unsafe { boot_stack::switch_and_call(&mut boot, stage2) }
    }
}
