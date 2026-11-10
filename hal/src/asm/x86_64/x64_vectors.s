// x86_64 exception and interrupt entry stubs (vectors 0-255).
//
// Every stub normalises the frame to
//   [vector][error code (real or 0)][rip][cs][rflags][rsp][ss]
// then jumps to isr_common, which saves the GPRs and calls the Rust handler
// `x86_exception_handler(&mut ExceptionContext)` (efiapi / Win64 convention:
// first argument in rcx, 32 bytes of shadow space).

.section .text.x86_isr,"xr"
.balign 16

.macro ISR_NOERR vec
.balign 16
isr_stub_\vec:
    push 0
    push \vec
    jmp isr_common
.endm

.macro ISR_ERR vec
.balign 16
isr_stub_\vec:
    push \vec
    jmp isr_common
.endm

.irp v,0,1,2,3,4,5,6,7,9,15,16,18,19,20,22,23,24,25,26,27,28,31
    ISR_NOERR \v
.endr
.irp v,8,10,11,12,13,14,17,21,29,30
    ISR_ERR \v
.endr

.balign 16
isr_common:
    cld
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    // The Rust handler may use SSE registers, and an interrupt can arrive
    // anywhere, so the interrupted code's FPU/SSE state is saved around it.
    mov rbp, rsp
    and rsp, -16
    sub rsp, 512
    fxsave [rsp]
    mov rcx, rbp
    sub rsp, 32
    call x86_exception_handler
    add rsp, 32
    fxrstor [rsp]
    mov rsp, rbp

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    add rsp, 16
    iretq

// Vectors 32-255 (external interrupts, IPIs). 224 identical 16-byte stubs:
// `push 0` fills the error-code slot and `call irq_common` pushes an address
// from which the vector is recovered, so no per-stub symbol or immediate is
// needed. The Rust side computes each gate address as irq_stub_first + 16 * n.
.balign 16
.global irq_stub_first
irq_stub_first:
.rept 224
    push 0
    call irq_common
    .balign 16
.endr

// The return address `call` pushed is stub_start + 7; turn it into the vector
// in place, leaving [vector][error code 0][rip][cs][rflags][rsp][ss].
irq_common:
    push rax
    push rcx
    mov rax, [rsp + 16]
    lea rcx, [rip + irq_stub_first]
    sub rax, rcx
    shr rax, 4
    add rax, 32
    mov [rsp + 16], rax
    pop rcx
    pop rax
    jmp isr_common

.section .rdata,"dr"
.balign 8
.global isr_stub_table
isr_stub_table:
.irp v,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad isr_stub_\v
.endr
