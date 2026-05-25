#!/bin/bash
# braim_cross_source_verification_test.sh
#
# Regression test for cross-source verification primitives:
#   - source as first-class node_type
#   - contradicts edge between statements
#   - contested verification state
#
# Verifies the contract in BRAIM_CROSS_SOURCE_VERIFICATION_SPEC.md §5.
#
# Usage:
#   ./braim_cross_source_verification_test.sh
#   BRAIM=/path/to/braim ./braim_cross_source_verification_test.sh
#
# Exit: 0 if all pass, 1 if any fail. Runs in temp dir.

set -uo pipefail

BRAIM="${BRAIM:-braim}"
TMP_PARENT="$(mktemp -d)"
DIR="$TMP_PARENT/braim_test_data"
mkdir -p "$DIR"
trap 'rm -rf "$TMP_PARENT"' EXIT

PASS=0
FAIL=0
FAILURES=()
NOT_IMPL=0

read_field() {
    python3 -c "
import json
with open('$DIR/current.json') as f:
    d = json.load(f)
n = d.get('nodes', {}).get('$1')
print(n.get('$2', 'MISSING') if n else 'MISSING')
"
}

count_nodes_of_type() {
    python3 -c "
import json
with open('$DIR/current.json') as f:
    d = json.load(f)
print(sum(1 for n in d['nodes'].values() if n.get('node_type') == '$1'))
"
}

skip_or_assert() {
    # If the command being tested errors with 'unknown subcommand' or similar,
    # mark as NOT IMPLEMENTED rather than fail.
    local case_name="$1"; shift
    local expected="$1"; shift
    local actual="$1"
    if [[ "$actual" == "MISSING" || "$actual" == "NOT_IMPL" ]]; then
        echo "  ⚠ $case_name → NOT IMPLEMENTED (expected per spec)"
        NOT_IMPL=$((NOT_IMPL+1))
        return
    fi
    if [[ "$actual" == "$expected" ]]; then
        echo "  ✓ $case_name → $actual"
        PASS=$((PASS+1))
    else
        echo "  ✗ $case_name → expected=$expected actual=$actual"
        FAIL=$((FAIL+1))
        FAILURES+=("$case_name: expected=$expected actual=$actual")
    fi
}

cli_supports() {
    local subcmd="$1"
    "$BRAIM" --data-dir "$DIR" $subcmd --help >/dev/null 2>&1
}

# --- Setup: bootstrap concepts ---

echo "== Setup =="
mk_concept() {
    "$BRAIM" --data-dir "$DIR" concept add "$1" \
        --domains "bootstrap" --sources "code:bootstrap.rs" 2>&1 \
        | grep -oP 'ID:\K[0-9]+' | head -1
}

C1=$(mk_concept "Refund")
C2=$(mk_concept "Payment")
C3=$(mk_concept "Card")
echo "  Concepts: Refund=$C1 Payment=$C2 Card=$C3"

# --- §5.1 Source entity tests ---

echo
echo "================================================================"
echo "SOURCE ENTITY TESTS (§5.1)"
echo "================================================================"

echo
echo "-- S1: braim source add creates source node_type --"
if cli_supports "source"; then
    SRC1=$("$BRAIM" --data-dir "$DIR" source add "Refund design doc 3.2" \
        --type doc --location "doc:billing_design.md:3.2" 2>&1 \
        | grep -oP 'ID:\K[0-9]+' | head -1)
    if [[ -n "$SRC1" ]]; then
        actual=$(read_field "$SRC1" "node_type")
        skip_or_assert "S1_source_node_type" "source" "$actual"
        st=$(read_field "$SRC1" "source_type")
        skip_or_assert "S1_source_type_field" "doc" "$st"
    else
        skip_or_assert "S1_source_node_type" "source" "MISSING"
    fi
else
    echo "  ⚠ S1: 'braim source' subcommand not implemented"
    NOT_IMPL=$((NOT_IMPL+1))
fi

echo
echo "-- S2: two statements with same legacy --sources share one source entity --"
S2A=$("$BRAIM" --data-dir "$DIR" statement add "stmt A about Refund" \
    --domains "test" --sources "doc:shared_source.md:1" \
    --depends "$C1:1.0" 2>&1 | grep -oP 'ID:\K[0-9]+' | head -1)
S2B=$("$BRAIM" --data-dir "$DIR" statement add "stmt B about Refund" \
    --domains "test" --sources "doc:shared_source.md:1" \
    --depends "$C1:1.0" 2>&1 | grep -oP 'ID:\K[0-9]+' | head -1)
# Expected: ONE source entity, referenced by both
SOURCE_COUNT=$(count_nodes_of_type "source")
if [[ "$SOURCE_COUNT" == "0" ]]; then
    echo "  ⚠ S2: source node_type not yet exists (0 source nodes); cannot test dedup"
    NOT_IMPL=$((NOT_IMPL+1))
else
    skip_or_assert "S2_source_deduplication" "1" "$SOURCE_COUNT"
fi

# --- §5.2 Contradicts edge tests ---

echo
echo "================================================================"
echo "CONTRADICTS EDGE TESTS (§5.2)"
echo "================================================================"

