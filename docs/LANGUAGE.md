# The Asc Language — Specification v1

> Status: **v1** — extends v0 with multi-unit linking, arrays, address-of,
> pointer field access, magic source constants, and the size-first kernel profile.
> Scope discipline: capabilities, channels, regions/arenas and modules are
> **specified here but not implemented yet** — their semantics below are the
> design contract.

Asc is a systems language for Ascension: capability-based, channel-typed,
ownership-checked, no GC, no runtime. Everything here is checked by `ascc` before
codegen; none of it survives into LLVM IR.

---

## 1. Compilation model

- Source files: `*.asc`. One file = one compilation unit = one object file.
- `ascc input.asc --kernel -o out.o` compiles a **kernel-mode unit**:
  - freestanding: no hosted-OS assumptions, no implicit runtime, no implicit heap
  - no FPU/SSE/MMX target features enabled — no floating point instructions emitted
  - no red zone on any function
  - size-first optimization: `optsize` on every function + `default<O2>` pipeline
    with loop/SLP vectorization disabled and MergeFunctions enabled (`-Os` parity;
    LLVM 19's C API cannot set SizeLevel directly — revisit on LLVM 20+)
  - large/kernel code model (higher-half linking at `0xFFFFFFFF80100000`)
- Without `--kernel`, defaults are still static/freestanding; only the target-feature
  restrictions differ. There is no hosted stdlib in any mode.
- Calling convention: System V AMD64. No name mangling in v0/v1 (`fn kmain` emits
  symbol `kmain` verbatim).
- **Cross-unit linking**: a bodyless function declaration (`fn f(...) -> T;`) is an
  external symbol reference resolved by the linker. Signatures are NOT verified across
  units (the linker sees only symbol names) — declarations are trusted, same trust
  model as C headers. Nominal types that cross units must be redeclared identically
  in each unit; they erase to identical LLVM types, so the linked program agrees.
- Inline asm templates: literal AT&T immediates need `$$` (`$N` is operand
  substitution); this build mis-substitutes `$N` operands inside memory references,
  so asm uses literal registers (`%eax`) with explicit pinning constraints (`{eax}`).
  Clobbers are braced in the constraint string (`~{memory}`, `~{cc}`).

## 2. Lexical structure

- Comments: `//` to end of line. No block comments in v0.
- Identifiers: `[A-Za-z_][A-Za-z0-9_]*`.
- Integer literals: decimal (`123`), hex (`0x3F8`). Type inference from context;
  unsuffixed literals default to `u64` and are implicitly narrowed to any integer
  type when in range (literals are the one implicit numeric conversion).
- Character literals: `'A'`, `'\n'`, `'\\'`, `'\''`, `'\0'` — typed `u8`.
- String literals: `"...\n"` with escapes `\n \t \r \\ \" \' \0`. A string literal
  expression has type `*const u8` and denotes an anonymous NUL-terminated constant
  in `.rodata`.
- Keywords (v0/v1): `fn let const return if else while true false type distinct
  struct asm volatile packed as never bool u8 u16 u32 u64 i8 i16 i32 i64`.
- Magic constants: `__FILE__` expands to the source path as given on the command
  line (`*const u8` string literal); `__LINE__` to the current line (`u64`).
- Pointer qualifiers: `const` and `volatile` after `*` (`*const u8`,
  `*volatile u16`); `const` is documentation-only in v0.

## 3. Types

### Primitives
```
u8 u16 u32 u64   — unsigned integers
i8 i16 i32 i64   — signed integers
bool             — true / false
void             — only valid as a function return type (no value)
never            — function never returns (kmain, panic); any expression may follow it
```

### Distinct nominal types
```
type Paddr = distinct u64    // new nominal type; NOT implicitly convertible
type Vaddr = distinct u64    // Paddr and Vaddr are unrelated types
```
- A distinct type is constructed explicitly: `Paddr(expr_of_u64)`.
- It is unwrapped explicitly: `u64(p)` — allowed only back to its exact base type.
- No arithmetic, comparison, or assignment between distinct types or between a
  distinct type and its base without these explicit conversions. Violation =
  hard compile error.
- Distinct types are Copy (see §7).

### Pointers
```
*T            // raw pointer to T
*volatile T   // accesses through it compile to volatile loads/stores
```
- Pointer indexing `p[i]` is element load/store at offset `i * sizeof(T)`;
  index must be an integer type.
- Field access through a pointer-to-struct auto-derefs (Go-style): `p.field`
  loads/stores through the pointer. No `->` operator. On a struct value,
  `.field` behaves as before.
- `&expr` takes the address of an lvalue (local, array element, struct field):
  result is `*mut T`. Address-of a rvalue or string literal is a hard error.
- `volatile` is part of the type but compatible in both directions on assignment
  and casts (mirrors C semantics; volatility affects access generation, not identity).

### Arrays `[N]T`
```
let buf: [16]u8;              // fixed-size value array, N = const-evaluable int
buf[i] = 'x';                 // unchecked indexing (C-style discipline), lvalue
let p: *const u8 = buf;       // implicit decay to pointer to first element
let q: *const u8 = buf as *const u8;   // explicit decay cast
const N: u64 = 16;
let buf2: [N]u8;              // length may be a named const
```
- Value semantics: arrays are Copy; assignment copies.
- Decay: where a `*T`/`*const T` is expected (arguments, assignments, returns),
  a `[N]T` decays to a pointer to element 0 — codegen emits the address, not a load.
- v1 restrictions: no array-of-array nesting; arrays cannot be parameter or return
  types (pass pointers); bounds are NOT checked (kernel discipline + reentrancy-critical
  paths like panic must not fault-check).

### Structs
```
struct TaskControlBlock {
    rsp: u64,
    id: u32,
}

#[packed]
struct GDTPointer {          // no padding — hardware structs are always packed
    limit: u16,
    base: u64,
}
```
- Field access via `.`. Structs are Copy in v0.
- Struct types are declared per-file (nominal); two structs with identical fields
  are different types.

### Named constants (module level)
```
const COM1: u16 = 0x3F8;
const HELLO: *const u8 = "Ascension: hello from Asc\n";
```
- Must be initialized with a compile-time-evaluable expression (literals,
  const-to-const references, integer ops of those). No memory is allocated for
  scalar consts; string-literal consts live in `.rodata`.

## 4. Functions

```
fn add(a: u64, b: u64) -> u64 {
    return a + b;
}

fn kpanic(src: *const u8, line: u64, msg: *const u8) -> never;   // extern declaration

fn kmain() -> never {        // never returns: halt loop at the end
    ...
}
```
- Parameters and return type required (except `void` functions may omit `return`).
- A bodyless declaration ending in `;` is an **extern declaration**: no definition
  is emitted; the linker resolves the symbol against another unit (or fails).
  A prototype and a definition of the same name must agree exactly; two definitions
  or two prototypes are hard errors.
- Recursion is allowed. There are no variadics, no default arguments, no method
  syntax in v0/v1.
- A `-> never` function's body must not return. Callers may use a call to a never
  function in statement position freely; control flow after it is unreachable.
- Arrays cannot be parameter or return types — pass pointers.

## 5. Statements and expressions

```
let x: u64 = 10;             // annotated
let y = x + 1;               // inferred
let msg: *volatile u16 = phys_to_virt(VGA_TEXT_PADDR) as *volatile u16;
x = y;                       // assignment (all v0 types are Copy)

if x > 10 { } else { }       // braces mandatory, no truthiness — condition is bool
while running { }            // braces mandatory

return;                      // void functions
return expr;                 // typed functions
```

Operators (precedence low→high):
1. `=` (assignment, right-assoc, statement-level)
2. `||`
3. `&&`
4. `==` `!=`
5. `<` `>` `<=` `>=`
6. `+` `-`
7. `*` `/` `%`
8. unary `-` `!`
9. postfix: call `()`, index `[]`, field `.`, cast `as`

- Arithmetic/bitwise ops require both operands to be the **same** integer primitive
  type (distinct types must be unwrapped first). Result type = operand type.
- Comparisons yield `bool`. Conditions must be `bool`.
- Casts: `expr as T` (pointer↔pointer always OK; integer↔integer explicit OK;
  integer↔pointer explicit OK). Also function-call style for distinct types only:
  `Vaddr(u64(p))`.

## 6. Inline assembly

Escape hatch for single instructions; everything else stays Asc.

```
fn outb(port: u16, val: u8) {
    asm { "outb %0, %1" :: "a"(val), "Nd"(port) }
}

fn inb(port: u16) -> u8 {
    let ret: u8;
    asm { "inb %1, %0" : "=a"(ret) : "Nd"(port) }
    ret
}
```

Grammar: `asm { TEMPLATE (: OUTPUTS)? (: INPUTS)? (: CLOBBERS)? }` where
OUTPUTS/INPUTS are comma-separated `"constraint"(binding)` pairs, TEMPLATE is a
string literal passed verbatim (AT&T dialect). Constraints follow LLVM/GCC inline
asm conventions. Volatile-by-default (side effects must not be optimized away).
The asm block is a statement; outputs bind to pre-declared locals.

## 7. Ownership & moves (design contract — minimal checker lands Wave 1)

- Every value is either `Copy` or owned-and-movable. v0 primitives, pointers,
  distinct types, and structs are `Copy`.
- Minimal rule implemented in v0: a moved binding may not be used afterwards.
  With all-v0 types being Copy this never fires yet — the checking infrastructure
  exists so Phase 1 types (buffers, channels, capabilities) plug in without
  redesign.
- Future (not in v0): move-on-send across `chan<T>` invalidates the sender's
  mapping (enforced with kernel cooperation), no aliased mutation.

## 8. Capabilities (design contract — NOT implemented in v0)

```
Capability<T>            // unforgeable token type; no constructor callable from user code
fn read_sensor(cap: Capability<SensorRead>) -> Data
```
- The type checker rejects any call requiring a capability the caller cannot
  produce from its own parameters/locals/globals-of-known-provenance.
- No ambient authority: a resource not named by some `Capability<T>` in scope is
  unreachable, unnameable, unaskable-for.
- Erasure: where the checker can prove a capability statically (compile-time
  proof), it erases to zero bytes. Runtime table entries exist only for
  revocable capabilities (kernel policy decision, cost measured, never assumed free).

## 9. Channels (design contract — NOT implemented in v0)

```
chan<Message>            // language primitive, statically payload-typed
send(ch, msg);           // keyword-like builtins, checked against chan's payload type
let m = recv(ch);
```
- A channel can only carry its declared message type; mismatch = compile error.
- Synchronous semantics at the kernel level; ownership transfer on send for
  non-Copy payloads.

## 10. Error policy

Any violation of this spec — type mismatches, distinct-type confusion, missing
capabilities (once implemented), bad asm constraints — is a **hard compile error**
with file:line:col. Warnings are reserved for style; correctness rules never
downgrade to warnings.

## 11. Out of scope (explicit backlog order)

generics beyond `Capability<T>`/`chan<T>` · traits/interfaces · enums + match ·
slices/dynamic arrays · modules & imports (replaces textual type redeclaration) ·
mutable module-level statics (deliberate: first real consumer is scheduler state,
must be capability-gated per CLAUDE.md rule 5) · closures · const generics ·
regions & arena syntax · WCET annotations · self-hosting.

## 12. Test suite

`tools/ascc/tests/` — `pos/*.asc` must compile clean under `--kernel`,
`neg/*.asc` must be rejected with a hard error; `run.sh` runs both plus a
cross-unit link check (`ld -r` + `nm`) and exits nonzero on any surprise.
`// EXPECT-UNDEF: <sym>` comments in positive files assert symbols the object
must leave undefined.
