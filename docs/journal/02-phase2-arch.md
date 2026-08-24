# Phase 2: ARCH, meaning interrupts are real

> Sprint: Faza 2 (BOOT+ARCH → QA → CR → DOCS) · Completed: 2026-08-24
> Deliverable: the CPU architecture layer is live: GDT/TSS/IDT/PIC in
> `kernel/arch/x86_64/`, PIT at 100 Hz in `kernel/drivers/`, 256 interrupt
> trampolines and an Asc dispatcher proven to survive a full register round
> trip. Until this sprint nothing asynchronous could touch the kernel; from
> here any instruction boundary can be interrupted.

## What shipped

```
kernel/arch/x86_64/
  io.asc          outb/outw/inb/inw asm primitives + store_u*_le helpers
                  (why they exist: known debt)
  gdt.asc         long-mode GDT built at runtime into a static: null,
                  kernel code/data, user code/data (selectors 0x08..0x20),
                  16-byte TSS descriptor at 0x28
  tss.asc         104-byte TSS plus its stacks: RSP0 -> int_stack (16 KiB),
                  IST1 -> df_stack (4 KiB); ltr through an asm block
  idt.asc         all 256 gates, type 0x8E, selector 0x08; vector 8 gets IST1
  isr_stubs.asm   irreducible. Two stub classes: ERRCODE vectors (8,10,11,
                  12,13,14,17,21) keep their CPU-pushed code, everyone else
                  pushes a dummy qword; isr_common saves 15 GPRs, calls
                  isr_dispatch_asc with RSP, restores, drops 2 slots, iretq.
                  256-entry qword pointer table in .rodata (stubs differ in
                  length)
  isr.asc         ISRFrame mirroring those 22 qwords; dispatcher halts on
                  exceptions with a full dump, runs pit_tick + EOI on IRQ0,
                  returns WITHOUT EOI on spurious master vector 39
  pic.asc         8259 cascade remapped to vectors 32..47, slave fully
                  masked, IRQ0 the only live line
kernel/drivers/
  pit.asc         channel 0 mode 3, divisor 11931 = 1193182/100 Hz;
                  pit_tick only bumps a static counter, no serial in the IRQ
boot/boot.asm     kmain(mb_magic, mb_info): magic into edi right after verify
                  (page tables spend eax), info ptr rides esi, physical,
                  valid while boot's identity map holds
kernel/core       bring-up gdt->tss->idt->pic->pit, sti LAST, stage markers
                  printed BEFORE each step so a hang names its own cause;
                  spinlock_acquire_irq around every tick read under live fire
tools/ascc        module-level statics + two bug fixes (below). Tests 15->24
Makefile          kernel/**/*.asm found automatically, nasm -f elf64 pattern
                  rule, zero manual object lists
```

## The frame ABI: one owner wrote both sides

isr_stubs.asm builds the frame bottom-up (15 GPRs, vector, error code, then
the CPU's own rip/cs/rflags/rsp/ss) and ISRFrame in isr.asc mirrors those 22
qwords field for field. One wave owns both files on purpose: the layout comment
in the ASM is the spec, the struct is its mirror, drift cannot hide. The proof
is end-to-end: plant recognizable constants into rax/rbx/rcx through pinned asm
inputs, execute ud2, read the dump. EXCEPTION 6 arrives with
rax=0xA1A2A3A4A5A6A7A8 intact; offset wrongness between CPU and Asc shows up
as garbage in exactly that dump. The stub table crosses into Asc as a function
return value (`isr_stub_table_ptr()`), not a data symbol: ascc links functions
across units but cannot name extern data yet.

## Statics: reality disagreed with the plan

The plan said no language work this sprint. Hardware tables and counters must
outlive function calls and there was no honest way to write them otherwise, so
module-level statics landed anyway: `let name: T;` at module level, annotation
required, initializer forbidden, zero-initialized storage, unit-private
internal linkage, guaranteed 16-byte alignment. Cross-unit sharing goes through
accessor functions only, which doubles as discipline: nothing global is
nameable outside its unit except through a function the owner chose to expose.

Two latent compiler bugs surfaced because this sprint was their first customer:

1. `#[packed]` never parsed: the lexer emits `packed` as a keyword token while
   the attribute matcher compared against an identifier, so the arm could not
   fire. Every packed-struct claim in LANGUAGE.md v1 was false until today.
   Fixed end to end (parser through LLVM `<{ }>` layout) with a negative test
   pinning the attribute to structs only.
2. Same-unit use-before-definition mangled the definition: codegen added a new
   function instead of reusing the forward declaration, LLVM renamed the body
   to `f.1`, the link died with an undefined reference. Codegen reuses the
   declaration now, and run.sh grew EXPECT-DEF alongside EXPECT-UNDEF so nm
   catches this class permanently.

## Decisions worth keeping

User segments sit in the GDT unused. PROC consumes real selectors next sprint
instead of churning the table later. IST1 goes live immediately for the same
reason: the double-fault gate needs dry ground even when RSP itself faulted.
Vector 39 (spurious IRQ7) is deliberately not acknowledged; EOI on a spurious
desyncs the PIC's in-service register, so the dispatcher just returns.
spinlock_acquire_irq was "correct by construction" since Faza 1; it now runs
under live interrupt fire around every tick-counter read, with the first pass
reporting whether IF survived acquisition ("ARCH: irq lock saw IF set").

## Verification evidence

| Check | Result |
|---|---|
| Normal boot (-m 128) | BOOT x4 -> LIBK -> gdt/tss/idt/pic/pit -> irq lock IF proof -> tick markers at wall-clock 1 Hz, zero PANIC/EXCEPTION lines |
| Selftest ISO | ud2 fires, EXCEPTION 6 err=0, planted rax/rbx/rcx intact in the dump, parks clean |
| Low RAM | boots at -m 4M, NOT at 2M: GRUB inside 2 MiB exhausts budget before handover (environmental, bss grew 36 -> 64 KiB). New QA bar: 4M |
| Footprint | text 1721 -> 8843 B (+7122): ~2.7 KiB trampoline code, 2048 B stub pointer table (.rodata), dispatcher + dump printers; bss 36 -> 64 KiB (int_stack, df_stack, IDT gates, tables) |
| Hygiene | exactly 1 iretq in the linked kernel, 0 SSE/FPU instructions, entry 0xFFFFFFFF80100020 unchanged, noredzone present |
| Compiler tests | 24/24 |

CR findings fixed during review, both invisible until the exact event they
exist for: TSS.IST1 was stored at offset 40 instead of 36 (SDM layout: RSP0 @4,
RSP1 @12, RSP2 @20, reserved @28, IST1 @36), which lands the double-fault
handler on garbage and triple-faults; IOPB sat at 102 instead of 100. Offsets
are named consts now (TSS_OFF_RSP0 / IST1 / IOMAP).

## Known debt

GDTR, IDTR and the whole TSS are written byte-wise through store_u*_le because
#[packed] was broken when ARCH was written; migrating hardware structs to real
#[packed] types is follow-up work now that the fix landed. The decimal printer
exists three times (~330 B each in kernel.asc, isr.asc, panic.asc) until
modules land.

## What's next (Faza 3)

MM: PMM bitmap fed by the multiboot2 memory map through the boot_mb_info()
handover, VMM replacing bootstrap paging, phys window activation. types.asc
stops being a contract and becomes load-bearing.
