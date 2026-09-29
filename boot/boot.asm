; Ascension: boot entry (Phase 0)
; Multiboot2 header + protected-mode entry + long mode transition +
; higher-half jump. This file is on the irreducible-ASM list: it runs
; before any language runtime exists.
;
; Boot flow:
;   GRUB2 (BIOS) -> 32-bit protected mode, eax = MB2 boot magic, ebx = info ptr
;   _start: verify magic -> static page tables -> PAE -> LME -> PG ->
;           far jump to 64-bit (physical target) -> reload GDT virtually ->
;           serial markers -> call kmain(mb_magic, mb_info)
;
; Handover ABI (SysV): rdi = MB2 boot magic, rsi = MB2 info pointer (phys).
; The magic is stashed into edi right after verification because every page
; table load below spends eax; the info pointer needs no stash, nothing
; between _start and the call writes ebx.
;
; NOTE: code below 64-bit sections must only use 32-bit registers/instructions;
; the serial helpers are split into a 32-bit and a 64-bit variant for that
; reason (no cross-mode calls).

%define MB2_MAGIC        0xE85250D6
%define MB2_ARCH_I386    0
%define BOOT_MAGIC       0x36D76289

%define KERNEL_VIRT_BASE 0xFFFFFFFF80000000   ; higher-half offset

%define PAGE_PRESENT     0x001
%define PAGE_WRITE       0x002
%define PAGE_HUGE        0x080                ; PS bit: 2 MiB page in PD
%define PML4_IDX_HIGH    511                  ; 0xFFFFFFFF80000000 >> 39
%define PDPT_IDX_HIGH    510
%define PML4_IDX_WINDOW  256                  ; 0xFFFF800000000000 >> 39
%define PDPT_IDX_WINDOW  0

%define CR4_PAE          (1 << 5)
%define CR0_PG           (1 << 31)
%define MSR_EFER         0xC0000080
%define EFER_LME         (1 << 8)

%define GDT_CODE_SEL     0x08
%define GDT_DATA_SEL     0x10

%define BOOT_STACK_SIZE  16384                ; 16 KiB, per PLAN contract

%define COM1             0x3F8                ; serial port (named-constant rule)

section .multiboot2 align=8
header_start:
    dd MB2_MAGIC                              ; magic
    dd MB2_ARCH_I386                          ; architecture: i386 protected mode
    dd header_end - header_start              ; header length
    dd -(MB2_MAGIC + MB2_ARCH_I386 + (header_end - header_start)) ; checksum
    ; end tag
    dw 0                                      ; type = none
    dw 0                                      ; flags
    dd 8                                      ; size
header_end:

section .rodata
align 16
gdt_start:
    dq 0                                      ; null descriptor
    dq 0x00209A0000000000                     ; code: L=1, present, ring 0
    dq 0x0000920000000000                     ; data: present, ring 0
gdt_end:

gdt_descriptor_low:                           ; used while identity-mapped
    dw gdt_end - gdt_start - 1
    dq gdt_start - KERNEL_VIRT_BASE

gdt_descriptor_high:                          ; used after the higher-half jump
    dw gdt_end - gdt_start - 1
    dq gdt_start

; progress markers on COM1: the only output that works before Asc runs
boot_msg_magic:  db "BOOT: multiboot2 magic ok", 13, 10, 0
boot_msg_paging: db "BOOT: page tables set", 13, 10, 0
boot_msg_long:   db "BOOT: long mode entered", 13, 10, 0
boot_msg_high:   db "BOOT: higher half reached", 13, 10, 0

section .text
global _start
extern kmain

