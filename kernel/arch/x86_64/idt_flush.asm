; Ascension: IDT register load. lidt from the descriptor at [rdi].

section .text
global idt_flush
idt_flush:
    lidt [rdi]
    ret

section .note.GNU-stack noalloc noexec nowrite progbits
