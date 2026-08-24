# Ascension: build system
#
#   make / make all    -> bootable ascension.iso
#   make ascc          -> Asc compiler (host tool; Rust + LLVM)
#   make kernel        -> ALL kernel objects (every kernel/**/*.{asc,asm}, mirrored under build/)
#   make link          -> linked higher-half kernel ELF (+ FOOTPRINT size report)
#   make iso           -> GRUB2 rescue ISO
#   make run           -> boot in QEMU, serial on stdio
#   make run-log       -> boot in QEMU, serial captured to build/serial.log
#   make link-selftest -> selftest variant ELF (entry: kernel_selftest.asc)
#   make iso-selftest  -> selftest ISO (identical pipeline, different entry unit)
#   make run-selftest  -> boot the selftest ISO in QEMU
#   make clean
#
# Toolchain: ascc (ours) · nasm · GNU ld · grub-mkrescue · qemu-system-x86_64

ASCC_REPO := tools/ascc
ASCC      := $(ASCC_REPO)/target/release/ascc
NASM      := nasm
LD        := ld

BUILD := build

QEMU_FLAGS := -serial stdio -display none -no-reboot

# --- Kernel sources -> objects -------------------------------------------------
#
# Discovery is automatic: every .asc AND .asm under kernel/ compiles to a
# mirrored path under build/ (e.g. kernel/libk/types.asc -> build/kernel/libk/types.o,
# kernel/arch/x86_64/gdt_flush.asm -> build/kernel/arch/x86_64/gdt_flush.o).
# Adding a new source file requires ZERO Makefile edits: the lists below are
# recomputed from the tree on every invocation.
#
# EXCEPTION TO THE WILDCARD: entry units. kernel/core/kernel.asc and
# kernel/core/kernel_selftest.asc both define kmain() and must NEVER be linked
# into the same image. They are named explicitly here and excluded from the
# shared library object set; each image adds back exactly ONE named entry
# object. No image ever gets its kmain by wildcard accident.

KERNEL_SRCS := $(sort $(shell find kernel -type f \( -name '*.asc' -o -name '*.asm' \)))
KERNEL_OBJS := $(patsubst kernel/%.asc,$(BUILD)/kernel/%.o,$(filter %.asc,$(KERNEL_SRCS))) \
               $(patsubst kernel/%.asm,$(BUILD)/kernel/%.o,$(filter %.asm,$(KERNEL_SRCS)))
KERNEL_DIRS := $(sort $(dir $(KERNEL_OBJS)))

BOOT_OBJ := $(BUILD)/boot.o

ENTRY_UNIT    := kernel/core/kernel.asc
SELFTEST_UNIT := kernel/core/kernel_selftest.asc

obj_of = $(patsubst kernel/%.asc,$(BUILD)/kernel/%.o,$(1))

ENTRY_OBJ    := $(call obj_of,$(ENTRY_UNIT))
SELFTEST_OBJ := $(call obj_of,$(SELFTEST_UNIT))

# Library objects: everything EXCEPT the two mutually-exclusive entry units.
LIB_OBJS := $(filter-out $(ENTRY_OBJ) $(SELFTEST_OBJ),$(KERNEL_OBJS))

IMAGE_OBJS          := $(BOOT_OBJ) $(LIB_OBJS) $(ENTRY_OBJ)     # -> ascension.elf
SELFTEST_IMAGE_OBJS := $(BOOT_OBJ) $(LIB_OBJS) $(SELFTEST_OBJ)  # -> ascension-selftest.elf

# --- Products ------------------------------------------------------------------
KERNEL_ELF   := $(BUILD)/ascension.elf
SELFTEST_ELF := $(BUILD)/ascension-selftest.elf

ISO               := ascension.iso
SELFTEST_ISO      := ascension-selftest.iso
ISO_STAGE         := $(BUILD)/iso
SELFTEST_ISO_STAGE := $(BUILD)/iso-selftest

