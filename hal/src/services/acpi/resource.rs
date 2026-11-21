//! Device resources: what firmware's `_CRS` says a device uses.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::mem::offset_of;
use core::ptr;

use acpica_sys as sys;

use super::namespace::Device;
use super::raw::check;
use super::AcpiError;
use crate::driver_traits::interrupt_controller::{Polarity, Trigger};

/// What an address-space descriptor ([`Resource::Address`]) describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressSpace {
    Memory,
    Io,
    BusNumber,
    /// A type this kernel has no name for.
    Other(u8),
}

/// One entry of a `_CRS`, decoded into plain values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    /// I/O ports `min..=max` (the device may be placed anywhere in that
    /// window, `alignment` apart), `length` ports long.
    Io { min: u16, max: u16, alignment: u8, length: u8 },
    /// `length` I/O ports at exactly `base`.
    FixedIo { base: u16, length: u8 },
    /// A memory window (24-bit, 32-bit and fixed 32-bit descriptors alike);
    /// `min == max` when the placement is fixed.
    Memory { min: u64, max: u64, length: u64, writable: bool },
    /// A 16/32/64-bit address-space descriptor (bridges and the like).
    Address { space: AddressSpace, produces: bool, min: u64, max: u64, length: u64, translation: u64 },
    /// Interrupt lines (GSIs). Covers both the legacy `IRQ` and the
    /// `Interrupt` descriptors.
    Irq { lines: Vec<u32>, trigger: Trigger, polarity: Polarity, shared: bool },
    /// DMA channels.
    Dma { channels: Vec<u8> },
    /// A descriptor kind this kernel does not decode (ACPICA's type number).
    Unsupported { kind: u32 },
}

impl Device {
    /// The device's current resources, from `_CRS`.
    ///
    /// Runs AML, so it can fail on bad firmware; `NotFound` means the device
    /// has no `_CRS`.
    pub fn resources(&self) -> Result<Vec<Resource>, AcpiError> {
        let mut found: Vec<Resource> = Vec::new();
        // SAFETY: valid handle; "_CRS" is NUL-terminated; the callback only
        // touches the `Vec` passed as context, alive for the whole call.
        check(unsafe {
            sys::AcpiWalkResources(
                self.handle.as_ptr(),
                c"_CRS".as_ptr().cast_mut(),
                Some(collect_resource),
                ptr::addr_of_mut!(found).cast(),
            )
        })?;
        Ok(found)
    }
}

/// `AcpiWalkResources` callback: decodes each descriptor onto the context `Vec`.
unsafe extern "C" fn collect_resource(resource: *mut sys::ACPI_RESOURCE, context: *mut c_void) -> sys::ACPI_STATUS {
    // SAFETY: `context` is the `&mut Vec<Resource>` from `resources`.
    let found = unsafe { &mut *context.cast::<Vec<Resource>>() };
    // SAFETY: ACPICA passes a valid descriptor.
    if let Some(decoded) = unsafe { decode(resource) } {
        if found.try_reserve(1).is_err() {
            return sys::AE_NO_MEMORY;
        }
        found.push(decoded);
    }
    sys::AE_OK
}

/// Copies a `T` out of the descriptor's data
///
/// # Safety
/// `data` must point at a `T`-sized, integer-only descriptor body.
unsafe fn field<T>(data: *const u8) -> T {
    // SAFETY: upheld by the caller; the read is unaligned on purpose.
    unsafe { ptr::read_unaligned(data.cast::<T>()) }
}

fn trigger(raw: u8) -> Trigger {
    // ACPI_LEVEL_SENSITIVE = 0, ACPI_EDGE_SENSITIVE = 1.
    if raw == 1 { Trigger::Edge } else { Trigger::Level }
}

fn polarity(raw: u8) -> Polarity {
    // ACPI_ACTIVE_HIGH = 0, ACPI_ACTIVE_LOW = 1, ACPI_ACTIVE_BOTH = 2.
    if raw == 1 { Polarity::ActiveLow } else { Polarity::ActiveHigh }
}

fn address_space(raw: u8) -> AddressSpace {
    // ACPI_MEMORY_RANGE = 0, ACPI_IO_RANGE = 1, ACPI_BUS_NUMBER_RANGE = 2.
    match raw {
        0 => AddressSpace::Memory,
        1 => AddressSpace::Io,
        2 => AddressSpace::BusNumber,
        other => AddressSpace::Other(other),
    }
}

