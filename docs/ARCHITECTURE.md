# Ascension: Architecture (living document)

> Status after Phase 2. This diagram grows as waves land; superseded states move
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
│    kmain(mb_magic, mb_info) -> never, arch bring-up in   │
│    contract order, sti LAST, idle loop hlt-waits on tick │
│  kernel/core/kernel_selftest.asc                         │
│    selftest ISO entry, fires ud2, dumps the frame        │
├──────────────────────────────────────────────────────────┤
│  CPU ARCH LAYER (LIVE, x86_64)                           │
│  kernel/arch/x86_64/ (Asc + irreducible NASM)            │
│    io.asc       outb/inb + LE store helpers              │
│    gdt/tss/idt  runtime-built tables, lgdt/ltr/lidt      │
│    isr.asc      ISRFrame + Asc dispatcher                │
│    pic.asc      8259 remapped to vectors 32..47          │
│    gdt_flush.asm / idt_flush.asm / isr_stubs.asm         │
│      (256 trampolines, one normalized frame shape)       │
│  kernel/drivers/pit.asc  scheduler tick, 100 Hz ch0 mode3│
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
| Interrupt stacks | `int_stack` 16 KiB (TSS.RSP0) + `df_stack` 4 KiB (IST1), both `.bss` in tss.asc |
| `kernel_end` symbol | exported for the future PMM |

## Build pipeline

```
kernel/core/kernel.asc ──ascc --kernel──► build/kernel.o
kernel/**/*.asc        ──ascc --kernel──► build/kernel/.../*.o
kernel/**/*.asm        ──nasm -f elf64──► build/kernel/.../*.o   (auto-discovered)
boot/boot.asm          ──nasm -f elf64──► build/boot.o
                                              │
              linker.ld ──ld -T───────────────┴──► build/ascension.elf
                                                       │
                              grub-mkrescue ◄── grub.cfg
                                    │
                                    ▼
                             ascension.iso  ──QEMU──► serial log
```

## Not yet built (in wave order)

ARCH landed in Faza 2: real GDT/TSS/IDT/PIC plus the PIT tick; only the context
switch is still pending (it goes with PROC). Remaining waves:

MM (PMM/VMM/heap; replaces the static bootstrap paging) → DRV (VGA/keyboard/PCI
as userspace processes) → PROC (scheduler, IPC, capabilities, syscalls, context
switch) → FS (objfs) → SH+BLD polish → continuous QA/CR/DOCS.
