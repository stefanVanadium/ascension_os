# Phase 1 — LIBK: types, panic, spinlock (and the language they needed)

> Sprint: Faza 1 (LANG+BLD → LIBK → QA → CR → DOCS) · Completed: 2026-08-24
> Deliverable: `kernel/libk/` is real — distinct address types with conversions,
> `kpanic`/`kassert` printing `PANIC file:0xNN: msg` on COM1, working interrupt-safe
> spinlocks — exercised from `kmain`, verified by two bootable ISOs (normal + selftest).

## What shipped

```
tools/ascc/  six language features + the size-first profile
  extern fn        bodyless `fn f(...) -> T;` declarations — multi-unit linking
                   via linker symbol resolution (no mangling, no keyword)
  [N]T arrays      value arrays, unchecked indexing, implicit decay to *T,
                   explicit decay cast (`buf as *u8`)
  &expr            address-of on lvalues → *T
  p.field          Go-style auto-deref field access through pointer-to-struct
                   (this also FIXED direct struct-local field access, which was
                   broken in v0: gen_field_ptr tried to "load" a struct as pointer)
  __FILE__ __LINE__ magic constants expanded at lex time from the token span
  -Os profile      --kernel now optimizes for size: optsize attribute on every
                   function + default<O2> pipeline, loop/SLP vectorization OFF,
                   MergeFunctions ON
kernel/libk/
  types.asc        Paddr/Vaddr + phys_to_virt/virt_to_phys (contract for MM)
  panic.asc        kpanic -> never, kassert, hex line formatting, own UART plumbing
  spinlock.asc     u32-slot locks: init/acquire/release + IRQ variants (pushfq/cli,
                   popfq restore); xchg test-and-set with pause spin loop
  mem.asc          memset/memcpy/memmove/memcmp in Asc — LLVM lowers optimized
                   loops to these symbols, so a freestanding kernel MUST export them
kernel/core/
  kernel.asc       kmain exercises the whole libk contract with serial markers
  kernel_selftest.asc  same flow, then deliberately fires KASSERT(false)
tools/ascc/tests/  15 cases (5 positive incl. cross-unit link check, 10 negative)
```

## Design decisions worth remembering

**Cross-unit nominal types without modules.** Each unit redeclares `type Paddr =
distinct u64;` textually. Within a compilation, nominality is enforced (hard errors);
across units, both sides erase to identical LLVM types, so the linked program agrees.
This is C headers minus header files. It is honest v1 scaffolding until modules land.

**The lock is a u32 slot, not a struct.** Same cross-unit problem: callers can't name
libk's struct type yet. So `spinlock_acquire(l: *u32)` takes the address of caller-owned
storage. The API shape survives; the wrapper struct returns with modules.

**mem.asc exists because of the optimizer.** With `-Os` on, LLVM turns byte-fill loops
into `call memset`. Freestanding = nobody else provides it. Our implementations use
volatile accesses specifically so LoopIdiomRecognize cannot convert them back into
recursive self-calls.

## Bugs found and fixed (all real, all by verification)

1. **Malformed clobber constraints**: `~memory` instead of `~{memory}` — latent since
   Faza 0 (nobody used clobbers). Symptom: silent segfault inside libLLVM at emission.
   Found by bisecting minimal asm cases, then confirming with llc-19 directly:
   `failed to parse constraints`.
2. **`$N` substitution unreliable on this LLVM build** in memory-reference positions
   (`xchg $0, ($1)` → `invalid operand`). Same family as Faza 0's constraint bug.
   Workaround used everywhere now: literal registers in templates (%eax) + explicit
   pinning constraints ({eax}). Literal AT&T immediates need `$$0`.
3. **String globals collided across units** (`asc.str.N` external linkage, duplicate
   symbol at link). Fixed with private linkage — string literals are module-local.
4. **Volatile flag lost on indexed loads**: the volatile decision checked the pointee
   type instead of the base pointer's volatility. Latent v0 bug; fixed alongside.

## Verification evidence

Normal boot (serial, exactly this order):
```
BOOT: multiboot2 magic ok / page tables set / long mode / higher half reached
LIBK: locks initialized / nested acquire ok / nested release ok / irq lock round trip ok
Ascension: hello from Asc        <- then clean halt
```
Selftest boot ends with:
```
SELFTEST: firing deliberate assert
PANIC kernel/core/kernel_selftest.asc:0x3a: selftest    <- clean halt after
```
QA matrix: clean build PASS · 15/15 compiler tests · boots at `-m 2M` · 0 SSE/FPU
instructions in objdump · noredzone on every function · cross-unit link resolves.
Footprint: text 835 B → 1721 B (+886 B for four modules incl. panic formatting),
bss unchanged 36 KiB, ELF ~7 KB class, ISO ~21 MB (GRUB-dominated).

## What's next (Faza 2)

BOOT+ARCH wave: real GDT/TSS, IDT + ISR trampolines, PIC — interrupts exist, which
turns spinlock_irq from "correct by construction" into "correct under fire". Then MM:
the phys window activates and types.asc stops being a contract and becomes load-bearing.