; =========================== 32-BIT REALITY ================================
bits 32
_start:
    cli

    ; --- verify Multiboot2 boot magic --------------------------------------
    cmp eax, BOOT_MAGIC
    jne .halt
    mov edi, eax                              ; stash magic in rdi (kmain arg0):
                                              ; eax is spent on page tables right
                                              ; below; 32-bit writes zero-extend

    ; --- temporary stack (identity-mapped physical address for now) ---------
    mov esp, boot_stack_top - KERNEL_VIRT_BASE

    ; --- early serial evidence ---------------------------------------------
    call serial_init32
    mov esi, boot_msg_magic - KERNEL_VIRT_BASE
    call serial_puts32

    ; --- fill static page tables (GRUB zero-filled .bss per ELF) ------------
    ; Table addresses are link-time relocations, so flag bits get OR-ed in at
    ; RUNTIME (all tables are 4 KiB-aligned, low 12 bits are free for flags).
    ; Identity map: first 4 MiB at virtual 0.
    mov eax, pdpt_table_low - KERNEL_VIRT_BASE
    or  al, PAGE_PRESENT | PAGE_WRITE
    mov dword [(pml4_table     - KERNEL_VIRT_BASE) + (0 * 8)], eax

    mov eax, pd_table_low - KERNEL_VIRT_BASE
    or  al, PAGE_PRESENT | PAGE_WRITE
    mov dword [(pdpt_table_low - KERNEL_VIRT_BASE) + (0 * 8)], eax

    ; higher-half map: 0xFFFFFFFF80000000 -> phys 0
    mov eax, pdpt_table_high - KERNEL_VIRT_BASE
    or  al, PAGE_PRESENT | PAGE_WRITE
    mov dword [(pml4_table      - KERNEL_VIRT_BASE) + (PML4_IDX_HIGH * 8)], eax

    mov eax, pd_table_high - KERNEL_VIRT_BASE
    or  al, PAGE_PRESENT | PAGE_WRITE
    mov dword [(pdpt_table_high - KERNEL_VIRT_BASE) + (PDPT_IDX_HIGH * 8)], eax
    ; PD entries: two 2 MiB huge pages covering phys 0x000000..0x400000
    mov dword [(pd_table_low  - KERNEL_VIRT_BASE) + (0 * 8)], 0x00000000 | PAGE_HUGE | PAGE_PRESENT | PAGE_WRITE
    mov dword [(pd_table_low  - KERNEL_VIRT_BASE) + (1 * 8)], 0x00200000 | PAGE_HUGE | PAGE_PRESENT | PAGE_WRITE
    mov dword [(pd_table_high - KERNEL_VIRT_BASE) + (0 * 8)], 0x00000000 | PAGE_HUGE | PAGE_PRESENT | PAGE_WRITE
    mov dword [(pd_table_high - KERNEL_VIRT_BASE) + (1 * 8)], 0x00200000 | PAGE_HUGE | PAGE_PRESENT | PAGE_WRITE

    ; Physical window (0xFFFF800000000000 -> phys 0, same 4 MiB): MM builds the
    ; real page tables through phys_to_virt() BEFORE the cr3 switch, so the
    ; window has to exist while boot.asm's tables still hold CR3. Dropped by
    ; vmm.asc when it installs its own tables.
    mov eax, pdpt_table_window - KERNEL_VIRT_BASE
    or  al, PAGE_PRESENT | PAGE_WRITE
    mov dword [(pml4_table      - KERNEL_VIRT_BASE) + (PML4_IDX_WINDOW * 8)], eax

    mov eax, pd_table_window - KERNEL_VIRT_BASE
    or  al, PAGE_PRESENT | PAGE_WRITE
    mov dword [(pdpt_table_window - KERNEL_VIRT_BASE) + (PDPT_IDX_WINDOW * 8)], eax

    mov dword [(pd_table_window - KERNEL_VIRT_BASE) + (0 * 8)], 0x00000000 | PAGE_HUGE | PAGE_PRESENT | PAGE_WRITE
    mov dword [(pd_table_window - KERNEL_VIRT_BASE) + (1 * 8)], 0x00200000 | PAGE_HUGE | PAGE_PRESENT | PAGE_WRITE

    mov esi, boot_msg_paging - KERNEL_VIRT_BASE
    call serial_puts32

    ; --- enter long mode ------------------------------------------------------
    mov eax, cr4
    or  eax, CR4_PAE                          ; PAE on
    mov cr4, eax

    mov eax, pml4_table - KERNEL_VIRT_BASE    ; PML4 (identity-mapped phys addr)
    mov cr3, eax

    mov ecx, MSR_EFER                         ; IA32_EFER
    rdmsr
    or  eax, EFER_LME                         ; long mode enable
    wrmsr

    mov eax, cr0
    or  eax, CR0_PG                           ; paging on -> long mode active
    mov cr0, eax

    lgdt [gdt_descriptor_low - KERNEL_VIRT_BASE]

    mov esi, boot_msg_long - KERNEL_VIRT_BASE
    call serial_puts32

    ; far jump into the 64-bit code segment. Target must be the PHYSICAL
    ; address of the entry label (paging is on, identity map holds): a
    ; ptr16:32 far jump can only carry a 32-bit offset, so the linked virtual
    ; form would not fit.
    jmp GDT_CODE_SEL:long_mode_entry_phys - KERNEL_VIRT_BASE

