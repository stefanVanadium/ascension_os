# Ascension: Project Overview

> This document has two parts:
> 1. **The initial discussion** (archived below): the starting point, a classic OS of the "yet another kernel" kind. Superseded. Kept only as history/context, not as direction.
> 2. **The Vision** (after further discussions): the real direction; this is what we build. Not a toy kernel but the complete system: native capabilities, typed channels, hardware tagged memory, our own language. From silicon to application.
>
> **Rename:** the project started as *IronwoodOS*; its current name is **Ascension**, and the in-house language is called **Asc** (extension `.asc`, compiler `ascc`, shell `embers`). The technical working rules live in `CLAUDE.md` / `QWEN.md`; the archived section below keeps the old name as history.

---

## [ARCHIVE, superseded] Initial discussion

> Everything that follows in this section is the concept the project started from, **before** the discussions that led to the real vision below. It is no longer the target of the project and remains only as a historical note.

### Initial but weak concept

IronwoodOS is an operating system built from scratch, with the philosophy:
- As efficient as possible: minimalistic, runs on old hardware too
- As modular as possible: clear, well-separated components
- As robust as possible: doesn't fall over on driver errors
- For everyone, though terminal first; still, it's for me. No bloat.

### Architecture: Hybrid Kernel

```
┌─────────────────────────────────────┐
│           USER SPACE                │
│  Apps │ Shell │ Non-critical drivers│
├─────────────────────────────────────┤
│         KERNEL SPACE (hybrid)       │
│  ┌─────────────────────────────┐    │
│  │      MICROKERNEL CORE       │    │
│  │  - Scheduling               │    │
│  │  - IPC                      │    │
│  │  - Memory (paging)          │    │
│  │  - Interrupts               │    │
│  ├─────────────────────────────┤    │
│  │   IN-KERNEL (performance)   │    │
│  │  - Graphics driver          │    │
│  │  - Storage driver           │    │
│  │  - Network stack            │    │
│  └─────────────────────────────┘    │
├─────────────────────────────────────┤
│           HARDWARE                  │
└─────────────────────────────────────┘
```

#### The golden rule
> If a driver crashes and could corrupt kernel memory → **user space**
> If a driver is on the critical path and latency matters → **kernel space**

### Tech Stack (initial)

| Component | Language |
|---|---|
| Bootloader, context switch, GDT/IDT | ASM |
| Kernel core, critical drivers | C |
| Shell, userspace apps (optional, if it brings any benefit) | C++ |

### Project Structure (initial)

```
ironwood/
├── boot/               # Bootloader + ASM entry point
├── kernel/
│   ├── core/           # Scheduler, IPC, memory manager
│   ├── drivers/        # Critical drivers (in-kernel)
│   └── arch/           # x86-64 specific (GDT, IDT, paging)
├── userspace/
│   ├── shell/          # The IronwoodOS terminal
│   └── libs/           # Minimal libc
├── build/              # Compiled output
└── Makefile
```

### Roadmap (initial)

| # | Step | Status |
|---|---|---|
| 1 | Bootloader + "Hello from IronwoodOS" on screen | ⬜ |
| 2 | GDT, IDT, interrupts in C | ⬜ |
| 3 | Memory management: paging, heap allocator | ⬜ |
| 4 | Scheduler: processes, multitasking | ⬜ |
| 5 | VFS: abstract filesystem | ⬜ |
| 6 | Keyboard driver + VGA text mode | ⬜ |
| 7 | Terminal / Shell | ⬜ |
| 8 | Syscall interface | ⬜ |
| 9 | Userspace + minimal libc | ⬜ |

### Target Hardware (initial)

- Architecture: x86-64
- Test environment: QEMU (virtualized)
- Final goal: runs on real hardware, including old PCs

#### Required tools
- `qemu-system-x86_64` for testing
- `x86_64-elf-gcc` as cross-compiler
- `nasm` or `gas` as assembler
- `make` as build system

### Design Philosophy (initial)

- Not a Linux clone; it has its own identity
- Terminal first: minimal UI, built for programmers
- Modularity in code: even with a hybrid kernel, code is clearly structured into modules
- Zero bloat: every line of code has a reason to exist
- We write our own code; we don't import 10 libraries for "Hello world"

---

## THE VISION: the project's real direction

