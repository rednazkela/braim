#!/bin/bash
# Wording lint for model-facing text in hooks/ and policies/.
#
# Purpose: keep extraction-shaped phrasing from being reintroduced into the
# strings that models read. This is a pure grep — no model is ever in the
# loop, no prompt is tuned against any model's judgment. If a model refuses
# a text that PASSES this lint, that refusal is a finding for a human to
# read (the model may be right and the ask may need to change), never a
# phrase list to extend until the refusal stops.
#
# Rationale (session 2026-10-16): wording that demands disclosure of the
# model's reasoning process, or asserts its own authority with compliance
# pressure, gets refused wholesale by 5.5-generation models — taking the
# legitimate receipt request down with it. The fix was rewording to
# deeds-only receipts (commands run, IDs returned). This lint pins that fix.
set -euo pipefail
cd "$(dirname "$0")/.."

# Phrases forbidden in model-facing text. Matched case-insensitively.
# Demands about internal process and coercive-compliance framing only —
# mentioning "reasoning" to DISCLAIM requesting it is fine and not matched.
patterns=(
  'reveal the chain'
  'chain of inferences'
  'reasoning chain'
  'disclose what you'
  'audit your own reasoning'
  'declaration of your reasoning'
  'blocked until you comply'
  'the compliant response'
  'non-compliance'
  'silence is non'
)

fail=0
for pat in "${patterns[@]}"; do
  # .sh files: only string-carrying lines (echo/printf) are model-facing;
  # comments and history notes may cite the old wording. .md policies are
  # model-facing in full.
  hits=$( { grep -niE "^[^#]*\b(echo|printf)\b.*$pat" hooks/*.sh 2>/dev/null; \
            grep -niE "$pat" policies/*.md 2>/dev/null; } || true)
  if [ -n "$hits" ]; then
    echo "wording_lint: forbidden phrase '$pat':" >&2
    echo "$hits" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "wording_lint: FAIL — extraction-shaped or coercive phrasing in model-facing text." >&2
  exit 1
fi
echo "wording_lint: OK"