.PHONY: all ascc kernel link link-selftest iso iso-selftest \
        run run-log run-selftest clean

# --- Top-level goals -------------------------------------------------------------

all: iso

ascc: $(ASCC)

$(ASCC): tools/ascc/src/*.rs tools/ascc/Cargo.toml
	cargo build --release --manifest-path $(ASCC_REPO)/Cargo.toml

# --- Compilation ------------------------------------------------------------------

# Output directories are order-only prerequisites: creating them does not
# count as an update to the objects that live inside them.
$(BUILD) $(KERNEL_DIRS):
	mkdir -p $@

$(BOOT_OBJ): boot/boot.asm | $(BUILD)
	$(NASM) -f elf64 $< -o $@

.SECONDEXPANSION:
$(BUILD)/kernel/%.o: kernel/%.asc $(ASCC) | $$(dir $$@)
	$(ASCC) --kernel $< -o $@

$(BUILD)/kernel/%.o: kernel/%.asm | $$(dir $$@)
	$(NASM) -f elf64 $< -o $@

kernel: $(KERNEL_OBJS)

# --- Linking -----------------------------------------------------------------------
#
# FOOTPRINT policy (CLAUDE.md, non-negotiable): every link reports section
# sizes. A size regression is flagged exactly like a functional regression.

define LINK_KERNEL # (1: objects, 2: output elf)
	$(LD) -nostdlib -static -z noexecstack -T linker.ld $(1) -o $(2)
	@printf 'FOOTPRINT (%s): ' '$(notdir $(2))'
	@size $(2) | awk 'NR==2 { printf "text=%d data=%d bss=%d total=%d\n", $$1, $$2, $$3, $$4 }'
endef

link: $(KERNEL_ELF)

$(KERNEL_ELF): $(IMAGE_OBJS) linker.ld
	$(call LINK_KERNEL,$(IMAGE_OBJS),$@)

link-selftest: $(SELFTEST_ELF)

$(SELFTEST_ELF): $(SELFTEST_IMAGE_OBJS) linker.ld
	$(call LINK_KERNEL,$(SELFTEST_IMAGE_OBJS),$@)

# --- ISO ----------------------------------------------------------------------------
#
# Both ISOs share one recipe shape; each stages into its own tree so building
# one never clobbers the other. grub.cfg always loads /boot/ascension.elf, so
# each stage installs ITS OWN ELF under that name.

define BUILD_ISO # (1: stage dir, 2: kernel elf, 3: iso out)
	mkdir -p $(1)/boot/grub
	cp $(2) $(1)/boot/ascension.elf
	cp boot/grub.cfg $(1)/boot/grub/grub.cfg
	grub-mkrescue -o $(3) $(1) 2>/dev/null
endef

iso: $(ISO)

$(ISO): $(KERNEL_ELF) boot/grub.cfg
	$(call BUILD_ISO,$(ISO_STAGE),$(KERNEL_ELF),$@)

iso-selftest: $(SELFTEST_ISO)

$(SELFTEST_ISO): $(SELFTEST_ELF) boot/grub.cfg
	$(call BUILD_ISO,$(SELFTEST_ISO_STAGE),$(SELFTEST_ELF),$@)

# --- Run ------------------------------------------------------------------------------

run: $(ISO)
	qemu-system-x86_64 -cdrom $(ISO) $(QEMU_FLAGS)

run-log: $(ISO)
	qemu-system-x86_64 -cdrom $(ISO) $(QEMU_FLAGS) -serial file:$(BUILD)/serial.log &

run-selftest: $(SELFTEST_ISO)
	qemu-system-x86_64 -cdrom $(SELFTEST_ISO) $(QEMU_FLAGS)

# --- Clean ------------------------------------------------------------------------------

clean:
	rm -rf $(BUILD) $(ISO) $(SELFTEST_ISO)
