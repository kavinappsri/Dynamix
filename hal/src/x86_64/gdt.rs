//! Kernel-owned GDT and TSS.

use core::arch::asm;
use core::cell::UnsafeCell;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x18;

/// IST slot (1-based) used for #DF, NMI and #MC.
pub const IST_EMERGENCY: u8 = 1;
const IST_STACK_SIZE: usize = 16 * 1024;

const CODE_SEGMENT: u64 = 0x00AF_9A00_0000_FFFF; // present, DPL0, executable, long mode
const DATA_SEGMENT: u64 = 0x00CF_9200_0000_FFFF; // present, DPL0, writable
const TSS_TYPE_AVAILABLE_64: u64 = 0x89;

/// 64-bit TSS is 104 bytes; kept as u32 words so field offsets are explicit.
const TSS_WORDS: usize = 26;
const TSS_IST1_WORD: usize = 9; // byte offset 36
const TSS_IOMAP_WORD: usize = 25; // iomap base is the u16 at byte offset 102

#[repr(C, align(16))]
struct IstStack(UnsafeCell<[u8; IST_STACK_SIZE]>);
#[repr(C, align(16))]
struct Tss(UnsafeCell<[u32; TSS_WORDS]>);
#[repr(C, align(16))]
struct Gdt(UnsafeCell<[u64; 5]>);

// SAFETY: written once by `init` during single-core boot, read by hardware after.
unsafe impl Sync for IstStack {}
unsafe impl Sync for Tss {}
unsafe impl Sync for Gdt {}

static IST_STACK: IstStack = IstStack(UnsafeCell::new([0; IST_STACK_SIZE]));
static TSS: Tss = Tss(UnsafeCell::new([0; TSS_WORDS]));
static GDT: Gdt = Gdt(UnsafeCell::new([0; 5]));

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

/// Installs the GDT, reloads the segment registers and loads the TSS.
pub unsafe fn init() {
    let tss_base = TSS.0.get() as u64;
    let ist_top = IST_STACK.0.get() as u64 + IST_STACK_SIZE as u64;
    let tss_limit = (TSS_WORDS * 4 - 1) as u64;

    // SAFETY: statics are exclusively ours at this point.
    unsafe {
        let tss = &mut *TSS.0.get();
        tss[TSS_IST1_WORD] = ist_top as u32;
        tss[TSS_IST1_WORD + 1] = (ist_top >> 32) as u32;
        tss[TSS_IOMAP_WORD] = ((TSS_WORDS * 4) as u32) << 16; // no I/O bitmap

        let tss_low = (tss_limit & 0xFFFF)
            | ((tss_base & 0xFFFF) << 16)
            | (((tss_base >> 16) & 0xFF) << 32)
            | (TSS_TYPE_AVAILABLE_64 << 40)
            | (((tss_limit >> 16) & 0xF) << 48)
            | (((tss_base >> 24) & 0xFF) << 56);
        let gdt = &mut *GDT.0.get();
        *gdt = [0, CODE_SEGMENT, DATA_SEGMENT, tss_low, tss_base >> 32];

        let pointer = DescriptorTablePointer { limit: (5 * 8 - 1) as u16, base: gdt.as_ptr() as u64 };
        asm!(
        "lgdt [{pointer}]",
        // Reload CS with a far return, then the data segments.
        "push {cs}",
        "lea {scratch}, [rip + 2f]",
        "push {scratch}",
        "retfq",
        "2:",
        "mov ds, {ds:x}",
        "mov es, {ds:x}",
        "mov ss, {ds:x}",
        pointer = in(reg) &pointer,
        cs = const KERNEL_CS as u64,
        ds = in(reg) KERNEL_DS as u32,
        scratch = out(reg) _,
        );
        asm!("ltr {0:x}", in(reg) TSS_SELECTOR as u32, options(nomem, nostack, preserves_flags));
    }
}
