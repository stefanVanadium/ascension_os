# Ascension: Glossary

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
| **selftest ISO** | `ascension-selftest.iso`: identical kernel except kmain deliberately fires a fault. Proves the panic/exception path end to end (serial dump plus clean halt). |
| **IDT gate** | One 16-byte IDT entry: what vector N redirects to. Quad 0 packs offset_lo \| selector \| ist \| type_attr \| offset_mid, quad 1 is offset_hi + reserved. All 256 of ours are type `0x8E` (present, ring 0, 64-bit interrupt gate) pointing at kernel CS `0x08`. |
| **IST** | Interrupt Stack Table: seven optional stack pointers in the TSS. A gate naming ISTn switches to that stack unconditionally on entry, even when RSP itself is what faulted. We use IST1 for the double-fault gate (`df_stack`, 4 KiB). |
| **TSS / RSP0** | Task State Segment, mandatory in long mode for privilege transitions and interrupt stack switching. RSP0 is the ring-0 stack entered from ring 3; currently aims at `int_stack` (16 KiB). PROC will update it per task switch. |
| **EOI** | End Of Interrupt: `0x20` written to the PIC command port after handling an IRQ so lower-priority lines can come through. Deliberately skipped for the spurious master vector. |
| **spurious IRQ** | Fake interrupt the master raises on IRQ7 when a cascaded slave line drops before the cascade completes. Shows up as vector 39 after our remap. Acknowledging it desyncs the PIC's in-service register, so the dispatcher returns without EOI. |
| **iretq frame** | The five qwords the CPU pushes on interrupt entry: rip, cs, rflags, rsp, ss. Our full frame adds vector, error code and 15 saved GPRs below them; ISRFrame in isr.asc mirrors all 22 qwords. |
| **ICW / OCW** | 8259 PIC command words. ICW1..4 run once during init (cascade, vector base for the remap, slave wiring, 8086 mode); OCWs drive operation afterwards (mask bits, EOI, ISR/IRR reads). |