.halt:
    hlt
    jmp .halt

long_mode_entry_phys:

; ============================ 64-BIT REALITY ===============================
bits 64
long_mode_entry:
    mov ax, GDT_DATA_SEL
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    ; reload GDT from its VIRTUAL descriptor. CS keeps its hidden descriptor
    ; from the far jump above (same GDT layout), so no second far jump is
    ; needed, and a ptr16:32 jump could not reach higher-half anyway.
    lgdt [rel gdt_descriptor_high]

    ; jump to the linked (higher-half virtual) address
    mov rax, higher_half
    jmp rax

higher_half:
    ; running at linked virtual addresses from here on
    mov rsp, boot_stack_top                   ; virtual symbol
    xor ebp, ebp                              ; terminate frame chain

    mov rsi, boot_msg_high
    call serial_puts64

    ; hand off to Asc: kmain(mb_magic, mb_info), SysV args rdi/rsi.
    ; rdi has carried the magic since _start, ebx the physical info
    ; pointer since GRUB (callee-saved, untouched by everything above).
    ; The identity map keeps that pointer valid until MM owns paging.
    mov esi, ebx                              ; zero-extends into rsi

    call kmain

    ; kmain is `-> never` and halts internally; if it ever returns anyway:
    cli
    hlt
    jmp higher_half

; ---------------------------------------------------------------------------
; Minimal 16550 UART output: BOOT-domain debug evidence only.
; Port I/O is address-independent, so the init/putc helpers work identically
; in both modes; only the string walkers differ (stack width, pointer size).
; ---------------------------------------------------------------------------

; mode-neutral: init COM1 115200 8N1, FIFO on. Clobbers eax, edx.
serial_init32:
serial_init64:
    mov dx, COM1 + 1
    xor al, al                                ; disable UART interrupts
    out dx, al
    mov dx, COM1 + 3
    mov al, 0x80                              ; DLAB on
    out dx, al
    mov dx, COM1
    mov al, 0x01                              ; divisor low: 1 -> 115200 baud
    out dx, al
    mov dx, COM1 + 1
    xor al, al                                ; divisor high
    out dx, al
    mov dx, COM1 + 3
    mov al, 0x03                              ; 8N1, DLAB off
    out dx, al
    mov dx, COM1 + 2
    mov al, 0xC7                              ; FIFO enable + clear
    out dx, al
    mov dx, COM1 + 4
    mov al, 0x0B                              ; DTR | RTS | OUT2
    out dx, al
    ret

; mode-neutral: write AL to COM1, polling LSR bit 5. Clobbers eax, edx.
serial_putc32:
serial_putc64:
    mov ah, al
.wait:
    mov dx, COM1 + 5
    in  al, dx
    test al, 0x20                             ; transmit holding register empty
    jz .wait
    mov al, ah
    mov dx, COM1
    out dx, al
    ret

; 32-bit variant: ESI -> NUL-terminated string (identity-mapped phys addr)
bits 32
serial_puts32:
    push esi
    push eax
.loop:
    lodsb
    test al, al
    jz .done
    call serial_putc32
    jmp .loop
.done:
    pop eax
    pop esi
    ret

; 64-bit variant: RSI -> NUL-terminated string (higher-half virtual addr)
bits 64
serial_puts64:
    push rsi
    push rax
.loop:
    lodsb
    test al, al
    jz .done
    call serial_putc64
    jmp .loop
.done:
    pop rax
    pop rsi
    ret

section .bss
align 4096
pml4_table:       resb 4096
pdpt_table_low:   resb 4096
pd_table_low:     resb 4096
pdpt_table_high:  resb 4096
pd_table_high:    resb 4096
pdpt_table_window: resb 4096
pd_table_window:  resb 4096

align 16
boot_stack_bottom:
    resb BOOT_STACK_SIZE
boot_stack_top:

section .note.GNU-stack noalloc noexec nowrite progbits
