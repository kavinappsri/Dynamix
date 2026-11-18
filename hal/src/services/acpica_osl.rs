//! ACPICA's OS services layer (OSL):
//! 
//! PCI configuration access is the legacy 0xCF8/0xCFC mechanism for segment 0
//! only, kept private here; extended config space (ECAM) is not supported yet.

// OSL functions are PascalCase, defined by ACPICA
#![allow(non_snake_case)]

use alloc::alloc::{alloc, alloc_zeroed, dealloc};
use alloc::boxed::Box;
use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::driver_traits::serial::{SerialWriter, active_serial};
use crate::services::addr::{PhysAddr, PhysRange};
use crate::services::cpu::{self, IrqState};
use crate::services::interrupts::{self, Irq, IrqConfig, IrqHandle, IrqStatus, Polarity, Trigger};
use crate::services::mmio::Mmio;
use crate::services::mmu::{self, MmuError};
use crate::services::sync::IrqSpinLock;
use crate::services::{timer, work};
use crate::x86_64::port;

// -------------------------------- ACPICA types --------------------------------

/// `ACPI_STATUS`
pub type AcpiStatus = u32;

pub const AE_OK: AcpiStatus = 0x0000;
pub const AE_ERROR: AcpiStatus = 0x0001;
pub const AE_NO_MEMORY: AcpiStatus = 0x0004;
pub const AE_NOT_EXIST: AcpiStatus = 0x0006;
pub const AE_ALREADY_EXISTS: AcpiStatus = 0x0007;
pub const AE_NOT_IMPLEMENTED: AcpiStatus = 0x000E;
pub const AE_LIMIT: AcpiStatus = 0x0010;
pub const AE_TIME: AcpiStatus = 0x0011;
pub const AE_BAD_PARAMETER: AcpiStatus = 0x1001;
pub const AE_BAD_ADDRESS: AcpiStatus = 0x1009;

/// `ACPI_WAIT_FOREVER`
pub const ACPI_WAIT_FOREVER: u16 = 0xFFFF;
/// `ACPI_SIGNAL_FATAL`
pub const ACPI_SIGNAL_FATAL: u32 = 0;
/// `ACPI_SIGNAL_BREAKPOINT`
pub const ACPI_SIGNAL_BREAKPOINT: u32 = 1;

/// `ACPI_OSD_HANDLER`
pub type OsdHandler = unsafe extern "C" fn(context: *mut c_void) -> u32;
/// `ACPI_OSD_EXEC_CALLBACK`
pub type OsdExecCallback = unsafe extern "C" fn(context: *mut c_void);

/// `ACPI_PCI_ID`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AcpiPciId {
    pub segment: u16,
    pub bus: u16,
    pub device: u16,
    pub function: u16,
}

/// `ACPI_SIGNAL_FATAL_INFO`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AcpiSignalFatalInfo {
    pub kind: u32,
    pub code: u32,
    pub argument: u32,
}

/// `ACPI_INTERRUPT_HANDLED`
const ACPI_INTERRUPT_HANDLED: u32 = 1;

/// How long a semaphore wait with no timeout may last before it is declared a
/// deadlock. With no other thread to signal, such a wait cannot end by itself.
pub const FOREVER_PANIC_NS: u64 = 10_000_000_000;

// -------------------------------- Setup --------------------------------

static RSDP: AtomicU64 = AtomicU64::new(0);

