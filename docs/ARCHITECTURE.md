# Ascension: Architecture (living document)

> Status after Phase 1. This diagram grows as waves land; superseded states move
> to `docs/journal/`, not deleted.

## What exists today

```
┌──────────────────────────────────────────────────────────┐
│  DEV MACHINE (host tools: never part of the OS)          │
│                                                          │
│   ascc  (tools/ascc, Rust + LLVM)                        │
│   .asc ──lex──parse──typeck──codegen(-Os)──► ELF .o      │
│          ▲                                               │
│          │ spec: docs/LANGUAGE.md · tests: tools/ascc/tests
└──────────┼───────────────────────────────────────────────┘
           │ compiles (multi-unit: linker resolves extern decls)
┌──────────▼───────────────────────────────────────────────┐
│  KERNEL (Asc)                                            │
│  kernel/libk/                                            │
│    types.asc    Paddr/Vaddr + conversions (contract→MM)  │
│    panic.asc    kpanic/kassert, descriptive halt, COM1   │
│    spinlock.asc u32-slot locks, xchg test-and-set + IRQ  │
│    mem.asc      memset/memcpy/memmove/memcmp (LLVM needs)│
│  kernel/core/kernel.asc                                  │
│    kmain() -> never, exercises the libk contract         │
│  kernel/core/kernel_selftest.asc                         │
│    selftest ISO entry, fires KASSERT(false) on purpose   │
├──────────────────────────────────────────────────────────┤
│  BOOTSTRAP (NASM, irreducible list)                      │
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
