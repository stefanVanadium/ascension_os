# ▲ Ascension — Project Overview

> Acest document are două părți:
> 1. **Discuția inițială** (arhivată mai jos) — punctul de plecare, un OS clasic, "am făcut și eu un kernel". Depășit. Păstrat doar ca istoric/context, nu ca direcție.
> 2. **Viziunea** (după discuții) — direcția reală, asta construim. Nu un kernel-toy, ci sistemul complet: capabilities native, canale tipizate, hardware tagged memory, limbaj propriu — de la silicon până la aplicație.
>
> **Redenumire:** proiectul a pornit ca *IronwoodOS*; numele curent este **Ascension**, iar limbajul propriu se numește **Asc** (extensie `.asc`, compilator `ascc`, shell `embers`). Regulile tehnice de lucru sunt în `CLAUDE.md` / `QWEN.md`; secțiunea arhivată de mai jos păstrează numele vechi ca istorie.

---

## [ARHIVĂ — depășit] Discuție inițială

> Tot ce urmează în această secțiune e conceptul de la care s-a plecat, **înainte** de discuțiile care au dus la viziunea reală de mai jos. Nu mai e ținta proiectului — rămâne doar ca notă istorică.

### Concept initial dar slab

IronwoodOS este un sistem de operare construit de la zero, cu filozofia:
- **Cât mai eficient** — minimalist, rulează și pe hardware vechi
- **Cât mai modular** — componente clare, bine separate
- **Cât mai robust** — nu cedează la erori de drivere
- **Pentru TOTI** — terminal first tho ca totusi e pentru mine, no bloat

### Arhitectură: Hybrid Kernel

```
┌─────────────────────────────────────┐
│           USER SPACE                │
│  Apps │ Shell │ Drivere non-critice │
├─────────────────────────────────────┤
│         KERNEL SPACE (hybrid)       │
│  ┌─────────────────────────────┐    │
│  │      MICROKERNEL CORE       │    │
│  │  - Scheduling               │    │
│  │  - IPC                      │    │
│  │  - Memory (paging)          │    │
│  │  - Interrupts               │    │
│  ├─────────────────────────────┤    │
│  │   IN-KERNEL (performanță)   │    │
│  │  - Graphics driver          │    │
│  │  - Storage driver           │    │
│  │  - Network stack            │    │
│  └─────────────────────────────┘    │
├─────────────────────────────────────┤
│           HARDWARE                  │
└─────────────────────────────────────┘
```

#### Regula de aur
> Dacă un driver crapă și poate corupe memorie kernel → **user space**
> Dacă un driver e pe critical path și latența contează → **kernel space**

### Stack Tehnic (inițial)

| Componentă | Limbaj |
|---|---|
| Bootloader, context switch, GDT/IDT | ASM |
| Kernel core, drivere critice | C |
| Shell, userspace apps (opțional, daca aduce vreun beneficiu) | C++ |

### Structura Proiectului (inițială)

```
ironwood/
├── boot/               # Bootloader + ASM entry point
├── kernel/
│   ├── core/           # Scheduler, IPC, memory manager
│   ├── drivers/        # Drivere critice (in-kernel)
│   └── arch/           # x86-64 specific (GDT, IDT, paging)
├── userspace/
│   ├── shell/          # Terminalul IronwoodOS
│   └── libs/           # libc minimală
├── build/              # Output compilat
└── Makefile
```

### Roadmap (inițial)

| # | Pas | Status |
|---|---|---|
| 1 | Bootloader + "Hello from IronwoodOS" pe ecran | ⬜ |
| 2 | GDT, IDT, interrupts în C | ⬜ |
| 3 | Memory management — paging, heap allocator | ⬜ |
| 4 | Scheduler — procese, multitasking | ⬜ |
| 5 | VFS — sistem de fișiere abstract | ⬜ |
| 6 | Driver tastatură + VGA text mode | ⬜ |
| 7 | Terminal / Shell | ⬜ |
| 8 | Syscall interface | ⬜ |
| 9 | Userspace + libc minimală | ⬜ |

### Target Hardware (inițial)

- **Arhitectură:** x86-64
- **Mediu de test:** QEMU (virtualizat)
- **Obiectiv final:** rulează pe hardware real, inclusiv PC-uri vechi

