# Ascension — Architecture (living document)

> Status after Phase 0. This diagram grows as waves land; superseded states move
> to `docs/journal/`, not deleted.

## What exists today

```
┌──────────────────────────────────────────────────────────┐
│  DEV MACHINE (host tools — never part of the OS)         │
│                                                          │
│   ascc  (tools/ascc, Rust + LLVM)                        │
│   .asc ──lex──parse──typeck──codegen──► x86_64 ELF .o    │
│          ▲                                               │
│          │ spec: docs/LANGUAGE.md                        │
└──────────┼───────────────────────────────────────────────┘
           │ compiles
┌──────────▼───────────────────────────────────────────────┐
│  KERNEL (Asc)                                            │
│  kernel/core/kernel.asc                                  │
│    kmain() -> never                                      │
│      polling UART driver (inline asm outb/inb)           │
│      halt_forever()                                      │
├──────────────────────────────────────────────────────────┤
│  BOOTSTRAP (NASM — irreducible list)                     │
│  boot/boot.asm                                           │
│    Multiboot2 header · magic check · static page tables  │
│    PAE → LME → PG → far jump → higher-half → kmain       │
├──────────────────────────────────────────────────────────┤
│  HARDWARE: QEMU x86_64 (BIOS + GRUB2, Multiboot2)        │
└──────────────────────────────────────────────────────────┘
```

## Memory layout (Phase 0)

| Region | Value |
|---|---|
| Kernel load address (phys) | `0x100000` |
| Kernel link/run address (virt) | `0xFFFFFFFF80100000` |
| Identity map | phys `0x0–0x400000` @ virt `0x0` |
| Higher-half map | phys `0x0–0x400000` @ virt `0xFFFFFFFF80000000` |
| Page tables | static, in `.bss`, hand-filled by boot.asm (2 MiB huge pages) |
| Boot stack | 16 KiB, `.bss`, 16-byte aligned |
| `kernel_end` symbol | exported for the future PMM |

## Build pipeline

```
kernel/core/kernel.asc ──ascc --kernel──► build/kernel.o
boot/boot.asm            ──nasm -f elf64─► build/boot.o
                                              │
              linker.ld ──ld -T───────────────┴──► build/ascension.elf
                                                       │
                              grub-mkrescue ◄── grub.cfg
                                    │
                                    ▼
                             ascension.iso  ──QEMU──► serial log
```

## Not yet built (in wave order)

LIBK (types/panic/spinlock) → ARCH (real GDT/IDT/TSS/PIC, context switch) →
MM (PMM/VMM/heap; replaces the static bootstrap paging) → DRV (VGA/keyboard/PCI as
userspace processes) → PROC (scheduler, IPC, capabilities, syscalls) → FS (objfs)
→ SH+BLD polish → continuous QA/CR/DOCS.
