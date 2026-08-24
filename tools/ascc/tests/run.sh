#!/usr/bin/env bash
# ascc test runner: positive cases must compile clean (kernel profile),
# negative cases must FAIL at some stage. Exits nonzero on any surprise.
#
#   ./run.sh              # uses ../target/release/ascc
#   ASCC=/path/ascc ./run.sh
set -u
cd "$(dirname "$0")"
ASCC="${ASCC:-../target/release/ascc}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0; fail=0

for src in pos/*.asc; do
    if "$ASCC" --kernel "$src" -o "$TMP/out.o" >/dev/null 2>"$TMP/err"; then
        # optional extra check: EXPECT-UNDEF lines name symbols the object
        # must leave undefined (extern declarations resolved at link time)
        ok=1
        while IFS= read -r sym; do
            [ -z "$sym" ] && continue
            if ! nm "$TMP/out.o" | grep -q " U $sym\$"; then
                echo "FAIL(pos) $src: expected undefined symbol '$sym', nm says:" >&2
                nm "$TMP/out.o" | grep " U " >&2
                ok=0
            fi
        done < <(grep -h '^// EXPECT-UNDEF:' "$src" | sed 's|^// EXPECT-UNDEF:[[:space:]]*||')
        if [ "$ok" = 1 ]; then pass=$((pass+1)); else fail=$((fail+1)); fi
    else
        echo "FAIL(pos) $src (should compile):" >&2
        sed 's/^/    /' "$TMP/err" >&2
        fail=$((fail+1))
    fi
done

for src in neg/*.asc; do
    if "$ASCC" --kernel "$src" -o "$TMP/out.o" >/dev/null 2>"$TMP/err"; then
        echo "FAIL(neg) $src (should be rejected):" >&2
        fail=$((fail+1))
    else
        pass=$((pass+1))
    fi
done

# cross-unit linking: the extern pair must link into one executable symbol set
if [ -f pos/extern_cross_unit_a.asc ] && [ -f pos/extern_cross_unit_b.asc ]; then
    "$ASCC" --kernel pos/extern_cross_unit_a.asc -o "$TMP/a.o" >/dev/null 2>&1 &&
    "$ASCC" --kernel pos/extern_cross_unit_b.asc -o "$TMP/b.o" >/dev/null 2>&1 &&
    ld -r "$TMP/a.o" "$TMP/b.o" -o "$TMP/ab.o" 2>/dev/null &&
    nm -u "$TMP/ab.o" | grep -q shared_helper && {
        echo "FAIL(link) extern_cross_unit pair did not resolve shared_helper" >&2
        fail=$((fail+1))
    } || true
fi

echo "ascc tests: $pass passed, $fail failed"
[ "$fail" = 0 ]

