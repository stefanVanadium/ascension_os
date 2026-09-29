/*
 * Ascension: kernel link script (Phase 0)
 *
 * The kernel is linked to run in the higher half at 0xFFFFFFFF80000000 + 1 MiB,
 * while GRUB loads it at physical 0x100000. AT(ADDR(...) - KERNEL_PHYS_OFF)
 * gives each section its physical load address (LMA) distinct from its virtual
 * address (VMA).
 */

ENTRY(_start)

KERNEL_VIRT_BASE = 0xFFFFFFFF80000000;

SECTIONS {
    . = 0xFFFFFFFF80100000;                 /* higher half, 1 MiB physical */

    .text : AT(ADDR(.text) - KERNEL_VIRT_BASE) {
        *(.multiboot2)                      /* header must be in first 32 KiB */
        *(.text .text.*)
    }
    kernel_text_end = .;

    .rodata : AT(ADDR(.rodata) - KERNEL_VIRT_BASE) {
        *(.rodata .rodata.*)
    }
    kernel_rodata_end = .;

    .data : AT(ADDR(.data) - KERNEL_VIRT_BASE) {
        *(.data .data.*)
    }
    kernel_data_end = .;                    /* .bss starts here */

    .bss : AT(ADDR(.bss) - KERNEL_VIRT_BASE) {
        *(COMMON)
        *(.bss .bss.*)
        . = ALIGN(4096);
    }

    kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note*)
        *(.comment*)
    }
}
