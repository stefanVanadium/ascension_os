# Ascension — Glossary

First terms, defined as we use them in this project. Kernel-edge vocabulary grows
with every phase; entries link to the doc that owns the concept.

| Term | Meaning |
|---|---|
| **Asc** | Our systems language (`.asc` files, `ascc` compiler). From *Ascension*. |
| **ascc** | The Asc compiler: Rust host tool + LLVM backend. Dev-machine only, never ships inside the OS. |
| **distinct type** | Nominal type wrapping a primitive (`type Paddr = distinct u64`). No implicit conversions in either direction; construction/wrapping is explicit (`Paddr(x)` / `u64(p)`). Enforced by ascc as hard errors. |
| **freestanding** | Compiled without hosted-OS assumptions: no runtime, no libc, no implicit heap. The only mode ascc has. |
| **red zone** | 128 bytes below RSP that leaf functions may use per SysV ABI. Forbidden in kernel code (interrupts stomp it); disabled via the `noredzone` LLVM attribute on every function. |
| **higher half** | Kernel linked at virtual `0xFFFFFFFF80100000` while loaded at physical `0x100000`; boot maps both and jumps to the virtual address before calling kmain. |
| **Multiboot2** | Boot protocol GRUB2 uses to load us. Header magic `0xE85250D6`, boot magic in EAX `0x36D76289`, info pointer in EBX. |
| **long mode** | x86_64's 64-bit mode. Entry sequence: PAE → page tables → `EFER.LME` → `CR0.PG` → far jump into a 64-bit code segment. |
| **LMA / VMA** | Load Memory Address (where GRUB puts bytes) vs Virtual Memory Address (where linked code expects to run). `linker.ld` ties them with `AT(...)`. |
| **kernel-mode unit** | An `.asc` file compiled with `--kernel`: no red zone, no FPU/SSE target features, size-first codegen level, kernel code model. |
| **erasure** | Compile-time guarantees (types, and later capabilities/channels) vanish before LLVM IR: zero runtime cost unless a runtime table is genuinely required and measured. |
| **extern declaration** | Bodyless `fn f(...) -> T;`, an external symbol reference resolved by the linker. Signatures are not checked across units (same trust model as C headers). |
| **array decay** | A `[N]T` value used where a pointer is expected becomes a pointer to element 0. Codegen emits the address, not an aggregate load. |
| **spinlock slot** | libk's lock representation: one aligned u32, 0 = free, 1 = held. Acquire is `xchg` test-and-set with a `pause` spin; IRQ variant saves RFLAGS via pushfq, restores via popfq. |
| **-Os profile** | Kernel-mode size-first optimization: `optsize` attribute on every function + `default<O2>` pass pipeline, vectorization off, MergeFunctions on. |
| **selftest ISO** | `ascension-selftest.iso`: identical kernel except kmain deliberately fires KASSERT(false). Proves the PANIC path end to end (serial line plus clean halt). |
