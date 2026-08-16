.section .text._start
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

    /* Save dtb ptr to r10 from r2 */
    mov r10, r2

    /* Initialize sp */
    ldr r0, =_stack_top
    bic r0, r0, #7
    mov sp, r0

    /* Enable acess to SIMD & FP registers */
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
    mov r0, r10
    bl rust_main

hang:
    wfe
    b hang

