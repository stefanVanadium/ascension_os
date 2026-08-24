; Ascension: GDT reload.
; lgdt from the descriptor at [rdi], then CS reloads via a far RETURN, not
; a far jump: a ptr16:32 jump cannot carry a higher-half target, while the
; return form pushes the full 64-bit label. Data segments reload with movs;
; outside CS that is all long mode requires.

%define SEL_KCODE 0x08
%define SEL_KDATA 0x10

section .text
global gdt_flush
gdt_flush:
    lgdt [rdi]
    push qword SEL_KCODE
    lea rax, [rel .cont]
    push rax
    retfq
.cont:
    mov ax, SEL_KDATA
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    ret

section .note.GNU-stack noalloc noexec nowrite progbits