#### Tools necesare
- `qemu-system-x86_64` — pentru testare
- `x86_64-elf-gcc` — cross-compiler
- `nasm` sau `gas` — assembler
- `make` — build system

### Filozofie Design (inițială)

- **Nu e un Linux clone** — identitate proprie
- **Terminal first** — UI minimalist, pentru programatori
- **Modulare în cod** — chiar dacă kernelul e hybrid, codul e structurat clar pe module
- **Zero bloat** — fiecare linie de cod are un motiv să existe
- **Scriem noi codu, nu importam 10 librari pentru "Hello world"**

---

## VIZIUNE — direcția reală a proiectului

> Asta construim. Nu doar un kernel funcțional, ci sistemul complet, gândit ca un singur organism — de la concept de hardware până la aplicație. Fără compromisuri istorice (Unix pentru mainframe-uri, Windows moștenind DOS). Trei principii: **izolare completă**, **transparență totală**, **compoziționalitate**.
>
> Un al patrulea principiu, moștenit direct din conceptul inițial arhivat mai sus și NEnegociabil indiferent cât de mare a crescut viziunea: **eficiență radicală de memorie și spațiu**. Rulează pe cât mai multe mașini, cât mai eficient — cum lucrau inginerii la început, când fiecare byte și fiecare ciclu trebuia să-și justifice locul. Capabilities și typed channels sunt gratuite dacă se pot verifica static și se pot elimina la compilare; dacă nu se pot elimina, costul lor se măsoară, nu se presupune acceptabil. Detalii concrete (targete de codegen, ce se urmărește în QA) sunt în `CLAUDE.md` → "EFFICIENCY & FOOTPRINT".

### Kernel: microkernel radical, tip seL4/L4

Kernelul propriu-zis ar avea sub 15.000 de linii de cod. Ar face exact patru lucruri:

- **Scheduling** — thread-uri, nimic mai mult
- **Memory management** — address spaces, pagini, nimic altceva
- **IPC (inter-process communication)** — sincron, prin canale tipizate
- **Capabilities** — controlul accesului la orice altă resursă

Tot restul — drivere de disc, stack de rețea, filesystem, chiar și driverele de placă video — rulează ca procese userspace normale, izolate în address space-uri separate. Dacă un driver de rețea crapă, nu-ți ia tot sistemul cu el, doar restartezi acel proces. Asta e diferența majoră față de Linux, unde un bug într-un driver poate corupe kernelul întreg pentru că totul rulează în același spațiu de adrese privilegiat.

Prețul e latența de IPC — un microkernel are overhead mai mare la trecerea mesajelor între procese decât un syscall direct într-un kernel monolitic. Dar seL4 a demonstrat că, cu un design de IPC bine făcut (registre, nu copiere de memorie, batching de mesaje), overhead-ul ăsta poate fi redus la câteva sute de cicli — acceptabil pentru 99% din workload-uri.

### Model de securitate: capabilities, nu permisiuni

Uită de user/group/root și de bit-uri rwx. În schimb, fiecare proces pornește cu un set explicit de **capabilities** — practic niște token-uri neforjabile care dau acces la o resursă anume, cu drepturi anume (read, write, execute, delegate). Un proces nu poate face nimic pentru care nu are un capability explicit, nici măcar să afle că o resursă există.

Practic: browser-ul tău primește capability la un singur socket de rețea și la un director sandbox, punct. Nu poate citi `/home`, nu pentru că i-ai interzis printr-o regulă, ci pentru că fizic nu are cum să adreseze acel fișier — nu există niciun API prin care să ceară acces la ceva ce nu i s-a dat explicit.

Asta elimină o clasă întreagă de vulnerabilități — privilege escalation devine aproape imposibil by design, pentru că nu există "privilegiu ambient" de escaladat.

### Modelul de concurență: actors + typed channels

Kernelul ar expune primitive de concurență la nivel de canale tipizate (gen Go channels sau Erlang mailboxes), nu thread-uri cu shared memory + locks. Locking manual e sursa a jumătate din bug-urile grele din sisteme concurente (race conditions, deadlocks). Dacă procesele comunică *doar* prin mesaje trimise pe canale, iar canalele sunt tipizate static, o grămadă din bug-urile astea dispar la compilare, nu la runtime.

