---
description: Execută următorul wave din PLAN.md — cu subagenți paraleli când wave-ul are 2+ domenii independente.
---

Execută următorul wave din sprintul activ.

1. Citește `.github/agents/PLAN.md` — primul wave cu `[ ]`.
2. Citește `.github/agents/EXECUTION.md` §2.
3. Un domeniu → direct (cu agent.md-ul lui citit din `.github/agents/`). 2+ domenii independente → lansează subagenți paraleli (tool-ul de task), câte unul per domeniu, toți într-un singur mesaj. Briefing per subagent: task-urile lui + headerele/interfețele fixate + ce fișiere NU atinge (headerele partajate le editează UN singur agent per wave).
4. Doar wave-ul curent. După rapoarte: trust but verify — `git diff --stat` + `make` + boot QEMU; apoi `[x]` în PLAN.md.
5. Output: Completion Report agregat (cu dovezi din serial).

Plan gol / totul `[x]`: `make` + boot QEMU final, checkpoint în MISSION_CONTROL.md, golește PLAN.md, `task terminat`. Omoară QEMU-urile pornite.
Blocat: `task in asteptare: astept <domeniu>` și stop.
