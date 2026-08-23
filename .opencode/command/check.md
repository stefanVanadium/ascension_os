---
description: Verificare Ascension — make curat + boot QEMU cu serial verificat.
---

Rulează verificările:

```bash
make                     # build complet, din curat
timeout 30 qemu-system-x86_64 -cdrom ascension.iso -serial file:/tmp/serial-check.log -display none -no-reboot > /dev/null 2>&1 &
sleep 25
tail -50 /tmp/serial-check.log
```

Raport compact: make PASS/FAIL, boot PASS/FAIL, ultimele mesaje din serial (panic? subsisteme inițializate? banner-ul așteptat apare exact o dată?). Omoară QEMU la final. Nu repara nimic decât dacă $ARGUMENTS conține „fix".
