# Phase 0: the Asc compiler and the first boot

> Sprint: Faza 0 (LANG → BOOT → BLD → integration → QA) · Completed: 2026-08-24
> Deliverable: a bootable ISO whose kernel entry is written in **Asc**, compiled by
> **our own `ascc`** (Rust host, LLVM backend), printing to serial from inside Asc code.

## What exists now

```
tools/ascc/            ~2,900 lines of Rust
  src/lexer.rs         tokens + spans, ASCII-only, no recovery
  src/parser.rs        recursive descent + Pratt expressions
  src/typeck.rs        distinct types, literal narrowing, cast matrix, const folding
  src/codegen.rs       AST -> LLVM IR (inkwell 0.10 / llvm-sys 191), freestanding config
boot/boot.asm          Multiboot2 header, long-mode transition, higher-half jump
kernel/core/kernel.asc kmain() in Asc: polling UART driver + halt loop
linker.ld              VMA 0xFFFFFFFF80100000 / LMA 0x100000
Makefile               ascc -> kernel.o + boot.o -> ELF -> grub-mkrescue ISO -> QEMU
```

## Why Rust + LLVM (and not something else)

The compiler is a dev-machine tool, never part of the OS. Hand-writing an x86_64
instruction selector or register allocator is a multi-year detour; LLVM already
solves it. Rust gives us sum types and exhaustive matching for the AST work.
Everything Asc-specific (distinct types, capability/channel rules once they exist)
lives in `typeck.rs` and is **erased before codegen**, exactly like Rust erases the
borrow checker before LLVM sees IR.

## The pipeline, on a real example

```asc
fn inb(port: u16) -> u8 {
    let ret: u8;
    asm { "inb %1, %0" : "=a"(ret) : "Nd"(port) }
    return ret;
}
```

1. lex: keywords, spans (`file:line:col` carried everywhere)
2. parse: `Stmt::Asm` with template string + constraint operands
3. typeck: output binding `ret` must be a declared local; inputs type-checked;
   every expression's resolved Ty recorded into an address-keyed map for codegen
4. codegen: GCC template `%0` translated to LLVM `$0`; register-class
   constraints expanded to explicit registers sized by operand width
   (`"a"` on u16 → `{ax}`); `LLVMGetInlineAsm` + `LLVMBuildCall2` build the call;
   result stored into the local's alloca

Emitted machine code for this function:

```asm
inb:
    mov  %edi,%edx
    in   (%dx),%al          ; the entire function body
    ret
```

## Kernel-mode guarantees enforced by ascc

| Guarantee | Mechanism | Verified |
|---|---|---|
| no red zone | `noredzone` enum attribute on every fn | `attributes #0 = { noredzone }` in IR |
| no FPU/SSE | target features `-mmx,-sse,...,-x87`; language has NO float literals at all | `objdump -d \| grep xmm` → 0 hits |
| higher-half linking | kernel code model + static reloc + `linker.ld` | ELF entry `0xffffffff80100020`, PhysAddr `0x100000` |
| distinct types | typeck rejects arithmetic/casts across distinct types | negative tests all hard-error |
| no ambient anything | nothing global is reachable without declaration (trivially true in v0; capabilities come later) | n/a |

Known limitation, tracked: inkwell exposes codegen opt levels 0–3 but not
SizeLevel, so `--kernel` uses OptLevel 1 as the closest size-first profile.
A custom pass pipeline with SizeLevel=1 is a Phase 1 item.

## Debugging war stories (what actually bit)

1. inkwell needs newer Rust than Debian ships. Debian's rustc 1.85 rejected
   inkwell 0.10's let-chains → rustup stable (1.98), user-level install.
2. Debian ships no static libPolly, so link libLLVM dynamically
   (`llvm19-1-prefer-dynamic` feature).
3. `noredzone`, not `no-red-zone`. The hyphenated spelling silently resolves to
   attribute kind 0, which poisons the module and segfaults `verify()` at emission
   time. Found by isolating emission stages, then confirming via ctypes against
   `libLLVM-19.so` that only the unhyphenated name resolves.
4. Generic constraints broken in Debian's llc-19: `"a"` fails with "couldn't
   allocate input reg"; explicit `"{ax}"` works, since clang expands classes anyway, so
   ascc does the same, sized by operand width.
5. NASM can't OR constants into relocations: page-table entries get their flag
   bits OR-ed in at runtime instead of assembly time.
6. The final bug was mine, not the toolchain's: `kmain`'s UART poll read
   `0x3F8` (data port) instead of `0x3FD` (LSR). QEMU monitor `info registers`
   showed RIP parked in `inb` with RDI=0x3F8; one look, one-line fix.

## QA evidence

Clean-build serial log (QEMU, `-serial file:`, `-no-reboot`):

```
BOOT: multiboot2 magic ok
BOOT: page tables set
BOOT: long mode entered
BOOT: higher half reached
Ascension: hello from Asc
```

- banner appears exactly once, then clean halt (no reboot loop)
- boots down to `-m 2M` RAM
- footprint: text 835 B · bss 36 KiB (page tables + boot stack) · ELF 7,120 B · ISO ~21 MiB (GRUB-dominated)
- zero SSE/FPU instructions in the linked kernel

Negative type tests (all hard errors, none warnings):

```text
Paddr(4096) + 1        -> distinct type `Paddr` does not participate in arithmetic...
let x: u64 = Paddr(1)  -> type mismatch: expected `u64`, found `Paddr`
p as u64 (distinct p)  -> `as` cannot touch distinct type...
Vaddr(Paddr(1))        -> cannot construct `Vaddr` from `Paddr`
if f(1) (bool param)   -> type mismatch: expected `bool`, found `u64`
```

## What comes next (Phase 1: LIBK wave)

- types.asc (Paddr/Vaddr/Capability<T>/chan<T> skeletons), panic.asc, spinlock.asc, all in Asc
- ascc: ownership/move checking beyond the Copy-everything v0, Capability<T> +
  chan<T> checking, size-first pass pipeline (SizeLevel=1)
- then BOOT+ARCH wave: real GDT/TSS/IDT replacing the bootstrap GDT
