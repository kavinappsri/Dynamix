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

/*
 * Fatal exception entry. Runs on the exception mode's own stack (set up by
 * armv7_init_fault_stacks below), so it works whatever state the kernel's SVC
 * stack is in.
 *
 *   vector_id   index handed to rust_exception_handler
 *   lr_adjust   bytes to subtract from lr to get the faulting instruction:
 *               4 for undefined/prefetch abort/SVC/FIQ, 8 for data aborts
 *   abort_kind  1 = data abort (read DFAR/DFSR), 2 = prefetch abort (IFAR/IFSR),
 *               0 = neither
 *
 * Frame (80 bytes), shared with `ExceptionContext` in armv7_exceptions.rs:
 *   r0-r12 @ 0..48, sp @ 52, lr @ 56, pc @ 60, spsr @ 64, far @ 68, fsr @ 72
 */
.macro EXCEPTION_ENTRY vector_id, lr_adjust, abort_kind
    sub lr, lr, #\lr_adjust    /* address of the faulting instruction */
    sub sp, sp, #80
    stmia sp, {r0-r12}
    str lr, [sp, #60]          /* pc */

    mrs r0, spsr
    str r0, [sp, #64]          /* spsr */

    /*
     * sp and lr of the mode that was interrupted live in that mode's banked
     * registers: step into it (masked, never user mode, which could not step
     * back), copy them out, and return to this mode.
     */
    mrs r4, cpsr
    bic r1, r0, #0x3f          /* drop T and the mode bits */
    and r2, r0, #0x1f
    cmp r2, #0x10
    moveq r2, #0x1f            /* user -> system: same registers, privileged */
    orr r1, r1, r2
    orr r1, r1, #0xC0
    msr cpsr_c, r1
    mov r2, sp
    mov r3, lr
    msr cpsr_c, r4
    str r2, [sp, #52]          /* sp */
    str r3, [sp, #56]          /* lr */

.if \abort_kind == 1
    mrc p15, 0, r0, c6, c0, 0  /* DFAR */
    str r0, [sp, #68]
    mrc p15, 0, r0, c5, c0, 0  /* DFSR */
    str r0, [sp, #72]
.elseif \abort_kind == 2
    mrc p15, 0, r0, c6, c0, 2  /* IFAR */
    str r0, [sp, #68]
    mrc p15, 0, r0, c5, c0, 1  /* IFSR */
    str r0, [sp, #72]
.else
    mov r0, #0
    str r0, [sp, #68]
    str r0, [sp, #72]
.endif

    /* rust_exception_handler(ctx: &ExceptionContext, vector_id: usize) -> ! */
    mov r0, sp
    mov r1, #\vector_id
    bl rust_exception_handler
1:  wfe
    b 1b
.endm

reset_handler:          EXCEPTION_ENTRY 0, 0, 0
undef_handler:          EXCEPTION_ENTRY 1, 4, 0
svc_handler:            EXCEPTION_ENTRY 2, 4, 0
prefetch_abort_handler: EXCEPTION_ENTRY 3, 4, 2
data_abort_handler:     EXCEPTION_ENTRY 4, 8, 1
reserved_handler:       EXCEPTION_ENTRY 5, 0, 0

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

fiq_handler:            EXCEPTION_ENTRY 7, 4, 0

/*
 * Gives the abort, undefined-instruction and FIQ modes a stack of their own.
 * Only the SVC stack is set up at boot; without these, the first fatal
 * exception would push its frame through an uninitialised banked sp and die
 * silently. Called once from armv7_exceptions::init, in a privileged mode,
 * with interrupts masked.
 */
.arm
.global armv7_init_fault_stacks
armv7_init_fault_stacks:
    mrs r2, cpsr
    cps #0x17                   /* abort */
    ldr sp, =abt_stack_top
    cps #0x1b                   /* undefined */
    ldr sp, =und_stack_top
    cps #0x11                   /* FIQ */
    ldr sp, =fiq_stack_top
    msr cpsr_c, r2              /* back to the caller's mode */
    bx lr
.ltorg

.section .bss.fault_stacks, "aw", %nobits
.balign 8
abt_stack:      .space 4096
abt_stack_top:
und_stack:      .space 4096
und_stack_top:
fiq_stack:      .space 1024
fiq_stack_top:
