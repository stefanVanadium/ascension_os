---
description: Închide sprintul Ascension — make clean build, boot QEMU final, CHANGELOG, checkpoint, golire PLAN, commit.
---

Închide sprintul curent (doar cu toate task-urile `[x]` sau la cererea explicită):

1. **Verificare finală:** `make clean && make` (din curat) + boot QEMU complet cu serial verificat (toate subsistemele inițializate, zero panic). FAIL → stop.
2. **CHANGELOG.md:** entry pe baza `git diff`-ului real.
3. **docs/:** ARCHITECTURE/MEMORY_MAP/SYSCALLS + journal actualizate dacă sprintul le-a atins.
4. **MISSION_CONTROL.md:** UN checkpoint (max 10 linii); >200 linii → arhivează în ARCHIVE/legacy.md.
5. **PLAN.md:** golește-l.
6. **Commit:** selectiv (fără `build/`, `*.iso`, `tools/ascc/target/`), mesaj descriptiv. NU push fără cerere.
7. Output: rezumat 5-10 linii. Omoară QEMU-urile pornite.
