// x86_64 exception entry stubs (vectors 0-31).
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

    mov rcx, rsp
    mov rbp, rsp
    and rsp, -16
    sub rsp, 32
    call x86_exception_handler
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

.section .rdata,"dr"
.balign 8
.global isr_stub_table
isr_stub_table:
.irp v,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad isr_stub_\v
.endr
