# Ascension — build system
#
#   make / make all    -> bootable ascension.iso
#   make ascc          -> Asc compiler (host tool; Rust + LLVM)
#   make kernel        -> kernel ELF objects (Asc + NASM)
#   make link          -> linked higher-half kernel ELF
#   make iso           -> GRUB2 rescue ISO
#   make run           -> boot in QEMU, serial on stdio
#   make run-log       -> boot in QEMU, serial captured to build/serial.log
#   make clean
#
# Toolchain: ascc (ours) · nasm · GNU ld · grub-mkrescue · qemu-system-x86_64

ASCC_REPO := tools/ascc
ASCC      := $(ASCC_REPO)/target/release/ascc
NASM      := nasm
LD        := ld

BUILD     := build
KERNEL_ELF := $(BUILD)/ascension.elf
ISO       := ascension.iso

QEMU_FLAGS  := -serial stdio -display none -no-reboot

.PHONY: all ascc kernel link iso run run-log clean

all: iso

$(ASCC): tools/ascc/src/*.rs tools/ascc/Cargo.toml
	cargo build --release --manifest-path $(ASCC_REPO)/Cargo.toml

ascc: $(ASCC)

$(BUILD)/boot.o: boot/boot.asm | $(BUILD)
	$(NASM) -f elf64 $< -o $@

$(BUILD)/kernel.o: kernel/core/kernel.asc $(ASCC) | $(BUILD)
	$(ASCC) --kernel $< -o $@

$(BUILD):
	mkdir -p $(BUILD)

kernel: $(BUILD)/boot.o $(BUILD)/kernel.o

link: $(KERNEL_ELF)

$(KERNEL_ELF): $(BUILD)/boot.o $(BUILD)/kernel.o linker.ld
	$(LD) -nostdlib -static -z noexecstack \
	    -T linker.ld $(BUILD)/boot.o $(BUILD)/kernel.o -o $@

iso: $(ISO)

$(ISO): $(KERNEL_ELF) boot/grub.cfg | $(BUILD)
	mkdir -p $(BUILD)/iso/boot/grub
	cp $(KERNEL_ELF) $(BUILD)/iso/boot/ascension.elf
	cp boot/grub.cfg $(BUILD)/iso/boot/grub/grub.cfg
	grub-mkrescue -o $@ $(BUILD)/iso 2>/dev/null

run: $(ISO)
	qemu-system-x86_64 -cdrom $(ISO) $(QEMU_FLAGS)

run-log: $(ISO)
	qemu-system-x86_64 -cdrom $(ISO) $(QEMU_FLAGS) -serial file:$(BUILD)/serial.log &

clean:
	rm -rf $(BUILD) $(ISO)

.PHONY: FORCE
FORCE:
