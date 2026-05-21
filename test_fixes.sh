#!/usr/bin/env bash
# test_fixes.sh — runnable validation for the five fixes in FIX_SPEC.md.
# Each test runs braim against an isolated graph, asserts the expected
# behavior, and prints PASS or FAIL.
#
# Run BEFORE landing the fix to confirm the bug reproduces (expected: FAIL).
# Run AFTER landing the fix to confirm the fix holds (expected: PASS).
#
# Usage:
#   ./test_fixes.sh                 # run all tests
#   ./test_fixes.sh t1_dup_sources_warn
#   STRICT=1 ./test_fixes.sh        # also exercise --strict-* paths

set -u  # do not set -e: each test catches its own failures

readonly TESTDIR="$(mktemp -d -t braim-fix-XXXXXX)"
trap 'rm -rf "$TESTDIR"' EXIT

readonly PASS_LABEL="\033[32mPASS\033[0m"
readonly FAIL_LABEL="\033[31mFAIL\033[0m"
readonly SKIP_LABEL="\033[33mSKIP\033[0m"

passed=0
failed=0
skipped=0

# helper: fresh graph per test
new_graph() {
  local name="$1"
  local dir="$TESTDIR/$name"
  mkdir -p "$dir"
  echo "$dir"
}

# helper: extract verdict, log, return 0/1
report() {
  local name="$1" verdict="$2" detail="${3:-}"
  case "$verdict" in
    PASS) echo -e "  $PASS_LABEL $name${detail:+ -- $detail}"; ((passed++)) ;;
    FAIL) echo -e "  $FAIL_LABEL $name${detail:+ -- $detail}"; ((failed++)) ;;
    SKIP) echo -e "  $SKIP_LABEL $name${detail:+ -- $detail}"; ((skipped++)) ;;
  esac
}

# helper: capture stderr only
stderr_of() {
  "$@" 2>&1 >/dev/null
}

# helper: capture stdout+stderr
output_of() {
  "$@" 2>&1
}

# ──────────────────────────────────────────────────────────────────
# Test 1 — duplicate source entries
# Expectation:
#   default mode: braim writes the statement AND emits a warning to stderr
#                 about duplicate source entries
#   strict mode (STRICT=1): braim refuses the write with non-zero exit
# ──────────────────────────────────────────────────────────────────

t1_dup_sources_warn() {
  local dir
  dir="$(new_graph t1)"
  pushd "$dir" >/dev/null

  # bootstrap two atomics so a 2-dep statement is legal
  braim concept add "A" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "B" --domains x --sources "doc:c:2" >/dev/null 2>&1

  local out
  out="$(output_of braim statement add "rule" \
            --domains a,b \
            --sources "doc:c:1,doc:c:1" \
            --depends "1:0.5,2:0.5")"

  if [[ "$out" =~ duplicate.*source ]] || [[ "$out" =~ "doc:c:1" ]]; then
    report "t1_dup_sources_warn" PASS "warning emitted"
  else
    report "t1_dup_sources_warn" FAIL "no duplicate-source warning in: $out"
  fi
  popd >/dev/null
}

t1_dup_sources_strict() {
  if [[ -z "${STRICT:-}" ]]; then
    report "t1_dup_sources_strict" SKIP "set STRICT=1 to run"
    return
  fi
  local dir
  dir="$(new_graph t1s)"
  pushd "$dir" >/dev/null
  braim concept add "A" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "B" --domains x --sources "doc:c:2" >/dev/null 2>&1

  if braim statement add "rule" \
       --domains a,b \
       --sources "doc:c:1,doc:c:1" \
       --depends "1:0.5,2:0.5" \
       --strict-sources >/dev/null 2>&1
  then
    report "t1_dup_sources_strict" FAIL "strict mode accepted duplicate sources"
  else
    report "t1_dup_sources_strict" PASS "strict mode rejected"
  fi
  popd >/dev/null
}

# ──────────────────────────────────────────────────────────────────
# Test 2 — PRIMARY + TERTIARY source mix
# Expectation:
#   default: warning about taxonomy mix on stderr
#   strict: rejected
# ──────────────────────────────────────────────────────────────────

t2_primary_tertiary_mix_warn() {
  local dir
  dir="$(new_graph t2)"
  pushd "$dir" >/dev/null
  braim concept add "A" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "B" --domains x --sources "doc:c:2" >/dev/null 2>&1

  local out
  out="$(output_of braim statement add "inferred" \
            --domains a,b \
            --sources "doc:c:1,inference:derived-from-a" \
            --depends "1:0.5,2:0.5")"

  if [[ "$out" =~ (taxonomy|mix|inference.*doc|doc.*inference|TERTIARY|PRIMARY) ]]; then
    report "t2_primary_tertiary_mix_warn" PASS "taxonomy warning emitted"
  else
    report "t2_primary_tertiary_mix_warn" FAIL "no taxonomy warning in: $out"
  fi
  popd >/dev/null
}

# ──────────────────────────────────────────────────────────────────
# Test 3 — duplicate domain entries
# ──────────────────────────────────────────────────────────────────

