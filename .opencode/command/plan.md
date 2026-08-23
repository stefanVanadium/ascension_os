---
description: Planning Mode Ascension — plan complet cu dependency order, adrese exacte de memorie și risks. Nu implementează până la 'aprobat'.
---

Intră în Planning Mode pentru: $ARGUMENTS

1. Citește `.github/agents/PLAN.md` — sprint activ? Confirmă înainte de suprascriere.
2. Citește `Ascension.md` + `CLAUDE.md` + `.github/agents/planner.agent.md` + secțiunile relevante din sursele existente.
3. Detalii hardware incerte (layouts, secvențe init, MSRs) → osdev wiki / Intel SDM ACUM, concluziile în plan.
4. Produce planul: Dependency Order (LANG → LIBK → BOOT+ARCH → MM → restul; marchează ce wave-uri au domenii paralelizabile și cine deține headerele partajate), Tasks pe tag-uri (LANG:/LIBK:/BOOT:...), Risks, Memory Layout cu adrese exacte. Nu uita `BLD:` pentru surse noi → Makefile.
5. **Scrie planul în `.github/agents/PLAN.md`**.
6. Termină cu: `Approval Gate: PENDING — reply 'aprobat' to start execution`

Nu scrie cod de producție.
