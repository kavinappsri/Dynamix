.section .text.boot, "ax"
.global _start

_start:
    /* ARM32 zImage Boot Header */
    b _execution_entry   /* 0x00 Branch Intruction */
    .word 0x016f2818     /* 0x04 Magic */
    .word _start         /* 0x08 zImage Start Address */
    .word _image_size    /* zImage Size Marker */

_execution_entry:
    /* Ensure supervisor mode & disable IRQ/FIRQ */
    cpsid if, #0x13

    /* Save dtb ptr to r9 from r2 */
    mov r9, r2

    /* Get physical adress of _start, linked kernel vma start */
    adr r10, _start
    ldr r11, =_kernel_vma_start

    /* paddr of .boottables */
    ldr r12, =_boottables_start
    sub r12, r12, r11
    add r12, r12, r10

    /* Zero two 16 KiB L1 tables plus 32 KiB of TTBR1 L2 tables. */
    mov r0, r12
    mov r1, #(4 * 16 * 1024)
    add r1, r0, r1                 /* r1 = end address for zero loop */
    mov r2, #0
    mov r3, #0
    mov r4, #0
    mov r5, #0

_zero_boottables:
    cmp r0, r1
    bhs _zero_boottables_done
    stmia r0!, {{r2, r3, r4, r5}}
    b _zero_boottables

_zero_boottables_done:

    /* Table layout within the 64KB .boottables region:
       r12 + 0x0000 : TTBR0 L1 (identity)
       r12 + 0x4000 : TTBR1 L1 (higher-half)
       r12 + 0x8000 : 32 TTBR1 coarse L2 tables */
    mov r4, r12                    /* r4 = TTBR0 L1 base (physical) */
    add r5, r12, #0x4000           /* r5 = TTBR1 L1 base (physical) */

    /* Section descriptor attributes for Normal, WB/WA, Shareable, full access, domain 0, executable*/
    ldr r0, =0x00011c0e

    /* Get aligned section base */
    lsr r7, r10, #20
    lsl r7, r7, #20

    /* TTBR0 identity map: 32 x 1MB sections starting at the physical load address. */
    lsr r1, r7, #20               /* r1 = starting L1 section index */
    mov r2, #32                    /* r2 = remaining sections to write */
    mov r3, r7                    /* r3 = running physical address */

_ttbr0_section_loop:
    orr r6, r3, r0                  /* r6 = section descriptor for this 1MB */
    str r6, [r4, r1, lsl #2]
    add r1, r1, #1
    add r3, r3, #0x00100000         /* += 1MB */
    subs r2, r2, #1
    bne _ttbr0_section_loop

    /*
     * QEMU enters at a 64 KiB offset within its physical MiB, while the
     * fixed higher-half VMA begins at a MiB boundary. A section descriptor
     * cannot represent that offset, so TTBR1 uses coarse L2 tables with
     * 4 KiB small-page descriptors.
     */
    lsr r1, r11, #20
    mov r2, #32
    mov r3, r10                    /* Actual physical address of _start. */
    add r8, r5, #0x4000            /* First 1 KiB TTBR1 L2 table. */
    ldr r0, =0x0000047e            /* Normal WB/WA, shareable, full RW. */

_ttbr1_l1_loop:
    /* L1 coarse-table descriptor, domain 0. */
    orr r6, r8, #0x01
    str r6, [r5, r1, lsl #2]

    mov r6, #256
_ttbr1_l2_loop:
    /* VA = _kernel_vma_start + offset; PA = runtime physical base + offset. */
    orr r12, r3, r0
    str r12, [r8], #4
    add r3, r3, #0x1000
    subs r6, r6, #1
    bne _ttbr1_l2_loop

    add r1, r1, #1
    subs r2, r2, #1
    bne _ttbr1_l1_loop

    /* Domain 0 = client (respect AP bits in every descriptor that uses domain 0). */
    ldr r0, =0x00000001
    mcr p15, 0, r0, c3, c0, 0      /* DACR */

    /* TTBCR.N = 1: TTBR0 covers [0, 0x80000000), TTBR1 covers the rest.*/
    mov r0, #1
    mcr p15, 0, r0, c2, c0, 2      /* TTBCR */

    /* TTBR0 / TTBR1 - physical base addresses of the two L1 tables, plus
    walk-attribute bits. */
    orr r1, r4, #0x0B
    mcr p15, 0, r1, c2, c0, 0      /* TTBR0 */
    orr r1, r5, #0x0B
    mcr p15, 0, r1, c2, c0, 1      /* TTBR1 */

    /* Invalidate TLB and caches */
    mov r0, #0
    mcr p15, 0, r0, c8, c7, 0      /* TLBIALL */
    mcr p15, 0, r0, c7, c5, 0      /* ICIALLU (invalidate icache) */
    dsb
    isb

    /* Enable the MMU (SCTLR.M) plus data/instruction caching. */
    mrc p15, 0, r0, c1, c0, 0      /* SCTLR */
    orr r0, r0, #(1 << 0)          /* M: MMU enable */
    orr r0, r0, #(1 << 2)          /* C: data cache enable */
    orr r0, r0, #(1 << 12)         /* I: instruction cache enable */
    mcr p15, 0, r0, c1, c0, 0
    isb

    /* From this point on, virtual memory is active */
    ldr r0, =_hh_entry
    bx r0

    /* Unreachable */
_hang_boot:
    wfe
    b _hang_boot

/* -------------------------------> HH ENTRY <-------------------------------- */
.section .text._hh_entry, "ax"
.global _hh_entry
.arm

_hh_entry:
    /* Initialize sp */
    ldr r0, =_stack_top
    bic r0, r0, #7
    mov sp, r0

    /* Enable access to SIMD & FP registers */
    mrc p15, 0, r0, c1, c0, 2
    orr r0, r0, #(0xF << 20)
    mcr p15, 0, r0, c1, c0, 2
    isb

    /* Turn on VFP / NEON hardware */
    mov r0, #(1 << 30)
    vmsr fpexc, r0

    /* Zero-initialize the .bss section */
    ldr r0, =_bss_start
    ldr r1, =_bss_end
    mov r2, #0

bss_loop:
    /* Clear bss */
    cmp r0, r1
    bhs bss_done
    str r2, [r0], #4
    b bss_loop

bss_done:
    /* Reset dtb ptr to r0 for rust_main */
    mov r0, r9
    bl rust_main

hang:
    wfe
    b hang