# Setup: two statements that disagree about the same subject
STMT_A=$("$BRAIM" --data-dir "$DIR" statement add "Refund processed within 24 hours per spec" \
    --domains "test,test" --sources "doc:spec_v1.md:5,code:refund.rs:10" \
    --depends "$C1:0.5,$C2:0.5" 2>&1 | grep -oP 'ID:\K[0-9]+' | head -1)
STMT_B=$("$BRAIM" --data-dir "$DIR" statement add "Refund processed within 48 hours per spec" \
    --domains "test,test" --sources "doc:spec_v2.md:5,code:refund_v2.rs:10" \
    --depends "$C1:0.5,$C2:0.5" 2>&1 | grep -oP 'ID:\K[0-9]+' | head -1)
echo "  Contradicting pair: STMT_A=$STMT_A (24h) vs STMT_B=$STMT_B (48h)"

echo
echo "-- C1: braim statement contradict creates edge, both become contested --"
if "$BRAIM" --data-dir "$DIR" statement contradict --help >/dev/null 2>&1; then
    "$BRAIM" --data-dir "$DIR" statement contradict "$STMT_A" "$STMT_B" \
        --reason "spec_v1 says 24h, spec_v2 says 48h" 2>&1 >/dev/null || true
    actual_a=$(read_field "$STMT_A" "verification_status")
    actual_b=$(read_field "$STMT_B" "verification_status")
    skip_or_assert "C1_STMT_A_contested" "contested" "$actual_a"
    skip_or_assert "C1_STMT_B_contested" "contested" "$actual_b"
else
    echo "  ⚠ C1: 'braim statement contradict' not implemented"
    NOT_IMPL=$((NOT_IMPL+1))
fi

echo
echo "-- C2: default query hides contested statements --"
OUT=$("$BRAIM" --data-dir "$DIR" query "Refund" 2>&1)
if ! grep -qE "ID:$STMT_A\b" <<< "$OUT" && ! grep -qE "ID:$STMT_B\b" <<< "$OUT"; then
    if [[ "$(read_field "$STMT_A" "verification_status")" == "contested" ]]; then
        echo "  ✓ C2_default_hides_contested → both hidden"
        PASS=$((PASS+1))
    else
        echo "  ⚠ C2: cannot test — contested state not yet set on STMT_A"
        NOT_IMPL=$((NOT_IMPL+1))
    fi
else
    echo "  ✗ C2_default_hides_contested → contested statement(s) returned in default query"
    FAIL=$((FAIL+1))
    FAILURES+=("C2: contested in default query output")
fi

echo
echo "-- C3: --include-contested flag surfaces them --"
OUT=$("$BRAIM" --data-dir "$DIR" query "Refund" --include-contested 2>&1 || true)
if grep -qE "unknown|unrecognized" <<< "$OUT"; then
    echo "  ⚠ C3: --include-contested flag not implemented"
    NOT_IMPL=$((NOT_IMPL+1))
elif grep -qE "ID:$STMT_A\b|ID:$STMT_B\b" <<< "$OUT"; then
    echo "  ✓ C3_include_contested_surfaces_them → at least one returned"
    PASS=$((PASS+1))
else
    echo "  ✗ C3_include_contested_surfaces_them → no contested statements returned"
    FAIL=$((FAIL+1))
    FAILURES+=("C3: --include-contested didn't surface them")
fi

# --- §5.3 Contested state behavior tests ---

echo
echo "================================================================"
echo "CONTESTED STATE TESTS (§5.3)"
echo "================================================================"

echo
echo "-- ST1: third-source auto-resolution (Mechanism A) via statement add-source --"
STATUS_A=$(read_field "$STMT_A" "verification_status")
if [[ "$STATUS_A" != "contested" ]]; then
    echo "  ⚠ ST1: cannot test — STMT_A status=$STATUS_A (contested state not implemented)"
    NOT_IMPL=$((NOT_IMPL+1))
elif ! "$BRAIM" --data-dir "$DIR" statement add-source --help >/dev/null 2>&1; then
    echo "  ⚠ ST1: 'braim statement add-source' not implemented"
    NOT_IMPL=$((NOT_IMPL+1))
