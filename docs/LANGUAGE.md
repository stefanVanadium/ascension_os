# The Asc Language — Specification v0

> Status: **v0 draft** — this document is the source of truth for what `ascc` v0 accepts.
> Scope discipline: v0 is the minimum surface needed to compile a freestanding x86_64
> program that boots and talks on serial (Phase 0 proof-of-life). Capabilities,
> channels, regions/arenas are **specified here but not implemented in v0** — they are
> Phase 1+ compiler work, and their semantics below are the design contract.

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
  - size-first optimization profile (optimize for code size)
  - large/kernel code model (higher-half linking at `0xFFFFFFFF80100000`)
- Without `--kernel`, defaults are still static/freestanding; only the target-feature
  restrictions differ. There is no hosted stdlib in any mode.
- Calling convention: System V AMD64. No name mangling in v0 (`fn kmain` emits symbol
  `kmain` verbatim).

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
- Keywords (v0): `fn let const return if else while true false type distinct
  struct asm volatile packed as never bool u8 u16 u32 u64 i8 i16 i32 i64`.
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
- `volatile` is part of the type but compatible in both directions on assignment
  and casts (mirrors C semantics; volatility affects access generation, not identity).

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

fn kmain() -> never {        // never returns: halt loop at the end
    ...
}
```
- Parameters and return type required (except `void` functions may omit `return`).
- Recursion is allowed. There are no variadics, no default arguments, no method
  syntax in v0.
- A `-> never` function's body must not return. Callers may use a call to a never
  function in statement position freely; control flow after it is unreachable.

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

## 11. Out of scope for v0 (explicit backlog order)

generics beyond `Capability<T>`/`chan<T>` · traits/interfaces · enums + match ·
arrays/slices as first-class types · closures · modules/imports · const generics ·
regions & arena syntax · WCET annotations · self-hosting.
