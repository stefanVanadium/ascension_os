# ▲ Ascension

> The OS that starts where Unix stopped — built from zero, all the way down to its own programming language.

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
![Status: Pre-production](https://img.shields.io/badge/status-pre--production-orange)
![Architecture: x86_64](https://img.shields.io/badge/arch-x86__64-informational)

**Ascension** is a full coherent system stack, designed as a single organism: a capability-based
microkernel, synchronous typed-channel IPC, a Plan 9-style typed-object namespace — and **Asc**,
a systems language built alongside the kernel, because C and Rust both force you to fake
capabilities and typed channels as library conventions instead of real, compiler-checked primitives.

No C anywhere in kernel or userspace. No ambient authority. No bloat.

---

## The idea in one breath

| Principle | What it means |
|---|---|
| **Capabilities, not permissions** | No root, no rwx bits. A process can touch exactly what it holds an unforgeable token for — everything else doesn't even *exist* to it |
| **Typed channels, not shared memory + locks** | `chan<T>`, `send`, `recv` are language primitives; message-type mistakes die at compile time |
| **Radical microkernel** | Under ~15k lines of kernel: scheduling, memory, IPC, capabilities. Everything else — drivers included — is isolated, restartable userspace |
| **Zero ambient authority** | If it's not in the function signature (`Capability<T>`), it's not reachable |
| **Radical efficiency** | Every byte and every cycle must earn its place. Size regressions fail QA like functional ones |

## Asc — the language that makes the OS possible

`Capability<T>`, `chan<T>`, ownership and explicit memory regions aren't libraries bolted onto a
generic language — they're primitives of Asc's type system:

```asc
fn read_sensor(cap: Capability<SensorRead>) -> Data {
    // unreachable without the capability — the compiler enforces it
}

let ch: chan<Message>
send(ch, msg)          // only Message can cross this channel. Ever.
```

- **Capabilities as types** — compile-time proofs, erased to (near-)zero runtime cost
- **Ownership + regions, no GC** — predictable latency, no garbage-collection pauses in the kernel
- **Bootstrap:** `ascc` v0 is a hosted tool written in Rust with an LLVM backend. Rust never ships
  with the OS — it's the dev machine's cross-compiler, one we happen to have written ourselves.
  Self-hosting is a long-term goal.

## Kernel architecture

```
┌─────────────────────────────────────────┐
│                USER SPACE                │
│ embers shell · apps · MOST drivers (vga, │  ← Asc, capability-scoped,
│  keyboard, pci, net, storage, fs impl)  │    isolated address spaces
├─────────────────────────────────────────┤
│         KERNEL SPACE (microkernel)      │
│   Scheduling · Memory · IPC · Capabilities
│                                         │
│   IN-KERNEL BY EXCEPTION ONLY           │
│   Serial (guaranteed panic output) · PIT│
├─────────────────────────────────────────┤
│      CPU ARCH LAYER (x86_64) + LIBK     │
├─────────────────────────────────────────┤
│                 HARDWARE                │
└─────────────────────────────────────────┘
```

Only two drivers live in-kernel, by exception: **serial** (panic output must survive a dead
userspace) and **PIT** (the scheduler tick). Everything else restarts when it crashes — your
network driver taking down the system is a Linux bug class this design eliminates by construction.

The full vision — including hardware tagged memory (à la CHERI), distributed capabilities and
real-time scheduling — lives in [`Ascension.md`](Ascension.md).

## Status & roadmap

**Pre-production.** Nothing boots yet — deliberately. The compiler comes first, because nothing
else can even be written before it exists:

```
LANG (ascc) → LIBK → BOOT + ARCH → MM → DRV → PROC → FS → SH + BLD → QA → CR → DOCS
```

| Wave | Deliverable |
|---|---|
| LANG | `ascc` v0 — lexer, parser, type checker (capabilities/channels/ownership), LLVM codegen |
| BOOT / ARCH | Multiboot2, long mode, GDT/IDT, ISR trampolines, context switch |
| MM / PROC | PMM, VMM, arenas · scheduler, typed IPC, capability table |
| FS / SH | objfs typed namespace · `embers` shell |
| QA | QEMU boot tests — including a low-memory configuration, plus ISO/kernel-size tracking |

This is a decade-scale hobby project, done properly or not at all.

## License

GPL-3.0 — anyone may use, study, modify and redistribute Ascension, but every derivative stays
free under the same terms. See [LICENSE](LICENSE).
