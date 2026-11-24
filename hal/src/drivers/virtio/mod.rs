//! virtio over memory-mapped transports


#![allow(dead_code)] // all codes documented, not used on platforms without virtio

#[cfg(feature = "driver-virtio-input")]
mod input;

use core::sync::atomic::{AtomicU16, Ordering, fence};

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::services::device::{Device, Resource};
use crate::services::mmio::Mmio;
use crate::services::pmm;
use crate::services::provider;

const MAGIC: usize = 0x000;
const VERSION: usize = 0x004;
const DEVICE_ID: usize = 0x008;
const DEVICE_FEATURES: usize = 0x010;
const DEVICE_FEATURES_SEL: usize = 0x014;
const DRIVER_FEATURES: usize = 0x020;
const DRIVER_FEATURES_SEL: usize = 0x024;
const GUEST_PAGE_SIZE: usize = 0x028;
const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_ALIGN: usize = 0x03C;
const QUEUE_PFN: usize = 0x040;
const QUEUE_READY: usize = 0x044;
const QUEUE_NOTIFY: usize = 0x050;
const INTERRUPT_STATUS: usize = 0x060;
const INTERRUPT_ACK: usize = 0x064;
const STATUS: usize = 0x070;
const QUEUE_DESC_LOW: usize = 0x080;
const QUEUE_DRIVER_LOW: usize = 0x090;
const QUEUE_DEVICE_LOW: usize = 0x0A0;
/// Start of the device-specific configuration space.
pub(crate) const CONFIG: usize = 0x100;

const MAGIC_VALUE: u32 = 0x7472_6976; // "virt"

const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;
const STATUS_FAILED: u32 = 128;

/// Feature bit 32, in the second feature word.
const VERSION_1_WORD: u32 = 1;
const VERSION_1_BIT: u32 = 1;

const DESCRIPTOR_DEVICE_WRITES: u16 = 2;

/// Device kind 18.
const DEVICE_INPUT: u32 = 18;

/// A virtio device's register window.
#[derive(Clone, Copy)]
pub struct Transport {
    regs: Mmio,
    modern: bool,
}

impl Transport {
    pub(crate) fn regs(&self) -> Mmio {
        self.regs
    }