Pentru partea de shared memory (unde ai nevoie de ea, gen buffere mari de date), aș folosi un model de **ownership** inspirat din Rust — memoria transferată printr-un canal își schimbă owner-ul, procesul vechi pierde fizic accesul (unmap), deci nu poți avea două procese care scriu simultan în aceeași pagină din greșeală.

### Filesystem: totul e un obiect tipizat, nu un fișier

Aici m-aș inspira din Plan 9, dar aș merge un pas mai departe. În loc de "totul e un fișier" (un stream de bytes fără structură), aș zice **totul e un obiect cu un schema versionat**. Un proces, o fereastră, o conexiune de rețea, un senzor — toate sunt adresabile printr-un namespace unificat, dar fiecare obiect știe ce tip de date expune (nu doar bytes bruți), iar sistemul poate valida la compile-time sau runtime dacă un consumator "înțelege" schema respectivă.

Namespace-urile ar fi per-proces, ca în Plan 9 — fiecare proces își vede propriul montaj al lumii, poți face sandboxing trivial doar prin manipularea namespace-ului, fără containere grele gen Docker.

### Determinism și timp real ca opțiune de prim rang

Pentru partea ta de embedded/RTOS, aș vrea ca schedulerul să suporte două moduri, comutabile per-task:

- **Best-effort** — scheduler normal, throughput-optimizat
- **Hard real-time** — task-ul primește garanții formale de worst-case execution time, verificate static (analiză WCET la compilare, nu la runtime)

Asta ar face sistemul valid atât pentru un laptop cât și pentru un microcontroller care controlează un motor, fără să ai nevoie de două OS-uri complet diferite (cum e azi Linux vs FreeRTOS).

### Limbaj de sistem: nu C

C e motivul pentru care avem 50 de ani de buffer overflows. Aș scrie totul într-un limbaj cu memory safety la compilare (gen Rust, sau ceva mai simplu, custom-făcut pentru kernel-uri — vezi cum a plecat comunitatea seL4 spre verificare formală în Isabelle/HOL). Corectitudinea kernelului ar trebui *dovedită matematic*, nu doar testată — seL4 e singurul kernel general-purpose cu proof formal de corectitudine end-to-end, și cred că ăsta ar trebui să fie standardul, nu excepția.

### Boot și update: imutabil și atomic

Sistemul de fișiere rădăcină ar fi read-only, tip image-based (gen NixOS sau ChromeOS) — update-urile se aplică atomic, ca o imagine nouă completă, cu rollback instant dacă boot-ul eșuează. Nu mai există stare de "sistem pe jumătate updatat" care să-ți crape mașina la 2 dimineața.

---

Practic, dacă ar trebui să-l numesc: un hibrid între **seL4** (kernel, capabilities, proof formal), **Plan 9** (namespace-uri, "totul e obiect"), **Erlang/BEAM** (concurență prin mesaje, supervizor trees pentru fault tolerance) și **NixOS** (update-uri atomice, imutabilitate).

### Limbaj propriu — de ce C sau Rust nu sunt suficiente

Nu e nebunie — dacă construiești un OS de la zero cu idei proprii de capabilities și namespace-uri, un limbaj generic (C sau Rust) o să te forțeze mereu să "traduci" ideile alea prin abstractizări care nu au fost gândite pentru ele. Un limbaj propriu ar putea exprima nativ conceptele — capability ca prim-cetățean în type system, canale tipizate ca parte din sintaxă, nu bibliotecă adăugată ulterior.

Practic, ce-ar însemna:

- **Capabilities ca tipuri, nu ca valori runtime.** Compilatorul ar ști static ce capabilități are o funcție, la fel cum Rust știe static dacă ai `&mut` sau `&`. Ai putea avea o semnătură de genul `fn read_sensor(cap: Capability<SensorRead>) -> Data` unde compilatorul verifică la compile-time că nu poți chema funcția fără capability-ul potrivit, mai degrabă decât runtime check.
- **Ownership + regiuni, nu garbage collector.** Pentru un kernel n-ai voie GC (latență imprevizibilă), deci ai nevoie de ceva gen Rust borrow-checker, dar simplificat — poate chiar mai strict, cu regiuni de memorie explicite (arena allocation) integrate în sintaxă, nu adăugate ca pattern.
- **Concurrency prin channels, sintactic, nu prin bibliotecă.** `send`/`recv` ca keyword-uri, nu funcții — compilatorul verifică tipurile mesajelor la compile time, deci un canal typed nu poate primi accidental un mesaj greșit.