/// Turns one `ACPI_RESOURCE` into a [`Resource`]. The end tag yields nothing.
///
/// # Safety
/// `resource` must point at a descriptor ACPICA produced.
unsafe fn decode(resource: *const sys::ACPI_RESOURCE) -> Option<Resource> {
    // SAFETY: the whole thing is packed, so every read is unaligned; the
    // descriptor body is what the per-type structs below describe.
    unsafe {
        let kind = ptr::addr_of!((*resource).Type).read_unaligned();
        let data = ptr::addr_of!((*resource).Data).cast::<u8>();

        Some(match kind {
            sys::ACPI_RESOURCE_TYPE_END_TAG => return None,

            sys::ACPI_RESOURCE_TYPE_IO => {
                let io: sys::ACPI_RESOURCE_IO = field(data);
                Resource::Io { min: io.Minimum, max: io.Maximum, alignment: io.Alignment, length: io.AddressLength }
            }
            sys::ACPI_RESOURCE_TYPE_FIXED_IO => {
                let io: sys::ACPI_RESOURCE_FIXED_IO = field(data);
                Resource::FixedIo { base: io.Address, length: io.AddressLength }
            }

            // The 24-bit descriptor counts in 256-byte units.
            sys::ACPI_RESOURCE_TYPE_MEMORY24 => {
                let memory: sys::ACPI_RESOURCE_MEMORY24 = field(data);
                Resource::Memory {
                    min: u64::from(memory.Minimum) << 8,
                    max: u64::from(memory.Maximum) << 8,
                    length: u64::from(memory.AddressLength) << 8,
                    writable: memory.WriteProtect == 1,
                }
            }
            sys::ACPI_RESOURCE_TYPE_MEMORY32 => {
                let memory: sys::ACPI_RESOURCE_MEMORY32 = field(data);
                Resource::Memory {
                    min: u64::from(memory.Minimum),
                    max: u64::from(memory.Maximum),
                    length: u64::from(memory.AddressLength),
                    writable: memory.WriteProtect == 1,
                }
            }
            sys::ACPI_RESOURCE_TYPE_FIXED_MEMORY32 => {
                let memory: sys::ACPI_RESOURCE_FIXED_MEMORY32 = field(data);
                Resource::Memory {
                    min: u64::from(memory.Address),
                    max: u64::from(memory.Address),
                    length: u64::from(memory.AddressLength),
                    writable: memory.WriteProtect == 1,
                }
            }

            sys::ACPI_RESOURCE_TYPE_ADDRESS16 => {
                let a: sys::ACPI_RESOURCE_ADDRESS16 = field(data);
                Resource::Address {
                    space: address_space(a.ResourceType),
                    produces: a.ProducerConsumer == 1,
                    min: u64::from(a.Address.Minimum),
                    max: u64::from(a.Address.Maximum),
                    length: u64::from(a.Address.AddressLength),
                    translation: u64::from(a.Address.TranslationOffset),
                }
            }
            sys::ACPI_RESOURCE_TYPE_ADDRESS32 => {
                let a: sys::ACPI_RESOURCE_ADDRESS32 = field(data);
                Resource::Address {
                    space: address_space(a.ResourceType),
                    produces: a.ProducerConsumer == 1,
                    min: u64::from(a.Address.Minimum),
                    max: u64::from(a.Address.Maximum),
                    length: u64::from(a.Address.AddressLength),
                    translation: u64::from(a.Address.TranslationOffset),
                }
            }
            sys::ACPI_RESOURCE_TYPE_ADDRESS64 => {
                let a: sys::ACPI_RESOURCE_ADDRESS64 = field(data);
                Resource::Address {
                    space: address_space(a.ResourceType),
                    produces: a.ProducerConsumer == 1,
                    min: a.Address.Minimum,
                    max: a.Address.Maximum,
                    length: a.Address.AddressLength,
                    translation: a.Address.TranslationOffset,
                }
            }

            sys::ACPI_RESOURCE_TYPE_IRQ => {
                let irq: sys::ACPI_RESOURCE_IRQ = field(data);
                let first = data.add(offset_of!(sys::ACPI_RESOURCE_IRQ, __bindgen_anon_1));
                Resource::Irq {
                    lines: (0..usize::from(irq.InterruptCount)).map(|i| u32::from(first.add(i).read())).collect(),
                    trigger: trigger(irq.Triggering),
                    polarity: polarity(irq.Polarity),
                    shared: irq.Shareable == 1,
                }
            }
            sys::ACPI_RESOURCE_TYPE_EXTENDED_IRQ => {
                let irq: sys::ACPI_RESOURCE_EXTENDED_IRQ = field(data);
                let first = data.add(offset_of!(sys::ACPI_RESOURCE_EXTENDED_IRQ, __bindgen_anon_1)).cast::<u32>();
                Resource::Irq {
                    lines: (0..usize::from(irq.InterruptCount)).map(|i| first.add(i).read_unaligned()).collect(),
                    trigger: trigger(irq.Triggering),
                    polarity: polarity(irq.Polarity),
                    shared: irq.Shareable == 1,
                }
            }

            sys::ACPI_RESOURCE_TYPE_DMA => {
                let dma: sys::ACPI_RESOURCE_DMA = field(data);
                let first = data.add(offset_of!(sys::ACPI_RESOURCE_DMA, __bindgen_anon_1));
                Resource::Dma { channels: (0..usize::from(dma.ChannelCount)).map(|i| first.add(i).read()).collect() }
            }

            other => Resource::Unsupported { kind: other },
        })
    }
}