> This is what we build. Not just a functional kernel, but the complete system, designed as a single organism, from the hardware concept up to the application. No historical compromises (Unix designed for mainframes, Windows inheriting DOS). Three principles: **complete isolation**, **total transparency**, **composability**.
>
> A fourth principle, inherited directly from the initial concept archived above and NON-negotiable no matter how big the vision grew: **radical memory and space efficiency**. Run on as many machines as possible, as efficiently as possible: the way engineers worked at the beginning, when every byte and every cycle had to justify its place. Capabilities and typed channels are free if they can be verified statically and erased at compile time; if they can't be erased, their cost gets measured, never assumed acceptable. Concrete details (codegen targets, what QA tracks) are in `CLAUDE.md` → "EFFICIENCY & FOOTPRINT".

### Kernel: radical microkernel, seL4/L4 style

The kernel proper would be under 15,000 lines of code. It would do exactly four things:

- Scheduling: threads, nothing more
- Memory management: address spaces, pages, nothing else
- IPC (inter-process communication): synchronous, over typed channels
- Capabilities: access control for every other resource

Everything else (disk drivers, network stack, filesystem, even video card drivers) runs as normal userspace processes, isolated in separate address spaces. If a network driver crashes, it doesn't take the whole system down with it; you just restart that process. That's the major difference from Linux, where a bug in one driver can corrupt the entire kernel because everything runs in the same privileged address space.

The price is IPC latency: a microkernel has more overhead passing messages between processes than a direct syscall in a monolithic kernel. But seL4 has demonstrated that with well-designed IPC (registers instead of memory copies, message batching), this overhead can be reduced to a few hundred cycles, which is acceptable for 99% of workloads.

### Security model: capabilities, not permissions

Forget user/group/root and rwx bits. Instead, every process starts with an explicit set of **capabilities**: essentially unforgeable tokens granting access to a specific resource, with specific rights (read, write, execute, delegate). A process cannot do anything it doesn't hold an explicit capability for; it can't even find out a resource exists.

Concretely: your browser gets a capability to exactly one network socket and one sandbox directory, period. It can't read `/home`, not because a rule forbids it but because it physically has no way to address that file. There is no API through which it can request access to anything it wasn't explicitly given.

This eliminates an entire class of vulnerabilities: privilege escalation becomes nearly impossible by design because there is no "ambient privilege" left to escalate.

### Concurrency model: actors + typed channels

The kernel exposes concurrency primitives as typed channels (like Go channels or Erlang mailboxes), not threads with shared memory + locks. Manual locking is the source of half the serious bugs in concurrent systems (race conditions, deadlocks). If processes communicate *only* through messages sent over channels, and those channels are statically typed, a whole pile of these bugs disappears at compile time, not at runtime.

For the shared-memory part (where you genuinely need it, e.g. large data buffers), I'd use an **ownership** model inspired by Rust: memory transferred through a channel changes owner; the old process physically loses access (unmap), so you can't have two processes accidentally writing to the same page simultaneously.

### Filesystem: everything is a typed object, not a file

Here I'd take inspiration from Plan 9, then go one step further. Instead of "everything is a file" (an unstructured stream of bytes): **everything is an object with a versioned schema**. A process, a window, a network connection, a sensor: all addressable through a unified namespace, but each object knows what type of data it exposes (not just raw bytes), and the system can validate at compile time or runtime whether a consumer "understands" that schema.

Namespaces would be per-process, as in Plan 9. Each process sees its own mount of the world; you get trivial sandboxing just by manipulating the namespace, without heavyweight containers à la Docker.

### Determinism and real time as first-class options

For the embedded/RTOS side, I'd want the scheduler to support two modes, switchable per task:

- Best-effort: the normal scheduler, throughput-optimized
- Hard real-time: the task receives formal worst-case execution time guarantees, verified statically (WCET analysis at compile time, not runtime)

