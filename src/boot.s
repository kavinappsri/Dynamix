.section .text._start
.global _start

_start:
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
    bl rust_main

hang:
    wfe
    b hang