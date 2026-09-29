# Phase 3: MM, meaning memory stops being a contract

> Sprint: Faza 3 (LANG → LIBK → MM → QA → CR → DOCS) · Completed: 2026-09-29
> Deliverable: the multiboot2 memory map feeds a bitmap PMM, a fresh set of page
> tables replaces boot.asm's static bootstrap paging, the `PHYS_MEM_BASE` window
> goes live so `phys_to_virt` is a real function and not a comment, and an
> explicit arena allocator is the only sanctioned dynamic allocation path. From
> here the identity map is gone and physical addresses stop being reachable by
> accident.

## What shipped

```
kernel/mm/
  mm.asc          address-space constants in one place: PAGE_SIZE, HUGE_SIZE,
                  ENTRIES_PER_TABLE, PTE_* flags, PTE_ADDR_MASK, PHYS_MEM_BASE,
                  LOW_MEM_CEILING, multiboot2 tag ids, frame math. Consumers
                  redeclare what they need textually until modules exist
  pmm.asc         bitmap allocator over the multiboot2 map. Byte-wise reads of
                  the info blob (it arrives at an arbitrary 8-aligned physical
                  address, typed reads would buy alignment questions during
                  bootstrap paging). One find_mmap_tag locator, entry count
                  divided by the tag's own entry_size, only type 1 opened.
                  Reserve [0,1MiB), the kernel image, the info blob, the bitmap.
                  First-fit lowest, pmm_alloc KASSERTs rather than returning 0,
                  pmm_free KASSERTs on double free. Print-once region inventory
  vmm.asc         new PML4 built THROUGH the bootstrap window, then cr3 load.
                  PML4[256] -> PDPT -> PD chain of 2 MiB huge pages for the
                  window, PML4[511] -> PDPT_HIGH -> 4 KiB pages for the binary
                  with W^X per section. EFER.NXE flipped before any NX bit is
                  written. Identity map dropped with the switch.
                  vmm_map/vmm_unmap/vmm_get_phys/vmm_is_mapped for the runtime
  heap.asc        explicit arenas, bump allocation, pages from pmm_alloc mapped
                  into the arena range on demand. No global heap, no free list.
                  OOM returns a clean zero, never a fabricated pointer
kernel/libk/
  symbols.asc     link-time symbol bridge. ascc emits no extern data symbols, so
                  kernel_end and the section ends come back through inline asm
                  `leaq sym(%rip)`. Single owner of the mechanism
kernel/core/
  boot_steps.asc  the MM bring-up tail shared by BOTH entry units, so the
                  selftest walks the same MM path instead of a copy that rots
kernel/arch/x86_64/
  msr.asc         rdmsr/wrmsr. v0 inline asm allows one output, so rdmsr
                  recovers edx:eax through a pinned pointer input
boot/boot.asm     physical window bootstrap (0xFFFF800000000000 -> first 4 MiB)
                  so MM can build real tables before the cr3 write
tools/ascc        break/continue, linker symbols, ptr<->int casts, packed
                  volatile reads, never-typed expression statements. Tests 24->31
```

## The one design decision everything else hangs off

`phys_to_virt` was a comment until this phase. The plan made the window real:
physical memory is mapped at `0xFFFF800000000000` in 2 MiB huge pages, so a
physical address becomes a pointer through one subtraction-free addition and the
Paddr/Vaddr distinction stops being ceremony. Everything else follows. Tables
are built as ordinary physical frames and written through the window, the bitmap
lands at a frame the map says is available, the arena pulls pages and maps them.

The window only claims what the multiboot2 map reported as available, rounded up
to 2 MiB, never the `0xFD00000000` region QEMU reports above the hole.

## Three bugs, each found by something the plan insisted on

1. **The bitmap's own frame was never reserved.** `reserve_range` computed
   `end_f = end >> PAGE_SHIFT`, so a reservation that began and ended inside one
   frame marked nothing at all. The bitmap is smaller than a page at small RAM
   sizes, so its self-reservation collapsed to an empty range, the frame stayed
   free, `pmm_alloc` handed it out as the PML4, and the page table then
   overwrote the very bitmap bits it was meant to protect. It surfaced as a
   page fault at `0xffffffffa5103a90` inside the page walk, with the PML4
   pointer and the bitmap start both reading `0x117000`. The fix marks the frame
   containing `end-1`, not the one starting at `end`. This one only reproduces
   below about 1 GiB of RAM, where the bitmap fits in a page; at 1 GiB and up it
   spans eight frames and the old arithmetic happened to be right.

2. **The window wrote past its own page directory above 1 GiB.** One PD holds
   512 huge pages, so 1 GiB. The loop had no bound, and a 4 GiB machine wrote
   2048 entries into a 512-entry table, straight into the frame allocated right
   after it. It booted clean anyway because the victim was the PDPT allocated
   immediately afterwards and `table_new` zeroes on allocation. Reorder one
   allocation and the PML4[511] entry dies. The window is now a PD chain with
   the bound KASSERTed against the 512 GiB a single PML4 slot can address.
   Verified by booting 128M, 1G, 1536M, 2048M, 4096M after the fix.