/// Tells the OSL where the RSDP is. Call once, before ACPICA starts.
pub fn init(rsdp_physical_address: u64) {
    RSDP.store(rsdp_physical_address, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsInitialize() -> AcpiStatus {
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsTerminate() -> AcpiStatus {
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetRootPointer() -> u64 {
    RSDP.load(Ordering::Relaxed)
}

/// No predefined-name overrides.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsPredefinedOverride(_init_val: *const c_void, new_val: *mut *mut u8) -> AcpiStatus {
    if !new_val.is_null() {
        // SAFETY: ACPICA passes a valid out-pointer.
        unsafe { new_val.write(core::ptr::null_mut()) };
    }
    AE_OK
}

/// No table overrides.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsTableOverride(_existing: *mut c_void, new_table: *mut *mut c_void) -> AcpiStatus {
    if !new_table.is_null() {
        // SAFETY: ACPICA passes a valid out-pointer.
        unsafe { new_table.write(core::ptr::null_mut()) };
    }
    AE_OK
}

/// No physical table overrides (a zero address means "keep the original").
///
/// # Safety
/// ACPICA passes valid out-pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsPhysicalTableOverride(
    _existing: *mut c_void,
    new_address: *mut u64,
    new_length: *mut u32,
) -> AcpiStatus {
    unsafe {
        if !new_address.is_null() {
            new_address.write(0);
        }
        if !new_length.is_null() {
            new_length.write(0);
        }
    }
    AE_OK
}

// -------------------------------- Memory --------------------------------

/// `AcpiOsFree` is not told the size, so each block carries it in front.
const ALLOC_HEADER: usize = 16;
const ALLOC_ALIGN: usize = 16;

unsafe fn allocate(size: u64, zeroed: bool) -> *mut c_void {
    let Some(total) = usize::try_from(size).ok().and_then(|size| size.checked_add(ALLOC_HEADER)) else {
        return core::ptr::null_mut();
    };
    if size == 0 {
        return core::ptr::null_mut();
    }
    let Ok(layout) = Layout::from_size_align(total, ALLOC_ALIGN) else {
        return core::ptr::null_mut();
    };
    // SAFETY: `layout` has a nonzero size.
    let base = unsafe { if zeroed { alloc_zeroed(layout) } else { alloc(layout) } };
    if base.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the block is at least `ALLOC_HEADER` bytes and 16-aligned.
    unsafe {
        (base as *mut usize).write(total);
        base.add(ALLOC_HEADER) as *mut c_void
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsAllocate(size: u64) -> *mut c_void {
    // SAFETY: forwarded contract.
    unsafe { allocate(size, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsAllocateZeroed(size: u64) -> *mut c_void {
    // SAFETY: forwarded contract.
    unsafe { allocate(size, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsFree(memory: *mut c_void) {
    if memory.is_null() {
        return;
    }
    // SAFETY: `memory` came from `allocate`, so the header sits right before it.
    unsafe {
        let base = (memory as *mut u8).sub(ALLOC_HEADER);
        let total = (base as *const usize).read();
        dealloc(base, Layout::from_size_align_unchecked(total, ALLOC_ALIGN));
    }
}

/// Makes `[addr, addr + len)` reachable through the identity map.
fn ensure_mapped(addr: u64, len: u64) -> bool {
    const PAGE: u64 = 4096;
    let Some(end) = addr.checked_add(len.max(1)).and_then(|end| end.checked_add(PAGE - 1)) else {
        return false;
    };
    let start = addr & !(PAGE - 1);
    let end = end & !(PAGE - 1);

    let range = |from: u64, to: u64| PhysRange::new(PhysAddr::new(from as usize), (to - from) as usize);
    match mmu::map_device(range(start, end)) {
        Ok(()) => true,
        // Part of it is ordinary memory already; find the pages that are not mapped.
        Err(MmuError::TypeConflict) => {
            let mut page = start;
            while page < end {
                match mmu::map_device(range(page, page + PAGE)) {
                    Ok(()) | Err(MmuError::TypeConflict) => {}
                    Err(_) => return false,
                }
                page += PAGE;
            }
            true
        }
        Err(_) => false,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsMapMemory(where_: u64, length: u64) -> *mut c_void {
    if where_ == 0 || !ensure_mapped(where_, length) {
        return core::ptr::null_mut();
    }
    where_ as usize as *mut c_void
}

/// Mappings are permanent and identity-mapped; there is nothing to undo.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsUnmapMemory(_logical_address: *mut c_void, _size: u64) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsGetPhysicalAddress(logical_address: *mut c_void, physical_address: *mut u64) -> AcpiStatus {
    if logical_address.is_null() || physical_address.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: checked non-null; ACPICA passes a valid out-pointer.
    unsafe { physical_address.write(mmu::va_to_pa(logical_address as usize) as u64) };
    AE_OK
}

fn read_physical(addr: u64, width: u32) -> Result<u64, AcpiStatus> {
    let bytes: u64 = match width {
        8 | 16 | 32 | 64 => u64::from(width / 8),
        _ => return Err(AE_BAD_PARAMETER),
    };
    if !ensure_mapped(addr, bytes) {
        return Err(AE_BAD_ADDRESS);
    }
    // SAFETY: just mapped; ACPICA only asks for addresses its tables describe.
    let mmio = unsafe { Mmio::new(addr as usize) };
    Ok(if addr % bytes == 0 {
        match width {
            8 => u64::from(mmio.read8(0)),
            16 => u64::from(mmio.read16(0)),
            32 => u64::from(mmio.read32(0)),
            _ => mmio.read64(0),
        }
    } else {
        // A misaligned wide access cannot be one volatile load; do it bytewise.
        (0..bytes as usize).fold(0u64, |acc, i| acc | (u64::from(mmio.read8(i)) << (8 * i)))
    })
}

fn write_physical(addr: u64, value: u64, width: u32) -> Result<(), AcpiStatus> {
    let bytes: u64 = match width {
        8 | 16 | 32 | 64 => u64::from(width / 8),
        _ => return Err(AE_BAD_PARAMETER),
    };
    if !ensure_mapped(addr, bytes) {
        return Err(AE_BAD_ADDRESS);
    }
    // SAFETY: see `read_physical`.
    let mmio = unsafe { Mmio::new(addr as usize) };
    if addr % bytes == 0 {
        match width {
            8 => mmio.write8(0, value as u8),
            16 => mmio.write16(0, value as u16),
            32 => mmio.write32(0, value as u32),
            _ => mmio.write64(0, value),
        }
    } else {
        for i in 0..bytes as usize {
            mmio.write8(i, (value >> (8 * i)) as u8);
        }
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadMemory(address: u64, value: *mut u64, width: u32) -> AcpiStatus {
    if value.is_null() {
        return AE_BAD_PARAMETER;
    }
    match read_physical(address, width) {
        // SAFETY: checked non-null.
        Ok(read) => unsafe {
            value.write(read);
            AE_OK
        },
        Err(status) => status,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWriteMemory(address: u64, value: u64, width: u32) -> AcpiStatus {
    match write_physical(address, value, width) {
        Ok(()) => AE_OK,
        Err(status) => status,
    }
}

// -------------------------------- Port I/O --------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadPort(address: u64, value: *mut u32, width: u32) -> AcpiStatus {
    if value.is_null() {
        return AE_BAD_PARAMETER;
    }
    if address > u64::from(u16::MAX) {
        return AE_BAD_ADDRESS;
    }
    let port = address as u16;
    // SAFETY: ACPICA only asks for ports its tables and AML describe.
    let read = unsafe {
        match width {
            8 => u32::from(port::inb(port)),
            16 => u32::from(port::inw(port)),
            32 => port::inl(port),
            _ => return AE_BAD_PARAMETER,
        }
    };
    // SAFETY: checked non-null.
    unsafe { value.write(read) };
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWritePort(address: u64, value: u32, width: u32) -> AcpiStatus {
    if address > u64::from(u16::MAX) {
        return AE_BAD_ADDRESS;
    }
    let port = address as u16;
    // SAFETY: see `AcpiOsReadPort`.
    unsafe {
        match width {
            8 => port::outb(port, value as u8),
            16 => port::outw(port, value as u16),
            32 => port::outl(port, value),
            _ => return AE_BAD_PARAMETER,
        }
    }
    AE_OK
}

// -------------------------------- PCI configuration --------------------------------

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// The address/data port pair is a two-step transaction.
static PCI_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

/// The dword-aligned config address for `id`/`reg`, or why there is none.
fn pci_address(id: &AcpiPciId, reg: u32) -> Result<u32, AcpiStatus> {
    if id.segment != 0 || reg > 0xFF {
        // Other segments and extended config space need ECAM.
        return Err(AE_NOT_IMPLEMENTED);
    }
    if id.bus > 0xFF || id.device > 0x1F || id.function > 0x7 {
        return Err(AE_BAD_PARAMETER);
    }
    Ok(0x8000_0000
        | (u32::from(id.bus) << 16)
        | (u32::from(id.device) << 11)
        | (u32::from(id.function) << 8)
        | (reg & 0xFC))
}

fn pci_read_dword(address: u32) -> u32 {
    let _guard = PCI_LOCK.lock();
    // SAFETY: the standard PCI configuration mechanism #1 ports.
    unsafe {
        port::outl(PCI_CONFIG_ADDRESS, address);
        port::inl(PCI_CONFIG_DATA)
    }
}

fn pci_write_dword(address: u32, value: u32) {
    let _guard = PCI_LOCK.lock();
    // SAFETY: see `pci_read_dword`.
    unsafe {
        port::outl(PCI_CONFIG_ADDRESS, address);
        port::outl(PCI_CONFIG_DATA, value);
    }
}

/// # Safety
///
/// checked non-null; ACPICA passes valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadPciConfiguration(
    pci_id: *const AcpiPciId,
    reg: u32,
    value: *mut u64,
    width: u32,
) -> AcpiStatus {
    if pci_id.is_null() || value.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: checked non-null; ACPICA passes valid pointers.
    let id = unsafe { *pci_id };
    let base = match pci_address(&id, reg) {
        Ok(address) => address,
        Err(status) => return status,
    };
    let shift = (reg & 3) * 8;
    let read = match width {
        8 | 16 if shift + width <= 32 => u64::from(pci_read_dword(base) >> shift) & ((1u64 << width) - 1),
        32 if shift == 0 => u64::from(pci_read_dword(base)),
        64 if shift == 0 && reg + 4 <= 0xFF => {
            u64::from(pci_read_dword(base)) | (u64::from(pci_read_dword(base + 4)) << 32)
        }
        _ => return AE_BAD_PARAMETER,
    };
    // SAFETY: checked non-null.
    unsafe { value.write(read) };
    AE_OK
}

/// # Safety
///
/// checked non-null; ACPICA passes valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWritePciConfiguration(pci_id: *const AcpiPciId, reg: u32, value: u64, width: u32) -> AcpiStatus {
    if pci_id.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: checked non-null; ACPICA passes a valid pointer.
    let id = unsafe { *pci_id };
    let base = match pci_address(&id, reg) {
        Ok(address) => address,
        Err(status) => return status,
    };
    let shift = (reg & 3) * 8;
    match width {
        8 | 16 if shift + width <= 32 => {
            let mask = (((1u64 << width) - 1) as u32) << shift;
            let merged = (pci_read_dword(base) & !mask) | (((value as u32) << shift) & mask);
            pci_write_dword(base, merged);
        }
        32 if shift == 0 => pci_write_dword(base, value as u32),
        64 if shift == 0 && reg + 4 <= 0xFF => {
            pci_write_dword(base, value as u32);
            pci_write_dword(base + 4, (value >> 32) as u32);
        }
        _ => return AE_BAD_PARAMETER,
    }
    AE_OK
}

// -------------------------------- Locks --------------------------------

/// An `ACPI_SPINLOCK`: masks interrupts while held.
///
/// ACPICA's `AcpiOsAcquireLock` returns the previous interrupt state as
/// `flags`. The state is kept in the lock instead and `flags` returned as 0;
/// the result is the same as long as locks are released in the order they
/// were taken, which ACPICA does.
struct OslLock {
    held: AtomicBool,
    saved: UnsafeCell<Option<IrqState>>,
}

/// # Safety
///
/// checked non-null
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsCreateLock(out_handle: *mut *mut c_void) -> AcpiStatus {
    if out_handle.is_null() {
        return AE_BAD_PARAMETER;
    }
    let lock = Box::new(OslLock { held: AtomicBool::new(false), saved: UnsafeCell::new(None) });
    // SAFETY: checked non-null.
    unsafe { out_handle.write(Box::into_raw(lock) as *mut c_void) };
    AE_OK
}

/// # Safety
///
/// `handle` came from `AcpiOsCreateLock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsDeleteLock(handle: *mut c_void) {
    if !handle.is_null() {
        // SAFETY: `handle` came from `AcpiOsCreateLock`.
        drop(unsafe { Box::from_raw(handle as *mut OslLock) });
    }
}

/// # Safety
///
/// `handle` came from `AcpiOsCreateLock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsAcquireLock(handle: *mut c_void) -> u64 {
    // SAFETY: `handle` came from `AcpiOsCreateLock`.
    let lock = unsafe { &*(handle as *const OslLock) };
    let state = cpu::irq_save();
    // One core, interrupts masked: nobody else can hold it, so a held lock
    // means this very context took it already.
    if lock.held.swap(true, Ordering::Acquire) {
        panic!("ACPICA lock acquired recursively (single core, would deadlock)");
    }
    // SAFETY: only the holder touches `saved`, and we hold the lock.
    unsafe { *lock.saved.get() = Some(state) };
    0
}

/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReleaseLock(handle: *mut c_void, _flags: u64) {
    // SAFETY: `handle` came from `AcpiOsCreateLock`.
    let lock = unsafe { &*(handle as *const OslLock) };
    // SAFETY: the caller holds the lock.
    let state = unsafe { (*lock.saved.get()).take() };
    lock.held.store(false, Ordering::Release);
    if let Some(state) = state {
        cpu::irq_restore(state);
    }
}

// -------------------------------- Semaphores --------------------------------

/// An `ACPI_SEMAPHORE`. ACPICA's mutexes are semaphores too.
struct OslSemaphore {
    units: AtomicU32,
    max: u32,
}

/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsCreateSemaphore(max_units: u32, initial_units: u32, out_handle: *mut *mut c_void) -> AcpiStatus {
    if out_handle.is_null() || initial_units > max_units {
        return AE_BAD_PARAMETER;
    }
    let semaphore = Box::new(OslSemaphore { units: AtomicU32::new(initial_units), max: max_units });
    // SAFETY: checked non-null.
    unsafe { out_handle.write(Box::into_raw(semaphore) as *mut c_void) };
    AE_OK
}

/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsDeleteSemaphore(handle: *mut c_void) -> AcpiStatus {
    if handle.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: `handle` came from `AcpiOsCreateSemaphore`.
    drop(unsafe { Box::from_raw(handle as *mut OslSemaphore) });
    AE_OK
}

/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWaitSemaphore(handle: *mut c_void, units: u32, timeout_ms: u16) -> AcpiStatus {
    if handle.is_null() || units == 0 {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: `handle` came from `AcpiOsCreateSemaphore`.
    let semaphore = unsafe { &*(handle as *const OslSemaphore) };
    let try_take = || {
        let mut available = semaphore.units.load(Ordering::Acquire);
        loop {
            let Some(left) = available.checked_sub(units) else { return false };
            match semaphore.units.compare_exchange_weak(available, left, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true,
                Err(now) => available = now,
            }
        }
    };

    if try_take() {
        return AE_OK;
    }
    if timeout_ms == 0 {
        return AE_TIME;
    }

    let forever = timeout_ms == ACPI_WAIT_FOREVER;
    let limit_ns = if forever { FOREVER_PANIC_NS } else { u64::from(timeout_ms) * 1_000_000 };
    let start = timer::now_ns();
    loop {
        core::hint::spin_loop();
        if try_take() {
            return AE_OK;
        }
        if timer::now_ns().wrapping_sub(start) >= limit_ns {
            if forever {
                panic!("ACPICA semaphore wait cannot finish: no thread exists that could signal it");
            }
            return AE_TIME;
        }
    }
}

/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsSignalSemaphore(handle: *mut c_void, units: u32) -> AcpiStatus {
    if handle.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: `handle` came from `AcpiOsCreateSemaphore`.
    let semaphore = unsafe { &*(handle as *const OslSemaphore) };
    let mut available = semaphore.units.load(Ordering::Acquire);
    loop {
        let total = match available.checked_add(units) {
            Some(total) if total <= semaphore.max => total,
            _ => return AE_LIMIT,
        };
        match semaphore.units.compare_exchange_weak(available, total, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return AE_OK,
            Err(now) => available = now,
        }
    }
}

// -------------------------------- Threads and deferred work --------------------------------

/// There is one execution context. ACPICA needs a nonzero id.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetThreadId() -> u64 {
    1
}

#[derive(Clone, Copy)]
struct ExecSlot {
    func: usize,
    ctx: usize,
}

/// Holds `AcpiOsExecute` requests while they wait in the work queue, so that
/// queueing from an interrupt handler does not allocate.
static EXEC_POOL: IrqSpinLock<[Option<ExecSlot>; work::CAPACITY]> = IrqSpinLock::new([None; work::CAPACITY]);

fn exec_trampoline(slot: *mut ()) {
    let taken = EXEC_POOL.lock()[slot as usize].take();
    if let Some(ExecSlot { func, ctx }) = taken {
        // SAFETY: `func` was stored from an `OsdExecCallback` by `AcpiOsExecute`.
        let callback: OsdExecCallback = unsafe { core::mem::transmute::<usize, OsdExecCallback>(func) };
        // SAFETY: ACPICA's callback with the context ACPICA gave us.
        unsafe { callback(ctx as *mut c_void) };
    }
}

/// Queues `function` to run from the kernel's main loop. Callable from
/// interrupt handlers; ACPICA does exactly that for GPEs.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsExecute(_kind: u32, function: Option<OsdExecCallback>, context: *mut c_void) -> AcpiStatus {
    let Some(function) = function else { return AE_BAD_PARAMETER };

    let slot = {
        let mut pool = EXEC_POOL.lock();
        let Some(slot) = pool.iter().position(Option::is_none) else {
            return AE_NO_MEMORY;
        };
        pool[slot] = Some(ExecSlot { func: function as usize, ctx: context as usize });
        slot
    };

    if work::submit(exec_trampoline, slot as *mut ()).is_err() {
        EXEC_POOL.lock()[slot] = None;
        return AE_NO_MEMORY;
    }
    AE_OK
}

/// Runs queued work until none is left, including work that work queues.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWaitEventsComplete() {
    while work::pending() > 0 {
        work::run_pending();
    }
}

// -------------------------------- Time --------------------------------

/// Busy-waits. Does not run queued work: AML may be what called us, and
/// running more AML from in here would re-enter the interpreter.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsSleep(milliseconds: u64) {
    timer::delay_ms(milliseconds);
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsStall(microseconds: u32) {
    timer::delay_us(u64::from(microseconds));
}

/// Monotonic time in 100 ns units.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetTimer() -> u64 {
    timer::now_ns() / 100
}

// -------------------------------- Interrupts --------------------------------

const MAX_INTERRUPT_HANDLERS: usize = 4;

/// ACPICA's handler and context for each installed slot. Atomics, so the
/// interrupt trampoline never needs a lock.
static HANDLER_FN: [AtomicUsize; MAX_INTERRUPT_HANDLERS] = [const { AtomicUsize::new(0) }; MAX_INTERRUPT_HANDLERS];
static HANDLER_CTX: [AtomicUsize; MAX_INTERRUPT_HANDLERS] = [const { AtomicUsize::new(0) }; MAX_INTERRUPT_HANDLERS];

struct Installed {
    number: u32,
    handle: IrqHandle,
}

static INSTALLED: IrqSpinLock<[Option<Installed>; MAX_INTERRUPT_HANDLERS]> =
    IrqSpinLock::new([const { None }; MAX_INTERRUPT_HANDLERS]);

/// How the line behind an ACPICA interrupt number (a GSI) is wired.
///
/// ISA lines honor the MADT's overrides; everything else is a PCI-style
/// level-triggered, active-low line.
fn interrupt_config(number: u32) -> Option<IrqConfig> {
    if number < 16 {
        interrupts::isa_irq(number as u8)
    } else {
        Some(IrqConfig { irq: Irq(number), trigger: Trigger::Level, polarity: Polarity::ActiveLow })
    }
}

fn interrupt_trampoline(_irq: Irq, slot: *mut ()) -> IrqStatus {
    let slot = slot as usize;
    let handler = HANDLER_FN[slot].load(Ordering::Acquire);
    if handler == 0 {
        return IrqStatus::NotMine;
    }
    // SAFETY: `handler` was stored from an `OsdHandler` by `AcpiOsInstallInterruptHandler`.
    let handler: OsdHandler = unsafe { core::mem::transmute::<usize, OsdHandler>(handler) };
    // SAFETY: ACPICA's handler with the context ACPICA gave us.
    let result = unsafe { handler(HANDLER_CTX[slot].load(Ordering::Relaxed) as *mut c_void) };
    if result == ACPI_INTERRUPT_HANDLED { IrqStatus::Handled } else { IrqStatus::NotMine }
}

/// Installs ACPICA's SCI handler (the only interrupt ACPICA asks for).
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsInstallInterruptHandler(number: u32, handler: Option<OsdHandler>, context: *mut c_void) -> AcpiStatus {
    let Some(handler) = handler else { return AE_BAD_PARAMETER };
    let Some(config) = interrupt_config(number) else { return AE_BAD_PARAMETER };
    if !interrupts::is_ready() {
        return AE_ERROR;
    }

    let mut installed = INSTALLED.lock();
    if installed.iter().flatten().any(|entry| entry.number == number) {
        return AE_ALREADY_EXISTS;
    }
    let Some(slot) = installed.iter().position(Option::is_none) else {
        return AE_NO_MEMORY;
    };

    HANDLER_CTX[slot].store(context as usize, Ordering::Relaxed);
    HANDLER_FN[slot].store(handler as usize, Ordering::Release);
    match interrupts::request(config.irq, config.trigger, config.polarity, interrupt_trampoline, slot as *mut ()) {
        Ok(handle) => {
            installed[slot] = Some(Installed { number, handle });
            AE_OK
        }
        Err(_) => {
            HANDLER_FN[slot].store(0, Ordering::Release);
            AE_ERROR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsRemoveInterruptHandler(number: u32, handler: Option<OsdHandler>) -> AcpiStatus {
    let Some(handler) = handler else { return AE_BAD_PARAMETER };

    let (slot, entry) = {
        let mut installed = INSTALLED.lock();
        let found = installed.iter().enumerate().position(|(slot, entry)| {
            entry.as_ref().is_some_and(|entry| entry.number == number)
                && HANDLER_FN[slot].load(Ordering::Relaxed) == handler as usize
        });
        let Some(slot) = found else { return AE_NOT_EXIST };
        (slot, installed[slot].take())
    };

    // The lock is released first: unregistering waits for a dispatch that is
    // still running, and that dispatch must not be stuck behind our lock.
    if let Some(entry) = entry {
        let _ = interrupts::unregister(entry.handle);
    }
    HANDLER_FN[slot].store(0, Ordering::Release);
    AE_OK
}

// -------------------------------- Output and signals --------------------------------

/// Writes raw bytes to the kernel's serial console. The target for ACPICA's
/// formatted output; see the module docs about `AcpiOsPrintf`.
///
/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dynamix_acpica_log_write(buffer: *const u8, length: usize) {
    let Some(uart) = active_serial() else { return };
    if buffer.is_null() {
        return;
    }
    // SAFETY: the caller passes `length` readable bytes.
    for &byte in unsafe { core::slice::from_raw_parts(buffer, length) } {
        uart.write_byte(byte);
    }
}

/// Fatal AML opcodes and breakpoints are logged and otherwise ignored, as
/// real firmware is known to trigger them harmlessly.
///
/// # Safety
///
/// See function definition
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsSignal(function: u32, info: *mut c_void) -> AcpiStatus {
    let uart = active_serial();
    match function {
        ACPI_SIGNAL_FATAL => {
            if let Some(uart) = uart {
                if info.is_null() {
                    let _ = writeln!(SerialWriter(uart), "[acpica] fatal AML opcode");
                } else {
                    // SAFETY: for this function ACPICA passes an `ACPI_SIGNAL_FATAL_INFO`.
                    let info = unsafe { *(info as *const AcpiSignalFatalInfo) };
                    let _ = writeln!(
                        SerialWriter(uart),
                        "[acpica] fatal AML opcode: type={:#x} code={:#x} argument={:#x}",
                        info.kind,
                        info.code,
                        info.argument
                    );
                }
            }
            AE_OK
        }
        ACPI_SIGNAL_BREAKPOINT => {
            if let Some(uart) = uart {
                let _ = writeln!(SerialWriter(uart), "[acpica] AML breakpoint");
            }
            AE_OK
        }
        _ => AE_BAD_PARAMETER,
    }
}

/// Nothing to do before entering a sleep state.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsEnterSleep(_sleep_state: u8, _rega_value: u32, _regb_value: u32) -> AcpiStatus {
    AE_OK
}
