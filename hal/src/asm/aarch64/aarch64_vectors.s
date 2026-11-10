.section .text.vectors
.global vector_table_el1

/*
 * Frame layout, shared with `ExceptionContext` in aarch64_exceptions.rs
 * (the Rust side asserts the size):
 *   x0..x29 @ 0..240, x30 @ 240, elr @ 248, spsr @ 256, esr @ 264, far @ 272
 */
.equ FRAME_SIZE, 288
/* q0..q31 (512 bytes), then fpsr and fpcr (16 bytes) */
.equ FP_FRAME_SIZE, 528

.macro SAVE_CONTEXT
    sub sp, sp, #FRAME_SIZE

    /* x0/x1 first: everything below clobbers them as scratch */
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
    str x30, [sp, #240]

    mrs x0, elr_el1
    mrs x1, spsr_el1
    stp x0, x1, [sp, #248]
    mrs x0, esr_el1
    mrs x1, far_el1
    stp x0, x1, [sp, #264]
.endm

.macro RESTORE_CONTEXT
    ldp x0, x1, [sp, #248]
    msr elr_el1, x0
    msr spsr_el1, x1

    ldr x30, [sp, #240]
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

    add sp, sp, #FRAME_SIZE
.endm

/*
 * One 128-byte vector slot is only room for a branch, so every slot just
 * jumps to an out-of-line entry below. (The bodies are far larger than 128
 * bytes: laying them out inline would shift every later slot off its
 * architectural offset.)
 */
.macro VECTOR target
    .balign 128
    b \target
.endm

/* Fatal path: report and halt. Never returns. */
.macro FAULT_ENTRY source, kind
fault_\source\()_\kind:
    SAVE_CONTEXT
    mov x0, sp
    mov x1, #\source
    mov x2, #\kind
    bl rust_exception_handler
1:  wfe
    b 1b
.endm

/*
 * The vector table must be aligned to 2048 bytes (2^11).
 * 4 sources (Current SP0, Current SPx, Lower AArch64, Lower AArch32) x
 * 4 types (Sync, IRQ, FIQ, SError)
 */
.balign 2048
vector_table_el1:
    /* Current EL with SP_EL0 */
    VECTOR fault_0_0
    VECTOR fault_0_1
    VECTOR fault_0_2
    VECTOR fault_0_3

    /* Current EL with SP_ELx: where the kernel runs. IRQs are serviced. */
    VECTOR fault_1_0
    VECTOR irq_entry_el1
    VECTOR fault_1_2
    VECTOR fault_1_3

    /* Lower EL using AArch64 */
    VECTOR fault_2_0
    VECTOR fault_2_1
    VECTOR fault_2_2
    VECTOR fault_2_3

    /* Lower EL using AArch32 */
    VECTOR fault_3_0
    VECTOR fault_3_1
    VECTOR fault_3_2
    VECTOR fault_3_3

FAULT_ENTRY 0, 0
FAULT_ENTRY 0, 1
FAULT_ENTRY 0, 2
FAULT_ENTRY 0, 3
FAULT_ENTRY 1, 0
FAULT_ENTRY 1, 2
FAULT_ENTRY 1, 3
FAULT_ENTRY 2, 0
FAULT_ENTRY 2, 1
FAULT_ENTRY 2, 2
FAULT_ENTRY 2, 3
FAULT_ENTRY 3, 0
FAULT_ENTRY 3, 1
FAULT_ENTRY 3, 2
FAULT_ENTRY 3, 3

/*
 * IRQ from the kernel itself. The interrupted code may be using the SIMD
 * registers (the Rust compiler freely does), so they are saved too, since the
 * Rust handler is free to clobber them.
 */
irq_entry_el1:
    SAVE_CONTEXT
    sub sp, sp, #FP_FRAME_SIZE
    stp q0, q1, [sp, #32 * 0]
    stp q2, q3, [sp, #32 * 1]
    stp q4, q5, [sp, #32 * 2]
    stp q6, q7, [sp, #32 * 3]
    stp q8, q9, [sp, #32 * 4]
    stp q10, q11, [sp, #32 * 5]
    stp q12, q13, [sp, #32 * 6]
    stp q14, q15, [sp, #32 * 7]
    stp q16, q17, [sp, #32 * 8]
    stp q18, q19, [sp, #32 * 9]
    stp q20, q21, [sp, #32 * 10]
    stp q22, q23, [sp, #32 * 11]
    stp q24, q25, [sp, #32 * 12]
    stp q26, q27, [sp, #32 * 13]
    stp q28, q29, [sp, #32 * 14]
    stp q30, q31, [sp, #32 * 15]
    mrs x0, fpsr
    mrs x1, fpcr
    str x0, [sp, #512]
    str x1, [sp, #520]

    add x0, sp, #FP_FRAME_SIZE
    bl aarch64_irq_entry

    ldr x0, [sp, #512]
    ldr x1, [sp, #520]
    msr fpsr, x0
    msr fpcr, x1
    ldp q0, q1, [sp, #32 * 0]
    ldp q2, q3, [sp, #32 * 1]
    ldp q4, q5, [sp, #32 * 2]
    ldp q6, q7, [sp, #32 * 3]
    ldp q8, q9, [sp, #32 * 4]
    ldp q10, q11, [sp, #32 * 5]
    ldp q12, q13, [sp, #32 * 6]
    ldp q14, q15, [sp, #32 * 7]
    ldp q16, q17, [sp, #32 * 8]
    ldp q18, q19, [sp, #32 * 9]
    ldp q20, q21, [sp, #32 * 10]
    ldp q22, q23, [sp, #32 * 11]
    ldp q24, q25, [sp, #32 * 12]
    ldp q26, q27, [sp, #32 * 13]
    ldp q28, q29, [sp, #32 * 14]
    ldp q30, q31, [sp, #32 * 15]
    add sp, sp, #FP_FRAME_SIZE

    RESTORE_CONTEXT
    eret