Partea grea: scrisul unui compilator solid (lexer, parser, type checker, code generator pentru ARM/RISC-V) e un proiect masiv de unul singur, separat de OS. Riscul real e să te împrăștii pe trei fronturi (CPU + OS + limbaj) și să nu termini niciunul.

**Ordinea de execuție:** fără C, niciodată — nici măcar "inițial". Compilatorul vine primul: `ascc` v0 e o unealtă host scrisă în Rust, cu LLVM ca backend de codegen (`tools/ascc/`); Rust-ul nu apare niciodată în OS — e doar unealta de dev, exact cum era un cross-compiler clasic. Design-ul limbajului (sintaxă, type system, semantica capabilities) se fixează pe hârtie în faza asta, iar abia când `ascc` compilează un subset minimal de Asc scriem kernelul, direct în Asc. Așa avem baza solidă *și* claritate pe ce vrem de la limbaj, fără să construim compilatorul în gol.

### Plafonul viziunii — sistemul coerent, de la silicon la aplicație

Nu doar un OS și un limbaj separate — un **sistem coerent**, gândit ca un singur organism, de la siliciu (concept) până la aplicație. Azi ai straturi complet separate care nu "știu" unul de altul — hardware-ul nu știe nimic de type safety, compilatorul nu știe nimic de scheduler, OS-ul nu știe nimic de intențiile programului. Fiecare strat re-verifică sau re-descoperă ce straturile de dedesubt/deasupra deja știau.

Visul: **un singur model de "capabilities" și "types" care traversează tot stack-ul**, de la ISA (instruction set) până la aplicația finală. Practic:

- **Limbajul** exprimă capabilities și ownership nativ în sintaxă
- **Compilatorul** generează cod care păstrează garanțiile astea până la nivel de instrucțiuni mașină — nu doar verifică la compilare și apoi "uită", ci emite metadate hardware-verificabile
- **CPU-ul** (chiar și cel TTL, într-o formă simplificată, ca demonstrație de concept) are suport nativ pentru tagged memory — fiecare cuvânt de memorie poartă un tag mic care spune "asta e un capability, nu poate fi falsificat prin aritmetică pe pointeri". Există precedent — CHERI de la Cambridge face exact asta pe ARM Morello, hardware capabilities la nivel de pointer.
- **OS-ul** nu mai verifică nimic la runtime pentru că garanțiile vin deja din hardware + compilator

Asta ar elimina o clasă întreagă de atacuri (buffer overflow, use-after-free, privilege escalation) nu prin patch-uri software, ci pentru că devin **fizic irepresentabile** în sistem.

**Scalare la sisteme distribuite.** De ce să te oprești la "OS pentru un calculator"? Modelul de capabilities + canale tipizate se scalează natural la sisteme distribuite. Un capability nu trebuie să fie local — poate fi un token care traversează rețeaua, cu aceleași garanții. Practic ai putea avea un sistem unde un proces local și un proces de pe un server la distanță vorbesc prin același model de canale tipizate, cu aceleași garanții de siguranță, fără diferență conceptuală între "local" și "distribuit". E ideea din spatele lui Erlang/BEAM dusă la extrem, combinată cu capabilities.

Și mai departe — un limbaj cu capabilities ca tipuri de prim rang e exact genul de fundație pe care ai vrea-o pentru sisteme unde AI-uri și cod scris de om colaborează direct pe același cod, pentru că poți exprima formal "acest agent AI are exact aceste capabilități, nu mai multe" — verificabil static, nu doar prompt-uri și promisiuni.

Deci, ca viziune completă: **un limbaj cu capabilities native → compilator care propagă garanțiile spre hardware → CPU cu tagged memory → OS minimal care nu mai are nevoie să reinventeze securitatea → model care se scalează de la un microcontroler până la sisteme distribuite multi-agent**. Un singur set de idei, aplicat consecvent la fiecare nivel, în loc de 5 straturi care nu se "văd" unul pe altul.

E genul de proiect de 10 ani, nu de-un semestru. Ăsta e plafonul — și ăsta e ce construim, nu varianta redusă.
