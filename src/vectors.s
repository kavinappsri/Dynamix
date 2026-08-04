.section .text.vectors
.global vector_table_el1
.global default_exception_entry

/* * Macro to push register state and call Rust exception handler.
 * Stack frame size: 288 bytes (16-byte aligned).
 */
.macro EXCEPTION_ENTRY source, kind
    /* Allocate 36 bytes of memory  */
    sub sp, sp, 288

    /* Read and save system fault context */
    mrs x0, elr_el1
    mrs x1, spsr_el1
    stp x0, x1, [sp, #16 * 16]

    mrs x0, esr_el1
    mrs x1, far_el1
    stp x0, x1,   [sp, #16 * 17]

    /* Save General Purpose Registers x0 - x29 */
    stp x0, x1, [sp, #16 * 0]
    stp x2, x3, [sp, #16 * 1]
    stp x4, x5, [sp, #16 * 2]
    stp x6, x7, [sp, #16 * 3]
    stp x8, x9, [sp, #16 * 4]
    stp x10, x11, [sp, #16 * 5]
    stp x12, x13, [sp, #16 * 6]
    stp x14, x15, [sp, #16 * 7]
    stp x16, x17, [sp, #16 * 8]
    stp x18, x19, [sp, #16 * 9]
    stp x20, x21, [sp, #16 * 10]
    stp x22, x23, [sp, #16 * 11]
    stp x24, x25, [sp, #16 * 12]
    stp x26, x27, [sp, #16 * 13]
    stp x28, x29, [sp, #16 * 14]
    str x30, [sp, #16 * 15]

    /* Prepare arguments for Rust function:
     * x0 = ExceptionContext pointer (sp)
     * x1 = source
     * x2 = kind
     */
    mov x0, sp
    mov x1, #\source
    mov x2, #\kind

    bl rust_exception_handler

    /* Restore registers and return */
    ldp x0, x1, [sp, #16 * 16]
    msr elr_el1, x0
    msr spsr_el1, x1

    ldr x30, [sp, #16 * 15]
    ldp x28, x29, [sp, #16 * 14]
    ldp x26, x27, [sp, #16 * 13]
    ldp x24, x25, [sp, #16 * 12]
    ldp x22, x23, [sp, #16 * 11]
    ldp x20, x21, [sp, #16 * 10]
    ldp x18, x19, [sp, #16 * 9]
    ldp x16, x17, [sp, #16 * 8]
    ldp x14, x15, [sp, #16 * 7]
    ldp x12, x13, [sp, #16 * 6]
    ldp x10, x11, [sp, #16 * 5]
    ldp x8, x9, [sp, #16 * 4]
    ldp x6, x7, [sp, #16 * 3]
    ldp x4, x5, [sp, #16 * 2]
    ldp x2, x3, [sp, #16 * 1]
    ldp x0, x1, [sp, #16 * 0]

    add sp, sp, #288

    udf #0
.endmacro


/*
 * The Vector Table must be aligned to 2048 bytes (2^11).
 * 4 sources (Current SP0, Current SPx, Lower AArch64, Lower AArch32) x 4 types (Sync, IRQ, FIQ, SError)
 */
.balign 2048
vector_table_el1:
    /* Current EL with SP_EL0 */
    .balign 128; EXCEPTION_ENTRY 0, 0   /* Synchronous */
    .balign 128; EXCEPTION_ENTRY 0, 1   /* IRQ */
    .balign 128; EXCEPTION_ENTRY 0, 2   /* FIQ */
    .balign 128; EXCEPTION_ENTRY 0, 3   /* SError */

    /* Current EL with SP_ELx */
    .balign 128; EXCEPTION_ENTRY 1, 0   /* Synchronous */
    .balign 128; EXCEPTION_ENTRY 1, 1   /* IRQ */
    .balign 128; EXCEPTION_ENTRY 1, 2   /* FIQ */
    .balign 128; EXCEPTION_ENTRY 1, 3   /* SError */

    /* Lower EL using AArch64 */
    .balign 128; EXCEPTION_ENTRY 2, 0   /* Synchronous */
    .balign 128; EXCEPTION_ENTRY 2, 1   /* IRQ */
    .balign 128; EXCEPTION_ENTRY 2, 2   /* FIQ */
    .balign 128; EXCEPTION_ENTRY 2, 3   /* SError */

    /* Lower EL using AArch32 */
    .balign 128; EXCEPTION_ENTRY 3, 0   /* Synchronous */
    .balign 128; EXCEPTION_ENTRY 3, 1   /* IRQ */
    .balign 128; EXCEPTION_ENTRY 3, 2   /* FIQ */
    .balign 128; EXCEPTION_ENTRY 3, 3   /* SError */