.section .text.vectors
.global vector_table

/*
 * ARM32 Vector Table (VBAR aligned to 32 bytes)
 * 0x00: Reset
 * 0x04: Undefined Instruction
 * 0x08: Software Interrupt (SVC)
 * 0x0C: Prefetch Abort
 * 0x10: Data Abort
 * 0x14: Reserved
 * 0x18: IRQ
 * 0x1C: FIQ
 */
.balign 32
vector_table:
    b reset_handler           /* 0x00 */
    b undef_handler           /* 0x04 */
    b svc_handler             /* 0x08 */
    b prefetch_abort_handler  /* 0x0C */
    b data_abort_handler      /* 0x10 */
    b reserved_handler        /* 0x14 */
    b irq_handler             /* 0x18 */
    b fiq_handler             /* 0x1C */

.macro EXCEPTION_ENTRY vector_id
    /* Allocate 80-byte stack frame (8-byte AAPCS aligned) */
    sub sp, sp, #80

    /* Save r0 through r12 at offsets 0..48 */
    stmia sp, {r0-r12}

    /* Save SP, LR, return PC, and SPSR */
    add r0, sp, #80
    str r0, [sp, #52]          /* Original SP */
    str lr, [sp, #56]          /* LR */
    str lr, [sp, #60]          /* Return PC */

    mrs r0, spsr
    str r0, [sp, #64]          /* SPSR */

    /* Read DFAR (Fault Address Register) and DFSR (Fault Status Register) */
    mrc p15, 0, r0, c6, c0, 0   /* DFAR */
    str r0, [sp, #68]
    mrc p15, 0, r0, c5, c0, 0   /* DFSR */
    str r0, [sp, #72]

    /* Call rust_exception_handler(ctx: &ExceptionContext, vector_id: usize) */
    mov r0, sp
    mov r1, #\vector_id
    bl rust_exception_handler
.endmacro

reset_handler:          EXCEPTION_ENTRY 0
undef_handler:          EXCEPTION_ENTRY 1
svc_handler:            EXCEPTION_ENTRY 2
prefetch_abort_handler: EXCEPTION_ENTRY 3
data_abort_handler:     EXCEPTION_ENTRY 4
reserved_handler:       EXCEPTION_ENTRY 5
irq_handler:            EXCEPTION_ENTRY 6
fiq_handler:            EXCEPTION_ENTRY 7