else
    # Create an independent contested pair (separate from STMT_A/STMT_B so ST2 is unaffected)
    STMT_C=$("$BRAIM" --data-dir "$DIR" statement add "Refund timeout is 30 days" \
        --domains "test" --sources "doc:policy_v1.md:1" \
        --depends "$C1:1.0" 2>&1 | grep -oP 'ID:\K[0-9]+' | head -1)
    STMT_D=$("$BRAIM" --data-dir "$DIR" statement add "Refund timeout is 60 days" \
        --domains "test" --sources "doc:policy_v2.md:1" \
        --depends "$C1:1.0" 2>&1 | grep -oP 'ID:\K[0-9]+' | head -1)
    "$BRAIM" --data-dir "$DIR" statement contradict "$STMT_C" "$STMT_D" \
        --reason "policy_v1 says 30 days, policy_v2 says 60 days" 2>&1 >/dev/null

    # Create a PRIMARY source and attach only to STMT_C → auto-resolve fires
    THIRD_SRC=$("$BRAIM" --data-dir "$DIR" source add "Authoritative policy transcript" \
        --type transcript --location "transcript:policy_call.txt:15" 2>&1 \
        | grep -oP 'ID:\K[0-9]+' | head -1)
    "$BRAIM" --data-dir "$DIR" statement add-source "$STMT_C" --source-id "$THIRD_SRC" 2>&1 >/dev/null

    after_c=$(read_field "$STMT_C" "verification_status")
    after_d=$(read_field "$STMT_D" "verification_status")
    if [[ "$after_c" != "contested" && "$after_d" == "invalid" ]]; then
        echo "  ✓ ST1_auto_resolution → STMT_C=$after_c STMT_D=$after_d"
        PASS=$((PASS+1))
    else
        echo "  ✗ ST1_auto_resolution → STMT_C=$after_c STMT_D=$after_d (expected non-contested + invalid)"
        FAIL=$((FAIL+1))
        FAILURES+=("ST1: STMT_C=$after_c STMT_D=$after_d")
    fi
fi

echo
echo "-- ST2: explicit resolve-contradiction sets winner/loser --"
if "$BRAIM" --data-dir "$DIR" statement resolve-contradiction --help >/dev/null 2>&1; then
    "$BRAIM" --data-dir "$DIR" statement resolve-contradiction "$STMT_A" "$STMT_B" \
        --winner "$STMT_A" --reason "spec_v2 was a draft, spec_v1 is authoritative" 2>&1 >/dev/null || true
    after_a=$(read_field "$STMT_A" "verification_status")
    after_b=$(read_field "$STMT_B" "verification_status")
    # Winner should be NOT contested (recomputed); loser should be invalid
    if [[ "$after_a" != "contested" && "$after_b" == "invalid" ]]; then
        echo "  ✓ ST2_explicit_resolve → STMT_A=$after_a STMT_B=$after_b"
        PASS=$((PASS+1))
    else
        echo "  ✗ ST2_explicit_resolve → STMT_A=$after_a STMT_B=$after_b (expected non-contested + invalid)"
        FAIL=$((FAIL+1))
        FAILURES+=("ST2: STMT_A=$after_a STMT_B=$after_b")
    fi
else
    echo "  ⚠ ST2: 'braim statement resolve-contradiction' not implemented"
    NOT_IMPL=$((NOT_IMPL+1))
fi

# --- §5.4 CLI surface tests ---

echo
echo "================================================================"
echo "CLI SURFACE TESTS (§5.4)"
echo "================================================================"

echo
echo "-- H1: braim --help mentions REQUIRED RULE 10 and CONTRADICTION RESOLUTION --"
HELP=$("$BRAIM" --help 2>&1)
if grep -qE "^10\." <<< "$HELP" && grep -qE "CONTRADICTION RESOLUTION" <<< "$HELP"; then
    echo "  ✓ H1_help_updated → rule 10 + section present"
    PASS=$((PASS+1))
else
    echo "  ⚠ H1: --help updates not shipped (rule 10 or CONTRADICTION RESOLUTION section missing)"
    NOT_IMPL=$((NOT_IMPL+1))
fi

echo
echo "-- H2: braim statement --help lists contradict and resolve-contradiction --"
STMT_HELP=$("$BRAIM" --data-dir "$DIR" statement --help 2>&1)
if grep -qE "contradict\b" <<< "$STMT_HELP" && grep -qE "resolve-contradiction" <<< "$STMT_HELP"; then
    echo "  ✓ H2_statement_help_lists_new_subcmds"
    PASS=$((PASS+1))
else
    echo "  ⚠ H2: contradict/resolve-contradiction subcommands not listed in statement --help"
    NOT_IMPL=$((NOT_IMPL+1))
fi

echo
echo "-- H3: braim query --help lists --include-contested flag --"
QUERY_HELP=$("$BRAIM" --data-dir "$DIR" query --help 2>&1)
if grep -qE -- "--include-contested" <<< "$QUERY_HELP"; then
    echo "  ✓ H3_query_help_lists_include_contested"
    PASS=$((PASS+1))
else
    echo "  ⚠ H3: --include-contested flag not listed in query --help"
    NOT_IMPL=$((NOT_IMPL+1))
fi

# --- Report ---

echo
echo "================================================================"
echo "RESULTS: $PASS passed, $FAIL failed, $NOT_IMPL not implemented (expected)"
echo "================================================================"

if [[ $FAIL -gt 0 ]]; then
    echo
    echo "Failed cases:"
    for f in "${FAILURES[@]}"; do
        echo "  - $f"
    done
    echo
    echo "Spec reference: BRAIM_CROSS_SOURCE_VERIFICATION_SPEC.md §3 and §5"
    exit 1
fi

if [[ $NOT_IMPL -gt 0 && $PASS -eq 0 ]]; then
    echo
    echo "Spec is not yet implemented in this braim build."
    echo "Re-run after maintainer ships per implementation order in spec §7."
    exit 1
fi

echo
echo "All implemented behaviors pass; remaining gaps marked NOT IMPLEMENTED per spec §6."
exit 0
