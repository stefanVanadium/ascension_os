# Phase 1 — LIBK: types, panic, spinlock (and the language they needed)

> Sprint: Faza 1 (LANG+BLD → LIBK → QA → CR → DOCS) · Completed: 2026-08-24
> Deliverable: `kernel/libk/` is real. Distinct address types with conversions,
> `kpanic`/`kassert` printing `PANIC file:0xNN: msg` on COM1, interrupt-safe spinlocks,
> all exercised from kmain and verified by two bootable ISOs (normal + selftest).

## What shipped

```
tools/ascc/  six language features + the size-first profile
  extern fn        bodyless `fn f(...) -> T;` declarations. Multi-unit linking
                   via linker symbol resolution (no mangling, no keyword).
  [N]T arrays      value arrays, unchecked indexing, implicit decay to *T,
                   explicit decay cast (`buf as *u8`)
  &expr            address-of on lvalues, gives *T
  p.field          Go-style auto-deref through pointer-to-struct. This also
                   FIXED direct struct-local field access, broken in v0:
                   gen_field_ptr tried to "load" a struct value as a pointer.
  __FILE__ __LINE__ expanded at lex time from the token span
  -Os profile      --kernel optimizes for size now: optsize attribute on every
                   function plus a default<O2> pipeline with loop/SLP
                   vectorization off and MergeFunctions on
kernel/libk/
  types.asc        Paddr/Vaddr + phys_to_virt/virt_to_phys (contract for MM)
  panic.asc        kpanic -> never, kassert, hex line formatting, own UART plumbing
  spinlock.asc     u32-slot locks: init/acquire/release plus IRQ variants
                   (pushfq/cli before acquire, popfq restore after release);
                   xchg test-and-set with a pause spin loop
  mem.asc          memset/memcpy/memmove/memcmp written in Asc. LLVM lowers
                   optimized loops to these symbols, so a freestanding kernel
                   must export them or fail to link.
kernel/core/
  kernel.asc       kmain exercises the whole libk contract, serial markers between steps
  kernel_selftest.asc  same flow, then deliberately fires KASSERT(false)
tools/ascc/tests/  15 cases (5 positive incl. a cross-unit link check, 10 negative)
```

## Design decisions

Cross-unit nominal types work without modules by textual redeclaration: each unit
writes `type Paddr = distinct u64;` itself. Within one compilation nominality is
enforced with hard errors; across units both sides erase to identical LLVM types so
the linked program agrees. C headers minus header files, basically. Honest scaffolding
until modules land.

The lock is a u32 slot instead of a struct, for the same reason: callers cannot name
libk's struct type yet. So `spinlock_acquire(l: *u32)` takes the address of
caller-owned storage. API shape survives; the wrapper struct returns together with
modules.

mem.asc exists because of the optimizer. With -Os on, LLVM turns byte-fill loops into
`call memset`. Freestanding means nobody else provides it. Our implementations use
volatile accesses specifically so LoopIdiomRecognize cannot convert these very loops
back into recursive self-calls.

## Bugs found and fixed (all real, all by verification)

1. Malformed clobber constraints: `~memory` instead of `~{memory}`, latent since
   Faza 0 because nobody used clobbers. Symptom was a silent segfault inside libLLVM
   at emission time. Found by bisecting minimal asm cases, then confirmed directly
   with llc-19: `failed to parse constraints`.
2. `$N` substitution unreliable on this LLVM build inside memory-reference positions:
   `xchg $0, ($1)` dies with `invalid operand`. Same family as Faza 0's constraint
   bug. Workaround now standard everywhere: literal registers in templates (%eax)
   paired with explicit pinning constraints ({eax}). Literal AT&T immediates need
   `$$0`, since bare `$` starts an operand reference.
3. String globals collided across units: `asc.str.N` with external linkage produced
   duplicate symbols at link. Fixed with private linkage; string literals are
   module-local.
4. Volatile flag lost on indexed loads: the volatile decision checked the pointee
   type instead of the base pointer's volatility. Latent v0 bug, fixed alongside.

## Verification evidence

Normal boot, serial log in exactly this order:
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
QA matrix: clean build PASS. Compiler tests 15/15. Boots at `-m 2M`. Zero SSE/FPU
instructions in objdump. noredzone on every function. Cross-unit link resolves.
Footprint went from text 835 B to 1721 B (+886 B for four modules including panic
formatting); bss unchanged at 36 KiB.

## What's next (Faza 2)

BOOT+ARCH wave: real GDT/TSS, IDT with ISR trampolines, PIC. Once interrupts fire,
spinlock_irq stops being "correct by construction" and becomes correct under fire.
Then MM: the phys window activates and types.asc stops being a contract and becomes
load-bearing.
