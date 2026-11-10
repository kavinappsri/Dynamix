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

/*
 * IRQ: the kernel runs in SVC mode, so rather than set up a stack for IRQ
 * mode, the return state is pushed onto the SVC stack (srs), the core is
 * switched to SVC (still masked), and the handler runs there. rfe returns
 * and restores the interrupted CPSR in one go.
 * The kernel is compiled soft-float, so no VFP/NEON state needs saving.
 */
irq_handler:
    sub lr, lr, #4              /* address of the interrupted instruction */
    srsdb sp!, #0x13            /* push {lr_irq, spsr_irq} on the SVC stack */
    cps #0x13
    push {r0-r4, r12, lr}       /* caller-saved + r4 (scratch below) + lr_svc */
    mov r4, sp
    bic sp, sp, #7              /* AAPCS: 8-byte aligned at calls */
    bl armv7_irq_entry
    mov sp, r4
    pop {r0-r4, r12, lr}
    rfeia sp!

fiq_handler:            EXCEPTION_ENTRY 7