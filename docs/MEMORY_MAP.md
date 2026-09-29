# MEMORY_MAP.md

Address space as the kernel builds it after `vmm_init()` returns. Before that
moment boot.asm's static tables own CR3 and only the first 4 MiB are reachable
through the window; the two maps are described separately because they are
genuinely two different address spaces, not two views of one.

These are the addresses the linker script and `kernel/mm/` actually use.
Section ends move with every build; `nm build/ascension.elf | grep kernel_.*end`
prints the current values.

## Physical

```
0x00000000 - 0x000FFFFF   reserved: BIOS, VGA buffer, EBDA. Never RAM for us.
                           Frame 0 is reserved forever so a zero result from
                           pmm_alloc can never pass for a real frame.
0x00100000 - 0x00116FFF   the kernel image. GRUB loads it at 1 MiB; linker.ld
                           puts kernel_end at 0x116000 for the current build
                           (text+rodata+data, then .bss, 16 KiB of ISR stack,
                           4 KiB double-fault stack, the IDT gates, GDT, TSS and
                           boot.asm's seven bootstrap page tables).
0x116000 - 0x1FFFFF       free. pmm_init places the bitmap at the first
                           page-aligned frame at or above kernel_end that is
                           available and clear of the multiboot2 info blob.
0x200000 - top_of_ram     free, minus whatever the PMM hands out. The window
                           covers exactly this, rounded up to 2 MiB.
above top_of_ram          whatever the multiboot2 map did not call available.
                           QEMU reports a reserved hole at 0xFD00000000 and the
                           window does not claim it.
```

The multiboot2 info structure is excluded explicitly. GRUB's map counts the
image, the blob and any modules as available RAM, so excluding them is our job.

## Virtual, after the CR3 switch

```
0x0000000000000000 - 0x00007FFFFFFFFFFF   user space. Unmapped. PML4[0..255]
                                            carries no entry yet; PROC owns this
                                            range and nothing else may claim it.
0xFFFF800000000000 - 0xFFFF800000000000
                     + window_top          the physical window. PML4[256] ->
                                            PDPT -> a chain of PDs, filled with
                                            2 MiB huge pages, RW and NX, from
                                            physical 0 up to the top the map
                                            reported (2 MiB rounded). One PD
                                            covers 1 GiB, so this is a chain
                                            and not a single directory; a
                                            KASSERT bounds it at the 512 GiB a
                                            single PML4 slot can address.
                                            phys_to_virt(p) = p + 0xFFFF800000000000
0xFFFFFFFF80000000 - 0xFFFFFFFF800FFFFF   guard gap, unmapped on purpose. A
                                            wild kernel pointer below the image
                                            faults here instead of landing on
                                            the bootstrap tables.
0xFFFFFFFF80100000 - kernel_text_end       .text, 4 KiB pages, present and
                                            executable, not writable.
kernel_text_end     - kernel_rodata_end    .rodata, present, not writable, NX.
kernel_rodata_end   - kernel_data_end      .data, present, writable, NX.
kernel_data_end     - kernel_end           .bss, present, writable, NX.
0xFFFFFFFF80116000                          kernel_end for the current build.
0xFFFFFFFF80116000 - 0xFFFFFFFF801FFFFF    unmapped, the rest of the 2 MiB
                                            the image sits in.
0xFFFFFFFF80200000 - 0xFFFFFFFF805FFFFF    the kernel arena: 4 MiB of address
                                            space, 2 MiB aligned, pages mapped
                                            into it on demand by arena_alloc.
0xFFFFFFFF80600000 onwards                 free for the next arena. A fourth
                                            arena is a KASSERT, not a silent
                                            overflow of a 4-entry table.
```

W^X is per section, decided by `map_binary_page` from the linker script's own
section ends. A page that straddles a boundary takes the flags of the section it
starts in, so at most a few rodata bytes share an executable page. The other
direction is not exact: `.rodata` and `.data` have no `ALIGN(4096)` between them
in linker.ld, so a page starting in rodata and running into data is mapped
writable. No executable+writable page results either way.

Deliberately unmapped: PML4[511][511] and the 1 MiB below the image. Both catch
wild pointers that would otherwise resolve onto something real.

## Before the switch: boot.asm's tables

Five regions, all built at runtime with the flag bits OR-ed in (link-time
relocations only give addresses, and all tables are 4 KiB aligned so the low 12
bits are free):

```
PML4[0]   -> PDPT_LOW  -> PD_LOW    identity map, first 4 MiB at virtual 0,
                                   2 huge pages, RW
PML4[256] -> PDPT_WIN  -> PD_WIN    the physical window, first 4 MiB, 2 huge
                                   pages, RW. Exists so vmm_init can build real
                                   tables through phys_to_virt before the switch
PML4[511] -> PDPT_HIGH -> PD_HIGH   higher half, 0xFFFFFFFF80000000 -> phys 0,
                                   2 huge pages covering 4 MiB, RW
```

The higher-half mapping is RW and executable, so boot code can run and build.
It is replaced wholesale by the table above the moment CR3 moves, which is why
the W^X story only starts at `vmm_init`. The identity map at virtual 0 does not
survive: nothing is mapped there afterwards.

## What maps what, in one table

| Range | Mechanism | Flags | Owner |
|---|---|---|---|
| window, 2 MiB steps | huge PD entries | RW, NX | vmm_init |
| kernel .text | 4 KiB PT entries | R+X | vmm_init, from linker section ends |
| kernel .rodata | 4 KiB PT entries | R, NX | vmm_init |
| kernel .data/.bss | 4 KiB PT entries | RW, NX | vmm_init |
| kernel arena | 4 KiB PT entries, on demand | RW, NX | arena_alloc via vmm_map |
| user space | nothing | unmapped | PROC |
| guard gap, 1 MiB below image, PML4[511][511] | nothing | unmapped | on purpose |