    pub(crate) fn reset(&self) {
        self.regs.write32(STATUS, 0);
        for _ in 0..10_000 {
            if self.regs.read32(STATUS) == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    /// Resets the device and negotiates no optional features (a modern device
    /// is only asked for `VERSION_1`). Queues can be set up afterwards.
    pub(crate) fn begin(&self) -> Result<(), ProbeError> {
        self.reset();
        self.regs.write32(STATUS, STATUS_ACKNOWLEDGE);
        self.regs.write32(STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        if self.modern {
            self.regs.write32(DEVICE_FEATURES_SEL, VERSION_1_WORD);
            if self.regs.read32(DEVICE_FEATURES) & VERSION_1_BIT == 0 {
                self.fail();
                return Err(ProbeError::InitFailed);
            }
            self.regs.write32(DRIVER_FEATURES_SEL, 0);
            self.regs.write32(DRIVER_FEATURES, 0);
            self.regs.write32(DRIVER_FEATURES_SEL, VERSION_1_WORD);
            self.regs.write32(DRIVER_FEATURES, VERSION_1_BIT);

            let status = STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK;
            self.regs.write32(STATUS, status);
            if self.regs.read32(STATUS) & STATUS_FEATURES_OK == 0 {
                self.fail();
                return Err(ProbeError::InitFailed);
            }
        } else {
            self.regs.write32(DRIVER_FEATURES, 0);
            self.regs.write32(GUEST_PAGE_SIZE, pmm::FRAME_SIZE as u32);
        }
        Ok(())
    }

    /// Tells the device the driver is ready; call once the queues are set up.
    pub(crate) fn finish(&self) {
        let features_ok = if self.modern { STATUS_FEATURES_OK } else { 0 };
        self.regs.write32(STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | features_ok | STATUS_DRIVER_OK);
    }

    pub(crate) fn fail(&self) {
        self.regs.write32(STATUS, STATUS_FAILED);
    }

    /// Reads and acknowledges the interrupt status; zero means it was not this device.
    pub(crate) fn ack_interrupt(&self) -> u32 {
        let status = self.regs.read32(INTERRUPT_STATUS);
        if status != 0 {
            self.regs.write32(INTERRUPT_ACK, status);
        }
        status
    }
}

/// Entries in every queue this module sets up. Small on purpose; the drivers
/// so far keep a handful of buffers outstanding.
pub(crate) const QUEUE_SIZE: u16 = 16;

/// Offset in the queue's memory where free space for buffers starts.
const SCRATCH_OFFSET: usize = pmm::FRAME_SIZE + 512;

/// One split virtqueue of [`QUEUE_SIZE`] entries, in two frames:
///
/// ```text
/// 0      descriptor table (256 bytes)
/// 256    available ring
/// 4096   used ring            (legacy transports want it page aligned)
/// 4608   scratch space for the driver's buffers
/// ```
pub(crate) struct Queue {
    index: u16,
    base: usize,
    last_used: AtomicU16,
    avail_idx: AtomicU16,
}

impl Queue {
    const AVAIL: usize = 256;
    const USED: usize = pmm::FRAME_SIZE;

    /// Allocates queue `index` and hands it to the device.
    pub(crate) fn new(transport: &Transport, index: u16) -> Result<Queue, ProbeError> {
        let regs = transport.regs;
        regs.write32(QUEUE_SEL, u32::from(index));
        let max = regs.read32(QUEUE_NUM_MAX);
        if max < u32::from(QUEUE_SIZE) {
            return Err(ProbeError::InitFailed);
        }

        let frame = pmm::alloc_frames(1).map_err(|_| ProbeError::MappingFailed)?;
        let base = frame.addr().as_usize();
        // SAFETY: two frames from the physical allocator, identity mapped and ours.
        unsafe { core::ptr::write_bytes(base as *mut u8, 0, 2 * pmm::FRAME_SIZE) };
        let queue = Queue { index, base, last_used: AtomicU16::new(0), avail_idx: AtomicU16::new(0) };

        regs.write32(QUEUE_NUM, u32::from(QUEUE_SIZE));
        if transport.modern {
            for (offset, address) in [
                (QUEUE_DESC_LOW, base),
                (QUEUE_DRIVER_LOW, base + Self::AVAIL),
                (QUEUE_DEVICE_LOW, base + Self::USED),
            ] {
                regs.write32(offset, address as u32);
                regs.write32(offset + 4, ((address as u64) >> 32) as u32);
            }
            regs.write32(QUEUE_READY, 1);
        } else {
            regs.write32(QUEUE_ALIGN, pmm::FRAME_SIZE as u32);
            regs.write32(QUEUE_PFN, (base >> 12) as u32);
        }
        Ok(queue)
    }

    /// Start of the free space after the rings; at least 3 KiB.
    pub(crate) fn scratch(&self) -> usize {
        self.base + SCRATCH_OFFSET
    }

    /// Describes `buffer` (device-writable, `len` bytes) as descriptor `id`.
    pub(crate) fn set_device_buffer(&self, id: u16, buffer: usize, len: u32) {
        let at = (self.base + usize::from(id) * 16) as *mut u8;
        // SAFETY: `id < QUEUE_SIZE` by the callers' construction, so the
        // descriptor lies inside the table.
        unsafe {
            at.cast::<u64>().write_volatile(buffer as u64);
            at.add(8).cast::<u32>().write_volatile(len);
            at.add(12).cast::<u16>().write_volatile(DESCRIPTOR_DEVICE_WRITES);
            at.add(14).cast::<u16>().write_volatile(0);
        }
    }

    /// Makes descriptor `id` available to the device.
    pub(crate) fn submit(&self, id: u16) {
        let slot = self.avail_idx.load(Ordering::Relaxed);
        let ring = (self.base + Self::AVAIL + 4 + 2 * usize::from(slot % QUEUE_SIZE)) as *mut u16;
        let idx = (self.base + Self::AVAIL + 2) as *mut u16;
        // SAFETY: both addresses are inside the available ring.
        unsafe {
            ring.write_volatile(id);
            fence(Ordering::SeqCst);
            idx.write_volatile(slot.wrapping_add(1));
        }
        self.avail_idx.store(slot.wrapping_add(1), Ordering::Relaxed);
    }

    /// Tells the device there is something new in the available ring.
    pub(crate) fn notify(&self, transport: &Transport) {
        fence(Ordering::SeqCst);
        transport.regs.write32(QUEUE_NOTIFY, u32::from(self.index));
    }

    /// The next descriptor the device has finished with, and how many bytes it wrote.
    pub(crate) fn pop_used(&self) -> Option<(u16, u32)> {
        fence(Ordering::SeqCst);
        let used = self.base + Self::USED;
        // SAFETY: inside the used ring.
        let device_idx = unsafe { ((used + 2) as *const u16).read_volatile() };
        let last = self.last_used.load(Ordering::Relaxed);
        if device_idx == last {
            return None;
        }

        let element = (used + 4 + 8 * usize::from(last % QUEUE_SIZE)) as *const u32;
        // SAFETY: inside the used ring.
        let (id, len) = unsafe { (element.read_volatile(), element.add(1).read_volatile()) };
        self.last_used.store(last.wrapping_add(1), Ordering::Relaxed);
        // A device naming a descriptor we never made is ignored.
        u16::try_from(id).ok().filter(|id| *id < QUEUE_SIZE).map(|id| (id, len))
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    let regs = dev.mem(0)?.mmio();
    if regs.read32(MAGIC) != MAGIC_VALUE {
        return Err(ProbeError::NotPresent);
    }
    let modern = match regs.read32(VERSION) {
        1 => false,
        2 => true,
        _ => return Err(ProbeError::Firmware),
    };

    // QEMU's `virt` has 32 slots and most of them are empty; that is a slot
    // with nothing behind it, not a failure.
    let child = match regs.read32(DEVICE_ID) {
        DEVICE_INPUT => ("virtio-input", "virtio,input"),
        _ => return Ok(()),
    };

    // Defers until the interrupt controller this slot's line goes to has bound.
    let irq = dev.irq(0)?;
    provider::register(dev, Transport { regs, modern });
    dev.spawn(child.0, &[child.1], &[Resource::Irq(irq)]);
    Ok(())
}

driver! {
    name: "virtio-mmio",
    matches: [Match::Fdt("virtio,mmio")],
    probe: probe,
}
