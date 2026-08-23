---
description: Starea sprintului Ascension — plan activ, progres, build/boot verde sau roșu.
---

Raportează starea, compact (max 15 linii):

1. `.github/agents/PLAN.md` — sprint activ? Progres pe domenii? Ce wave urmează?
2. Ultimul checkpoint din `.github/agents/MISSION_CONTROL.md` (o linie).
3. `git status` + `git log --oneline -5`.
4. `make` — verde?

Sprint activ → ce urmează (`/wave`). Fără sprint → gata de `/plan`.