3. **`rdmsr` did not declare the registers it destroys.** The template writes
   `%eax` and `%edx` and the clobber list said only `"memory"`, so LLVM was free
   to keep a live value in either register across the block and to allocate the
   halves pointer itself to EAX or EDX, which would store the low half to
   address 0. It worked because the allocator happened to pick RDX. One token.

CR found all three plus a fourth worth its own line: `arena_alloc` returned a
bare `u64`, so the one Vaddr-valued API in the phase whose whole job was
establishing that discipline let a caller pass an arena address where a Paddr was
expected without the compiler complaining. It returns `Vaddr` now, and
`arena_create` refuses a range that starts inside a live mapping or overlaps a
live arena, because `arena_alloc` skips the map for pages that are already
mapped and would otherwise hand out the kernel image.

CR also moved the "table page must be under the bootstrap window" assert out of
`table_new` and into a `table_new_bootstrap` that only `vmm_init` calls. The
rule is true during bring-up and a lie afterwards, and the PROC wave's first
userspace address space is a guaranteed way to hear about it.

## Decisions worth keeping

The inline-asm linker symbol works. Wave 0 pinned it with a test before anyone
depended on it, and the AT&T displacement form (`leaq kernel_end(%rip), %0`)
parses where the bracket form does not, so `boot.asm` did not grow a third
kmain argument.

First-fit lowest is not obviously the right policy and it is the right one here:
it keeps every allocation made before the cr3 switch below `LOW_MEM_CEILING`,
which is exactly the range boot.asm's window can reach. The KASSERT in
`table_new_bootstrap` is the enforcement, and it is honest about why the rule
exists.

`kernel_end_phys` returning the section's linked virtual address, with the
subtraction left to the caller, keeps the Paddr/Vaddr hop visible at the
consumer instead of hidden inside the symbol bridge.

## Verification evidence

| Check | Result |
|---|---|
| Normal boot (-m 128M) | BOOT x4 -> LIBK -> gdt/tss/idt/pic/pit -> 7 mmap regions printed -> MM pmm/vmm/window proof/arena -> sti LAST -> tick markers at 1 Hz, zero PANIC/EXCEPTION |
| Selftest ISO | walks the identical MM path, then ud2 gives EXCEPTION 6 err=0, cr2=0, planted rax/rbx/rcx intact |
| High RAM | 1G, 1536M, 2048M, 4096M all reach the window top the map implies and keep ticking |
| Low RAM ladder | 4M boots (712 free frames), 32M, 8M fine. 2M does not: GRUB itself reports "out of memory" before handover. New floor stays 4M, environmental |
| PMM stress | 1000 alloc/free roundtrips, 64-frame burst with per-frame pattern write and verify, alloc after release |
| Arena | 3 MiB across a 2 MiB PD boundary, one byte written per page plus the last byte of each block; OOM returns 0 |
| Reservation accounting | frames 0, 255 (legacy), 256 (kernel), 279, 280 (bitmap) all report set after the fix |
| W^X | text PTE `0x100021` (present, accessed, no RW, no NX), rodata PTE `0x8000000000104001` (present, NX, no RW). Executing rodata faults correctly: EXCEPTION 14 err=0x11, cr2=0xffffffff80104000, I/D bit set. Writing to a read-only page does NOT fault in this environment, on qemu64, Nehalem and Haswell alike, with the TLB flushed by both `invlpg` and a CR3 reload; the PTE contents were read straight from physical memory to confirm they are right. Write-side enforcement is unverified by environment, not a kernel defect |
| Footprint | text+rodata 8843 -> 15861 B (+7018): bitmap PMM, page table builder, window, arena, mb2 inventory. bss 65536 -> 73728 (+8192): boot.asm gained pdpt_table_window + pd_table_window, dead the moment CR3 moves |
| Hygiene | `nm -u` on the linked kernel is empty, no SSE/FPU emitted, `noredzone` on every function |
| Compiler tests | 31/31 |

## Known debt

`vmm_unmap` has no callers, so its `invlpg` and the double-free assert in
`pmm_free` have never executed in a boot. `vmm_map` accepts any `virt`, so a
ring-0 caller can hand user space a page; both want a KASSERT before PROC
touches them. The arena refill path calls `pmm_alloc`, which KASSERT-panics on
exhaustion, so the "OOM returns 0" contract only covers address-space
exhaustion, not physical exhaustion; the comment says so and the mismatch is
deliberate for a boot-time allocator. `pmm.asc` carries a note about the missing
lock, `vmm.asc` and `heap.asc` do not carry the equivalent note about the
missing capability, and `Capability<T>` does not exist in ascc yet, so no
subsystem can satisfy rule 5 today. Three units now depend on that answer.

`linker.ld` puts no `ALIGN(4096)` between `.rodata` and `.data`, so a page that
starts in rodata and runs into data gets `PTE_WRITE | PTE_NX`: read-only strings
in a writable page. No executable+writable page results, so it is not a rule
violation, but it is a hole nobody wrote down. Two pages of alignment would make
the per-section flags exact.

## What's next (Faza 4)

PROC: threads on top of the scheduler, the capability table so `pmm_alloc` and
`vmm_map` stop being ambient authority, and userspace address spaces, which is
the first thing that will need a PML4 below PML4[256] and the first real test of
whether the window's PD chain holds up.
