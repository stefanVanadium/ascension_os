; Ascension: 256 interrupt trampolines + common frame builder.
; Irreducible ASM (CLAUDE.md list): exact iretq-frame stack discipline a
; young compiler cannot emit. Every stub normalizes to ONE frame shape:
; vectors whose CPU pushes a real error code (8,10,11,12,13,14,17,21) keep
; it; all others push a dummy qword, so vector + error-code slots always
; exist at the same offsets. isr_common saves the 15 GP registers in fixed
; order, hands RSP to the Asc dispatcher, restores, drops the two stub
; slots, iretq.
;
; Frame handed up (ascending addresses), mirrored by ISRFrame in isr.asc:
;   r15 r14 r13 r12 r11 r10 r9 r8 rbp rdi rsi rdx rcx rbx rax
;   vector errcode rip cs rflags rsp ss
;
; The table holds qword POINTERS because stubs differ in length; ascc can
; only consume functions across units, hence isr_stub_table_ptr.

section .text

extern isr_dispatch_asc

%macro NOERR_STUB 1
isr%1:
    push qword 0                    ; dummy error-code slot
    push qword %1                   ; vector number
    jmp isr_common
%endmacro

%macro ERRCODE_STUB 1
isr%1:
    push qword %1                   ; vector only: CPU already pushed the code
    jmp isr_common
%endmacro

%assign vec 0
%rep 32
    %if vec = 8 || vec = 10 || vec = 11 || vec = 12 || vec = 13 || vec = 14 || vec = 17 || vec = 21
        ERRCODE_STUB vec
    %else
        NOERR_STUB vec
    %endif
%assign vec vec+1
%endrep
%rep 224
    NOERR_STUB vec
%assign vec vec+1
%endrep

isr_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    mov rdi, rsp
    call isr_dispatch_asc

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16                     ; vector + error-code slots
    iretq

section .rodata
align 8
global isr_stub_table
isr_stub_table:
%assign i 0
%rep 256
    dq isr %+ i
%assign i i+1
%endrep

section .text
global isr_stub_table_ptr
isr_stub_table_ptr:
    lea rax, [rel isr_stub_table]
    ret

section .note.GNU-stack noalloc noexec nowrite progbits
