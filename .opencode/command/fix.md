---
description: Repară un bug Ascension — reproducere în QEMU, diagnoză cu serial/monitor QEMU, fix minim, verificare prin boot.
---

Repară bug-ul: $ARGUMENTS

1. **Reproduce în QEMU** (fundal, serial capturat). Triple fault / reboot loop → `qemu -d int,cpu_reset` sau monitorul QEMU (`info registers`) + progres markers. Nu poți reproduce → raportează.
2. **Cauza rădăcină.** Suspecți clasici: paddr/vaddr amestecate (ascc trebuie să le respingă), spinlock_irq lipsă, EOI uitat, stack nealiniat, off-by-one în bitmap/page tables, `ret` în loc de `iretq`, constraint-e inline-asm greșite. Comportament hardware incert → osdev wiki. `git log` pe fișier dacă pare regresie.
3. **Fix minim** conform CLAUDE.md (constante named, nu magic hex; conversii Paddr/Vaddr doar prin funcțiile dedicate).
4. **Verifică:** `make` + boot QEMU + scenariul care crăpa acum trece, cu dovada pe serial.
5. Raport: cauză → fix → dovada. Omoară QEMU la final.
