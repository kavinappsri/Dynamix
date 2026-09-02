.section .text.boot, "ax"
.global _start

_start:
    /* ARM64 Boot Header */
    b _execution_entry    /* 0x00: Branch instruction over header */
    .word 0               /* 0x04: Reserved */
    .quad 0               /* 0x08: text_offset (0 = 2MB aligned) */
    .quad _image_size     /* 0x10: Image size (computed by linker script) */
    .quad 0x2             /* 0x18: Flags (0x2 = Little Endian, 4KB page size) */
    .quad 0               /* 0x20: Reserved */
    .quad 0               /* 0x28: Reserved */
    .quad 0               /* 0x30: Reserved */
    .ascii "ARM\x64"      /* 0x38: Magic string (0x644d5241) */
    .word 0               /* 0x3C: Reserved (PE header offset if EFI) */

_execution_entry:
    /* Save dtb pointer in x19 from x0 */
    mov x19, x0

    /* Get the physical adress of _start */
    adrp x20, _start
    add x20, x20, #:lo12:_start

    /* Get _kernel_vma_start */
    movz x21, #:abs_g3:_kernel_vma_start
    movk x21, #:abs_g2_nc:_kernel_vma_start
    movk x21, #:abs_g1_nc:_kernel_vma_start
    movk x21, #:abs_g0_nc:_kernel_vma_start

    /* Get physical address of .boottables */
    movz x22, #:abs_g3:_boottables_start
    movk x22, #:abs_g2_nc:_boottables_start
    movk x22, #:abs_g1_nc:_boottables_start
    movk x22, #:abs_g0_nc:_boottables_start

    sub x22, x22, x21
    add x22, x22, x20

    /* Zero all 4 boot table pages */
    mov x0, x22
    mov x1, #(4 * 4096)
    add x1, x0, x1                   /* x0 now has end address for zero loop */

_zero_boottables:
    cmp  x0, x1
    b.hs _zero_boottables_done
    stp  xzr, xzr, [x0], #16
    b    _zero_boottables

_zero_boottables_done:
    movz x23, #:abs_g3:_stack_top
    movk x23, #:abs_g2_nc:_stack_top
    movk x23, #:abs_g1_nc:_stack_top
    movk x23, #:abs_g0_nc:_stack_top

    /* Set sp to .stack */
    sub x23, x23, x21
    add x23, x23, x20
    mov sp, x23

    /* Table layout within the 4-page .boottables region
    x22 + 0*4096 : TTBR0 L1 (identity)
    x22 + 1*4096 : TTBR0 L2 (identity)
    x22 + 2*4096 : TTBR1 L1 (higher-half)
    x22 + 3*4096 : TTBR1 L2 (higher-half)*/
    mov x24, x22                  /* x24 = TTBR0 L1 base */
    add x25, x22, #4096           /* x25 = TTBR0 L2 base */
    add x26, x22, #8192           /* x26 = TTBR1 L1 base */
    add x27, x22, #12288          /* x27 = TTBR1 L2 base */

    /* L2 Block descriptot for normal ram */
    mov x0, #0x701

    /* !assert x20 is always 2MB aligned */

    /* TTBR0 identity map */
    lsr x1, x20, #21              /* x1 = phys_load_addr >> 21 */
    and x1, x1, #0x1FF            /* x1 = starting L2 index */
    mov x2, #16                   /* x2 = remaining entries to write */
    mov x3, x20                   /* x3 = running physical address */

ttbr0_l2_loop:
    orr x4, x3, x0                /* x4 = block descriptor for this 2MB */
    str x4, [x25, x1, lsl #3]
    add x1, x1, #1
    add x3, x3, #0x200000         /* += 2MB */
    subs x2, x2, #1
    b.ne ttbr0_l2_loop

    /* TTBR0 L1 - table descriptor pointing at above l2 entry */
    lsr x1, x20, #30
    and x1, x1, #0x1FF
    orr x4, x25, #0b11            /* valid(bit0) | table(bit1) */
    str x4, [x24, x1, lsl #3]

    /* TTBR1 Higher halh map */
    lsr x1, x21, #21
    and x1, x1, #0x1FF
    mov x2, #16
    mov x3, x20

ttbr1_l2_loop:
    orr x4, x3, x0
    str x4, [x27, x1, lsl #3]
    add x1, x1, #1
    add x3, x3, #0x200000
    subs x2, x2, #1
    b.ne ttbr1_l2_loop

    lsr x1, x21, #30
    and x1, x1, #0x1FF
    orr x4, x27, #0b11
    str x4, [x26, x1, lsl #3]

    /* Program MAIR_EL1, TCR_EL1, TTBR0_EL1, TTBR1_EL1 */
    /* MAIR indexes - 0. 0xFF Normal; 1. 0x00 Device nGnRnE; 2. 0x44 Normal Non-Cacheable*/
    movz x0, #0x00ff
    movk x0, #0x0044, lsl #16
    msr mair_el1, x0

    movz x0, #0x3519
    movk x0, #0xb519, lsl #16
    movk x0, #1, lsl #32
    msr tcr_el1, x0

    msr ttbr0_el1, x24
    msr ttbr1_el1, x26

    dsb ish
    isb

    tlbi vmalle1
    dsb ish
    isb

    /* Enable the MMU (SCTLR_EL1.M) plus instruction/data caching. */
    mrs x0, sctlr_el1
    orr x0, x0, #(1 << 0)
    orr x0, x0, #(1 << 2)
    orr x0, x0, #(1 << 12)
    msr sctlr_el1, x0
    isb

    /* From this pt onward, Virtual memory is active */
    movz x0, #:abs_g3:_hh_entry
    movk x0, #:abs_g2_nc:_hh_entry
    movk x0, #:abs_g1_nc:_hh_entry
    movk x0, #:abs_g0_nc:_hh_entry
    br   x0

    /* Unreachable. */
_hang_boot:
    wfe
    b _hang_boot

/* -------------------------------> HH ENTRY <-------------------------------- */
.section .text._hh_entry, "ax"
.global _hh_entry

_hh_entry:
    /*Initialize stack pointer & 16 bit alignment*/
    ldr x0, =_stack_top
    and x0, x0, #~0xF
    mov sp, x0

    /*Enable Access to SIMD and FP registers*/
    mrs x0, cpacr_el1
    orr x0, x0, #(3 << 20)
    msr cpacr_el1, x0
    isb

    /*Zero-initialize the .bss section */
    ldr x0, =_bss_start
    ldr x1, =_bss_end

bss_loop:
    /*Clear .bss / Ram */
    cmp x0, x1
    b.hs bss_done
    str xzr, [x0], #8
    b bss_loop

bss_done:
    /* Reset dtb pointer back to x0 */
    mov x0, x19
    bl rust_main

hang:
    wfe
    b hang