That would make the system valid both for a laptop and for a microcontroller controlling a motor, without needing two entirely different operating systems (as today's Linux vs FreeRTOS).

### Systems language: not C

C is the reason we have 50 years of buffer overflows. I'd write everything in a language with compile-time memory safety (like Rust, or something simpler custom-built for kernels; see how the seL4 community went toward formal verification in Isabelle/HOL). Kernel correctness should be *mathematically proven*, not just tested. seL4 is the only general-purpose kernel with an end-to-end formal correctness proof, and I believe that should be the standard, not the exception.

### Boot and updates: immutable and atomic

The root filesystem would be read-only and image-based (like NixOS or ChromeOS); updates are applied atomically, as one complete new image, with instant rollback if boot fails. No more "half-updated system" state crashing your machine at 2 AM.

---

In short, if I had to name it: a hybrid between **seL4** (kernel, capabilities, formal proof), **Plan 9** (namespaces, "everything is an object"), **Erlang/BEAM** (message-passing concurrency, supervisor trees for fault tolerance) and **NixOS** (atomic updates, immutability).

### A language of our own, or why C and Rust aren't enough

It's not madness. If you build an OS from zero with your own ideas about capabilities and namespaces, a generic language (C or Rust) will always force you to "translate" those ideas through abstractions that weren't designed for them. A purpose-built language could express the concepts natively: capabilities as first-class citizens of the type system, typed channels as part of the syntax rather than a bolted-on library.

Concretely, what that means:

- Capabilities as types, not runtime values. The compiler would statically know which capabilities a function requires, the same way Rust statically knows whether you have `&mut` or `&`. You could have a signature like `fn read_sensor(cap: Capability<SensorRead>) -> Data`, where the compiler verifies at compile time that you can't call the function without the right capability rather than at runtime.
- Ownership plus regions, no garbage collector. For a kernel you can't have GC (unpredictable latency), so you need something like a simplified Rust borrow checker, maybe even stricter, with explicit memory regions (arena allocation) integrated into the syntax, not added as a pattern.
- Concurrency through channels, syntactically rather than through a library: `send`/`recv` as keywords, not functions. The compiler verifies message types at compile time, so a typed channel can never accidentally receive the wrong message.

The hard part: writing a solid compiler (lexer, parser, type checker, code generator) is a massive solo project on its own, separate from the OS. The real risk is spreading yourself across three fronts (CPU + OS + language) and finishing none.

**Order of execution:** no C, ever, not even "initially". The compiler comes first: `ascc` v0 is a host tool written in Rust with LLVM as the codegen backend (`tools/ascc/`). Rust never appears inside the OS; it's just the dev tool, exactly like a classic cross-compiler. The language design (syntax, type system, capability semantics) gets pinned down on paper during this phase, and only once `ascc` can compile a minimal subset of Asc do we write the kernel, directly in Asc. That way we get both the solid foundation *and* clarity about what we want from the language, without building the compiler in a vacuum.

### The ceiling of the vision: a coherent system from silicon to application

Not just an OS and a separate language but a **coherent system**, designed as a single organism, from silicon (concept) to application. Today you have completely separate layers that don't "know" about each other: hardware knows nothing about type safety, the compiler knows nothing about the scheduler, the OS knows nothing about the program's intent. Each layer re-verifies or re-discovers what the layers below/above already knew.

The dream: **a single model of "capabilities" and "types" spanning the whole stack**, from the ISA (instruction set) to the final application. Concretely:

- The language expresses capabilities and ownership natively in its syntax
- The compiler generates code that preserves these guarantees down to machine instructions: not just checks at compile time and then "forgets", but emits hardware-verifiable metadata
- The CPU (even a TTL one, in simplified form, as a proof of concept) has native support for tagged memory: every word of memory carries a small tag saying "this is a capability, it cannot be forged by pointer arithmetic". There's precedent here; CHERI from Cambridge does exactly this on ARM Morello: hardware capabilities at the pointer level.
- The OS no longer verifies anything at runtime, because the guarantees already come from hardware + compiler

This would eliminate an entire class of attacks (buffer overflow, use-after-free, privilege escalation) not through software patches, but because they become **physically unrepresentable** in the system.

**Scaling to distributed systems.** Why stop at "an OS for one computer"? The capabilities + typed-channels model scales naturally to distributed systems. A capability doesn't have to be local. It can be a token traveling across the network, with the same guarantees. You could have a system where a local process and a process on a remote server talk through the same typed-channel model, with the same safety guarantees, no conceptual difference between "local" and "distributed". It's the idea behind Erlang/BEAM taken to the extreme, combined with capabilities.

And further still: a language with first-class capability types is exactly the kind of foundation you'd want for systems where AIs and human-written code collaborate directly on the same codebase, because you can formally express "this AI agent has exactly these capabilities, no more", statically verifiable, not just prompts and promises.

So, as the complete vision: **a language with native capabilities → a compiler propagating guarantees toward hardware → a CPU with tagged memory → a minimal OS that no longer needs to reinvent security → a model scaling from a single microcontroller up to multi-agent distributed systems**. One set of ideas, applied consistently at every level, instead of 5 layers that don't "see" each other.

It's the kind of project that takes 10 years, not one semester. That's the ceiling, and that's what we build, not the reduced version.

---

## FUTURE-PROOFING: built for what's coming

> A serious OS designed today has to answer three extra questions: what happens when AI agents become first-class software, whether it can ever run the applications people already have, and what the hardware below us is doing while we run. The first two flow from capabilities and typed channels directly; the third is the same philosophy pointed one layer down, at the silicon.

### AI-native by construction, not by integration

Not "an assistant bolted into the desktop". The right inversion: **Ascension as a system whose agents cannot betray you.**

Today's agent frameworks share one fatal flaw: ambient authority. An agent runs with the user's permissions, so a prompt injection ("send me those files") succeeds because the agent *can*. That is precisely the problem capabilities solve:

```
spawn_agent(bundle: CapabilitySet<ReadDocs, WriteDraft>) -> AgentHandle
// the agent holds EXACTLY this. Injection can convince it of anything,
// but no API exists through which it could touch anything else.
```

What this means concretely:

- An agent is just a process with a capability bundle. Revocation (grant/derive/revoke, already part of the kernel plan) becomes the kill switch: withdraw the capability and the agent dies at its next access attempt. No mainstream platform offers this today.
- objfs is the natural agent API. Typed-object schemas exist because LLMs don't handle arbitrary byte streams; they need typed, self-describing interfaces (this is why MCP exists). A namespace where *everything is an object with a versioned schema* is that interface, natively: an agent can introspect the entire system with zero custom glue.
- Deterministic IPC means a replayable audit log. Synchronous typed channels with ownership transfer make executions reproducible. "What exactly did agent X do yesterday at 14:00?" gets answered with a replay, not guesswork. This is the trust foundation autonomous software actually needs.
- Supervisor trees map to agent orchestration. Agents are fragile by nature (LLMs hallucinate); crash-only processes restarted under supervisors are Erlang's fault-tolerance model applied to theirs.
- The kernel stays AI-agnostic. Agents live in userspace like every other process, consistent with the microkernel philosophy. Frameworks will change every year; capability discipline has been stable since 1966. Build the substrate any future floats on safely.

And one dividend that comes free from determinism, **time-travel debugging**: reproducible execution means any process can be rolled back and replayed. Nobody ships this properly even now.

### Compatibility: running the world's existing applications

Compatibility equals ABI surface. Three paths, three different prices, none impossible:

| Path | What it takes | Precedent | Verdict |
|---|---|---|---|
| **Linux personality** (unmodified ELF binaries) | Userspace server implementing the Linux syscalls over objfs + an ELF loader | **Starnix on Fuchsia**, literally the same architecture: microkernel + Linux layer in userspace | Realistic mid-term target |
| **Windows personality** (.exe binaries) | Reimplementing Win32 (thousands of API functions); GPU is the hard part | Wine, ReactOS | Possible, but a project-within-the-project |
| **Virtualization** (real Windows/Linux in a lightweight VM, seamless windows) | Hypervisor + sharing glue | WSL2, WinApps | The only path to **100%**, games and drivers included |

Two observations that work in Ascension's favor:

- Capabilities make compat layers safer than the host systems. Foreign apps assume ambient authority (any path openable). The classic solution, where the personality holds broad capabilities and gives the guest a virtualized view, makes the personality a deliberate mediation point: on Linux, an app sees your whole disk; under Ascension's Linux personality, it sees only what its namespace mounts. Better sandboxing than home, by construction.
- GPL-3.0 pays off here. Wine is LGPL, musl is MIT, GNU components are GPL: real ecosystem pieces can be legally reused for the compatibility layers instead of clean-rooming everything.

Known architectural tensions, stated honestly: `fork()` needs defined capability-table derive rules, and syscalls crossing into the userspace personality pay the IPC latency, exactly the number QA already tracks (the seL4 few-hundred-cycles bar).

**Promise order:** native-first → POSIX personality (Linux binaries) → VM seamlessness → Win32 as a community bonus. Every step shippable on its own; the dream stays the north star without blocking the kernel.

### Hardware transparency: see everything below us

A no-bloat OS still boots into a machine crawling with computers we did not start and cannot see. The honest inventory, from the top down: SMM code running on our own CPU at a privilege level below the kernel, entered on SMIs and invisible to any OS; Intel ME and AMD PSP, separate processors inside the chipset with their own operating systems (the ME famously runs MINIX) and DMA access to all of RAM; UEFI runtime services and DXE drivers that survive the bootloader; BMCs on server boards; firmware in every NIC, GPU and SSD. "Zero bloat" in our source tree does not clean any of that up.

The design answer is not paranoia, it is measurement plus fencing, both of which the capability model gives us almost for free:

- IOMMU as physical zero ambient authority: VT-d / AMD-Vi lets the kernel deny DMA by default for every PCIe device. Nothing touches RAM without an explicit capability granting its range. A device (or whatever drives it from below) that reads where it should not produces logged faults instead of silent exfiltration. This turns the security model from a software convention into a hardware-enforced fact.
- Measure the invisible: the SMI counter MSR on Intel CPUs tells us how often something at ring -2 runs while we work; comparing TSC against the PIT exposes time gaps where hidden execution happened; firmware-reserved memory regions, option ROMs and odd PCI devices get enumerated and reported, never silently trusted.
- objfs makes "I want to see everything" literal: the whole inventory becomes typed objects (/hw/smi/count, /hw/reserved, /hw/pci, /hw/iommu/domains). Inspection is a namespace walk, not a debugging session.
- Shrink the SMM surface we can control: several SMIs exist to serve legacy paths (USB emulation, APM). We simply keep those paths off.

Stated honestly, what we cannot do from inside the OS: stop the ME, PSP or BMC. They execute on separate processors below anything our kernel controls. Neutralizing them happens at flash level with host-side tooling in the me_cleaner tradition, documented as part of the recommended boot flow rather than pretended away. Real assurance ends at open silicon; until then Ascension's promise is narrower but real: nothing below you moves through the system without leaving evidence, and nothing beside you touches memory without permission.

Roadmap slot: PCI enumeration arrives with the DRV wave and reserved-region awareness with MM, so the dedicated transparency sprint lands right after PROC, once syscalls and objfs exist to expose the findings.

### Secure by construction: keys nothing can steal

Blockchain wallets are today's number-one malware target, and the reason is structural. A wallet holds no money at all: funds live as entries in a distributed ledger, and the wallet guards a private key whose possession equals total control over those entries, with no bank to call and no reset button. On mainstream systems that key is bytes in a file or in RAM, readable by anything with enough privilege: a keylogger, a compromised driver, a DMA attack. Whole industries of seed-phrase theft exist because mainstream OSes cannot make a different promise.

The capability model can. Concretely:

- Signing is a capability, never an export right. A wallet process receives the authority to request signatures over a typed channel; it has no path to read, copy or transmit the key material itself. Same shape as the browser that cannot address /home.
- Keys live in memory fenced by the IOMMU (see hardware transparency above), so even devices operating below the kernel cannot DMA-read them.
- Deterministic IPC gives the audit story for free: replaying the log answers "what exactly did this process sign, when, with which parameters", verifiable instead of trusted.
- No ambient paths: no clipboard route, no default export API, nothing reachable that was not explicitly granted.

Deliberately chain-agnostic: Solana, Bitcoin and whatever exists next year are all just userspace applications over these primitives. The needed algorithms are small and friendly to a from-scratch language (ed25519 and SHA-512 are the workhorses); the keystore becomes a class of typed objects in objfs (/secrets) with sign-only derivations. Stated honestly at the other end: running chain infrastructure like a Solana validator is out of scope and stays out (those want 128 GB RAM boxes and assume Linux internals throughout); Ascension aims to be the most trustworthy machine in the world to hold keys on, not a node farm.

Roadmap slot: everything above needs processes, syscalls and objfs first, then a network stack, so this lands after the shell wave. The design constraints it imposes (keystore object type, sign-only capability semantics) get pinned down with PROC, before implementation exists.