t3_dup_domains_warn() {
  local dir
  dir="$(new_graph t3)"
  pushd "$dir" >/dev/null
  braim concept add "A" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "B" --domains x --sources "doc:c:2" >/dev/null 2>&1
  braim concept add "C" --domains x --sources "doc:c:3" >/dev/null 2>&1

  local out
  out="$(output_of braim statement add "rule" \
            --domains library,library,library \
            --sources "doc:c:1,doc:c:2,doc:c:3" \
            --depends "1:0.3,2:0.3,3:0.4")"

  if [[ "$out" =~ (duplicate.*domain|library.*3|domain.*duplicate) ]]; then
    report "t3_dup_domains_warn" PASS "duplicate-domain warning emitted"
  else
    report "t3_dup_domains_warn" FAIL "no warning in: $out"
  fi
  popd >/dev/null
}

t3_dup_domains_strict() {
  if [[ -z "${STRICT:-}" ]]; then
    report "t3_dup_domains_strict" SKIP "set STRICT=1 to run"
    return
  fi
  local dir
  dir="$(new_graph t3s)"
  pushd "$dir" >/dev/null
  braim concept add "A" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "B" --domains x --sources "doc:c:2" >/dev/null 2>&1
  braim concept add "C" --domains x --sources "doc:c:3" >/dev/null 2>&1

  if braim statement add "rule" \
       --domains library,library,library \
       --sources "doc:c:1,doc:c:2,doc:c:3" \
       --depends "1:0.3,2:0.3,3:0.4" \
       --strict-domains >/dev/null 2>&1
  then
    report "t3_dup_domains_strict" FAIL "strict mode accepted dup domains"
  else
    report "t3_dup_domains_strict" PASS "strict mode rejected"
  fi
  popd >/dev/null
}

# ──────────────────────────────────────────────────────────────────
# Test 4 — gap register clears on connect
# Expectation:
#   After braim registers a gap between A and B (via a failed perspective
#   or proximity query), adding a statement that --depends on both A and B
#   removes the (A,B) pair from `braim audit`'s Gap register section.
# ──────────────────────────────────────────────────────────────────

t4_gap_register_clears_on_connect() {
  local dir
  dir="$(new_graph t4)"
  pushd "$dir" >/dev/null
  braim concept add "A" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "B" --domains x --sources "doc:c:2" >/dev/null 2>&1

  # Provoke a gap-register write by asking for a path that doesn't exist.
  braim perspective "A" "B" >/dev/null 2>&1 || true

  # Pre-state check: gap register should now mention A and B.
  local pre_audit
  pre_audit="$(braim audit 2>&1)"
  if ! [[ "$pre_audit" =~ "A".*"B" ]]; then
    report "t4_gap_register_clears_on_connect" SKIP \
           "could not provoke a gap register entry; check braim build"
    popd >/dev/null
    return
  fi

  # Add a connecting statement.
  braim statement add "A and B are related" \
    --domains x,y \
    --sources "doc:c:1,doc:c:2" \
    --depends "1:0.5,2:0.5" >/dev/null 2>&1

  # Post-state check: gap register should no longer mention this pair.
  local post_audit
  post_audit="$(braim audit 2>&1)"
  if [[ "$post_audit" =~ "Gap register"$'\n'.*"none" ]] \
     || ! [[ "$post_audit" =~ "A".*"B" ]]; then
    report "t4_gap_register_clears_on_connect" PASS "gap cleared after connect"
  else
    report "t4_gap_register_clears_on_connect" FAIL \
           "gap still flagged after a connecting statement was added"
  fi
  popd >/dev/null
}

# ──────────────────────────────────────────────────────────────────
# Test 5 — multi-word atomic decomposes into existing atomics → warn
# ──────────────────────────────────────────────────────────────────

t5_multiword_atomic_warns() {
  local dir
  dir="$(new_graph t5)"
  pushd "$dir" >/dev/null
  braim concept add "Library" --domains x --sources "doc:c:1" >/dev/null 2>&1
  braim concept add "Card" --domains x --sources "doc:c:2" >/dev/null 2>&1

  local out
  out="$(output_of braim concept add "Library Card" \
            --domains x \
            --sources "doc:c:3")"

  if [[ "$out" =~ (compound|decompose|existing.*atomic|consider) ]]; then
    report "t5_multiword_atomic_warns" PASS "decomposition hint emitted"
  else
    report "t5_multiword_atomic_warns" FAIL "no hint in: $out"
  fi
  popd >/dev/null
}

# ──────────────────────────────────────────────────────────────────
# Driver
# ──────────────────────────────────────────────────────────────────

ALL_TESTS=(
  t1_dup_sources_warn
  t1_dup_sources_strict
  t2_primary_tertiary_mix_warn
  t3_dup_domains_warn
  t3_dup_domains_strict
  t4_gap_register_clears_on_connect
  t5_multiword_atomic_warns
)

main() {
  command -v braim >/dev/null || { echo "braim not in PATH"; exit 2; }

  echo "braim fix-spec test suite"
  echo "──────────────────────────"
  echo "isolated test dir: $TESTDIR"
  echo

  local targets=("$@")
  if [[ ${#targets[@]} -eq 0 ]]; then
    targets=("${ALL_TESTS[@]}")
  fi

  for t in "${targets[@]}"; do
    if declare -F "$t" >/dev/null; then
      "$t"
    else
      report "$t" FAIL "unknown test"
    fi
  done

  echo
  echo "──────────────────────────"
  echo "PASS: $passed   FAIL: $failed   SKIP: $skipped"
  if [[ $failed -gt 0 ]]; then
    exit 1
  fi
}

main "$@"
