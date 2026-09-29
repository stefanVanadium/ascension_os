# Ascension: Architecture (living document)

> Status after Phase 3. This diagram grows as waves land; superseded states move
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
│    types.asc    Paddr/Vaddr + conversions (LOAD-BEARING)  │
│    symbols.asc  link-time symbols via inline asm         │
│    panic.asc    kpanic/kassert, descriptive halt, COM1   │
│    spinlock.asc u32-slot locks, xchg test-and-set + IRQ  │
│    mem.asc      memset/memcpy/memmove/memcmp (LLVM needs)│
│  kernel/mm/                    (LIVE, owns the address   │
│                                 space after Phase 3)      │
│    mm.asc       address-space constants, frame math      │
│    pmm.asc      bitmap PMM fed by the multiboot2 map,     │
│                 first-fit lowest, KASSERT on misuse       │
│    vmm.asc      new PML4 built through the window, W^X    │
│                 per section, 2 MiB window chain, then     │
│                 cr3 load; map/unmap/get_phys afterwards   │
│    heap.asc     explicit arenas, bump allocator, pages   │
│                 mapped on demand. No global heap          │
│  kernel/core/boot_steps.asc    the MM bring-up tail,     │
│                 shared verbatim by both entry units       │
│  kernel/core/kernel.asc                                  │
│    kmain(mb_magic, mb_info) -> never, arch then MM       │
│    bring-up in contract order, sti LAST, idle loop        │
│    hlt-waits on tick                                       │
│  kernel/core/kernel_selftest.asc                         │
│    selftest ISO entry, same MM path, fires ud2, dumps the │
│    frame                                                  │
├──────────────────────────────────────────────────────────┤
│  CPU ARCH LAYER (LIVE, x86_64)                           │
│  kernel/arch/x86_64/ (Asc + irreducible NASM)            │
│    io.asc       outb/inb + LE store helpers              │
│    gdt/tss/idt  runtime-built tables, lgdt/ltr/lidt      │
│    isr.asc      ISRFrame + Asc dispatcher, cr2 in the    │
│                 page-fault dump                          │
│    msr.asc      rdmsr/wrmsr (EFER.NXE before any NX bit) │
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

## Memory layout (Phase 3)

Full detail in [MEMORY_MAP.md](MEMORY_MAP.md). Summary:

| Region | Value |
|---|---|
| Kernel load address (phys) | `0x100000` |
| Kernel link/run address (virt) | `0xFFFFFFFF80100000` |
| Physical window | PML4[256], `0xFFFF800000000000` + phys, 2 MiB huge pages, RW+NX, a PD chain not a single PD. This is what makes `phys_to_virt` real |
| Kernel image mapping | PML4[511] → PDPT_HIGH → 4 KiB pages, W^X per section |
| Identity map | GONE after `vmm_init` writes CR3. Nothing is mapped at virtual 0 |
| Guard gap | `0xFFFFFFFF80000000–0xFFFFFFFF800FFFFF` and PML4[511][511] unmapped on purpose, so a wild pointer faults |
| User space | `0x0–0x7FFFFFFFFFFF`, unmapped until PROC |
| Page tables | Phase 0–2: static, in `.bss`, hand-filled by boot.asm. Phase 3: built at runtime by `vmm_init` from `pmm_alloc` frames, all below 4 MiB while bootstrap paging still holds |
| Kernel arena | 4 MiB of address space at `0xFFFFFFFF80200000`, pages mapped on demand |
| Boot stack | 16 KiB, `.bss`, 16-byte aligned |
| Interrupt stacks | `int_stack` 16 KiB (TSS.RSP0) + `df_stack` 4 KiB (IST1), both `.bss` in tss.asc |
| `kernel_end` symbol | defined in linker.ld, read through inline asm in `symbols.asc` |

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

ARCH landed in Faza 2 (GDT/TSS/IDT/PIC plus the PIT tick, context switch still
pending, it goes with PROC). MM landed in Faza 3: bitmap PMM, runtime page
tables, live physical window, explicit arenas. Remaining waves:

PROC (scheduler, capabilities, syscalls, context switch, userspace address
spaces) → DRV (VGA/keyboard/PCI as userspace processes) → FS (objfs) → SH+BLD
polish → continuous QA/CR/DOCS.

The one thing every later wave inherits from MM: `pmm_alloc`, `vmm_map` and
`arena_alloc` are ambient authority today. `Capability<T>` does not exist in
ascc yet, so PROC is the wave that has to fix it or name the debt explicitly.
