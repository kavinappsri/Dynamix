.section .text._start
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