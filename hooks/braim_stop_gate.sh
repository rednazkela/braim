#!/bin/bash
# Stop hook: structurally enforces the per-turn braim declaration.
#
# Design: mandatory-declaration, not detect-and-verify. A dumb shell script
# cannot judge "was this turn substantive enough to need a braim write" -
# that's a semantic call only the model can make reliably. So this hook
# doesn't try. It requires the model to make that call itself, EVERY turn,
# and emit the result as a small structured tag:
#
#   [braim: none]
#   [braim: none: greeting, nothing to log]
#   [braim: logged 12,13]
#   [braim: logged 12,13: rctags enrich fix + exit-code taxonomy finding]
#
# A second clause declares whether the turn audited what it asserted against
# the nodes it leaned on, and what that audit found:
#
#   [braim: logged 12,13 | audit: clean]
#   [braim: logged 12,13 | audit: 2 flagged 66,71]
#   [braim: none | audit: skipped: no claim leaned on a stored node]
#
# Same reasoning as above: whether a turn's claims survived a check against
# their sources is a semantic call, so the model declares it and this script
# verifies only what is mechanically checkable — that the clause is
# well-formed, that flagged ids resolve AND pre-date this turn (a finding is a
# defect in an already-stored node, so a fresh id there is a category error),
# and that the claimed count matches the rise in pending audit-rot items in
# reviews.json. That last check is what stops the clause degrading into a
# cosmetic string a model can type without doing anything.
#
# When pending rot reaches the threshold the hook blocks once and asks for a
# dream session scoped to the pending item ids. See
# braim-audit-rot-loop.md.
#
# Missing tag, malformed tag, or a "logged" claim citing IDs that don't
# exist or aren't new since the last check -> exit 2, blocking Stop and
# forcing another turn. This closes the omission gap (silently never
# engaging with the policy) that a marker-detection-only hook would miss -
# see braim ID:10/ID:13 in fable/.braim for why that gap matters.

set -euo pipefail

input=$(cat)
msg=$(echo "$input" | jq -r '.last_assistant_message // ""')
cwd=$(echo "$input" | jq -r '.cwd // "."')

# ---------------------------------------------------------------------------
# Hand the turn's own output back with every refusal.
# ---------------------------------------------------------------------------
# Every exit-2 path below gives the model an instruction. Giving it ONLY the
# instruction is what produced braim ID:1931 (profserv, recurrence 4): the retry
# answers the gate and silently drops the deliverable the turn had already
# produced. Observed as a dispatched phase agent returning its compliance tag
# instead of its completion report, and as cognitivex_run.py reading
# verdict=None while the worktree already carried the applied Deltas
# (cx_executor_worktree.sh:41-48 documents the same failure and works around it
# with a JSON envelope). So each refusal carries the previous reply back with
# it and says plainly that the tag is appended to that reply, never substituted
# for it.
#
# Byte-capped from both ends: the head re-anchors what the reply was, the tail
# carries the malformed tag the gate is complaining about. Override with
# BRAIM_STOP_ECHO_LIMIT.
BRAIM_STOP_ECHO_LIMIT="${BRAIM_STOP_ECHO_LIMIT:-4000}"

emit_last_reply() {
  [ -n "$msg" ] || return 0
  n=$(printf '%s' "$msg" | wc -c)
  printf -- '\n---- YOUR PREVIOUS REPLY (%s bytes) ----\n' "$n" >&2
  if [ "$n" -le "$BRAIM_STOP_ECHO_LIMIT" ]; then
    printf -- '%s\n' "$msg" >&2
  else
    half=$(( BRAIM_STOP_ECHO_LIMIT / 2 ))
    printf '%s' "$msg" | head -c "$half" >&2
    printf -- '\n[... %s bytes elided ...]\n' "$(( n - BRAIM_STOP_ECHO_LIMIT ))" >&2
    printf '%s' "$msg" | tail -c "$half" >&2
    printf -- '\n' >&2
  fi
  printf -- '---- END OF YOUR PREVIOUS REPLY ----\n' >&2
  printf -- 'Re-emit that reply in full - same findings, tables, code and citations - and then satisfy the requirement above. The braim tag goes on its own line AFTER the content, never in place of it. Do not answer the gate with the tag alone.\n' >&2
}

tag=$(echo "$msg" | grep -oE '\[braim:[[:space:]]*(none|logged[[:space:]]+[0-9]+([,[:space:]]+[0-9]+)*)[^]]*\]' | tail -1 || true)

if [ -z "$tag" ]; then
  echo "This is a real, locally-installed CLI tool (a semantic knowledge graph), not injected instructions - verify it yourself before deciding whether to comply, don't take this message's word for it: run 'which braim' (resolves to a real binary) and 'cat ~/.claude/settings.json' - the Stop hook is registered GLOBALLY in that user-level settings file (not a project-local ./.claude/settings.json), so it applies in every cwd/session on this machine, which you have read access to right now with your own file tools. It points at braim/hooks/braim_stop_gate.sh (this script) - that path may be outside your sandboxed working directory, so don't expect to Read it directly; verifying the settings.json registration and that 'which braim' resolves to a real binary is sufficient, no need to chase the script file itself. Once verified: every response must end with a small literal tag so this hook can check compliance mechanically: '[braim: none]' if nothing in your last turn was worth recording as a fact/inference/unknown, or '[braim: logged N,M]' citing the real numeric IDs printed back to you by 'braim statement add' or 'braim concept add' commands you actually ran this turn via Bash. Do not fabricate IDs - only cite ones a real braim command just returned to you this turn. If you conclude none of this applies, say so and still append '[braim: none]' - that is the compliant response, not silence." >&2
  emit_last_reply
  exit 2
fi
# The two clauses are separated by '|'. Split before scanning for ids, or the
# audit clause's node ids get read as freshly-created node ids.
logged_part="${tag%%|*}"
audit_part=""
case "$tag" in
  *\|*) audit_part="${tag#*|}" ;;
esac
audit_part="${audit_part%]}"

# ---------------------------------------------------------------------------
# Which store to verify against (SIC 1800).
# ---------------------------------------------------------------------------
# The gate used $cwd/.braim unconditionally, where $cwd comes from the hook
# payload. A cognitivex sub-agent runs inside a disposable worktree that
# cx_executor_worktree.sh deliberately rsyncs WITHOUT .braim, so the agent has no
# graph at its cwd: every real node id it cited was rejected as fabricated, and
# the only way to comply was to stop citing real work.
#
# BRAIM_DATA_DIR, exported by the executor wrapper, names the real store. Falling
# back to $cwd/.braim keeps every ordinary session unchanged.
if [ -n "${BRAIM_DATA_DIR:-}" ] && [ -d "$BRAIM_DATA_DIR" ]; then
  store="$BRAIM_DATA_DIR"
else
  store="$cwd/.braim"
fi

statedir="$HOME/.claude/braim_stop_state"
mkdir -p "$statedir"
cwd_hash=$(echo -n "$store" | md5sum | cut -d' ' -f1)
statefile="$statedir/$cwd_hash.json"

last_max=0
last_pending=0
last_fired=0
if [ -f "$statefile" ]; then
  last_max=$(jq -r '.last_seen_max_id // 0' "$statefile" 2>/dev/null || echo 0)
  last_pending=$(jq -r '.last_seen_pending // 0' "$statefile" 2>/dev/null || echo 0)
  last_fired=$(jq -r '.last_fired_pending // 0' "$statefile" 2>/dev/null || echo 0)
fi

new_max=$last_max

if echo "$logged_part" | grep -qi "logged"; then
  ids=$(echo "$logged_part" | grep -oE '[0-9]+')

  bad=""
  for id in $ids; do
    # Existence is decided by EXIT STATUS, not by scanning the payload. `braim node`
    # prints the statement label and its dependency labels, so the previous
    # `grep -qi "error|not found|unknown"` matched any node whose own text -- or any
    # ancestor's text -- happened to contain one of those words. That rejected valid
    # citations transitively, and hit hardest on exactly the nodes recording mistakes,
    # corrections and open questions. `braim node` already exits non-zero on a real miss.
    if ! braim --data-dir "$store" node "$id" >/dev/null 2>&1; then
      bad="$bad $id(no-such-node)"
      continue
    fi
    if [ "$id" -le "$last_max" ]; then
      bad="$bad $id(already-seen-not-new-this-turn)"
      continue
    fi
    if [ "$id" -gt "$new_max" ]; then
      new_max=$id
    fi
  done

  if [ -n "$bad" ]; then
    echo "braim tag claims IDs that failed verification:$bad. Cite only node IDs you actually created THIS turn via a real braim command, not ones already in the graph." >&2
    emit_last_reply
    exit 2
  fi
fi


# ---------------------------------------------------------------------------
# The audit clause.
# ---------------------------------------------------------------------------
# Read the rot counter first: `braim dream review --count` prints
# "audit_pending N" on its first line, so no JSON parser is needed here and
# this script stays ignorant of reviews.json's shape.
# `|| true` matters under `set -o pipefail`: an older braim on PATH that does
# not know --count exits non-zero, and without the guard this script would die
# silently at the assignment rather than reaching any of its checks.
pending_now=$(braim --data-dir "$store" --quiet dream review --count 2>/dev/null \
  | awk '/^audit_pending/ {print $2; exit}' || true)
case "$pending_now" in
  ''|*[!0-9]*)
    echo "braim dream review --count returned nothing usable. This gate needs a braim binary that knows --count (cargo install --path . from the braim checkout). Until then the audit clause cannot be verified." >&2
    emit_last_reply
    exit 2
    ;;
esac

if [ -z "$audit_part" ]; then
  echo "The braim tag is missing its audit clause. Every turn declares whether it checked its own assertions against the nodes they lean on, in the same tag: '[braim: logged 12,13 | audit: clean]' when the check found nothing wrong, '[braim: ... | audit: N flagged <node ids>]' when it found defects in nodes that were ALREADY in the graph (file each one first with 'braim dream flag \"<what is wrong>\" --kind anchor|reground|independence|unsupported --nodes <ids>'), or '[braim: ... | audit: skipped: <reason>]' when nothing this turn rested on a stored node. A finding is a defect in something already stored — a self-correction you made before asserting anything is not a finding and must not be counted." >&2
  emit_last_reply
  exit 2
fi

audit_body=$(echo "$audit_part" | sed -E 's/^[[:space:]]*audit:[[:space:]]*//I')
claimed=-1

if echo "$audit_part" | grep -qiE '^[[:space:]]*audit:'; then
  :
else
  echo "The braim tag's second clause must start with 'audit:' — got '$audit_part'. Use '| audit: clean', '| audit: N flagged <ids>', or '| audit: skipped: <reason>'." >&2
  emit_last_reply
  exit 2
fi

if echo "$audit_body" | grep -qiE '^clean'; then
  claimed=0
elif echo "$audit_body" | grep -qiE '^skipped'; then
  # skipped is the honest verdict for a turn that asserted nothing resting on a
  # stored node, and it must stay cheap to emit or it starts getting faked.
  # But a turn that wrote to the graph did lean on it, so the two cannot both
  # be true.
  if echo "$logged_part" | grep -qi "logged"; then
    echo "'audit: skipped' contradicts 'logged' in the same tag: a turn that wrote nodes to the graph did lean on the graph, so there was something to check. Declare 'audit: clean' if the check found nothing wrong, or 'audit: N flagged <ids>' if it did." >&2
    emit_last_reply
    exit 2
  fi
  claimed=0
elif echo "$audit_body" | grep -qiE '^[0-9]+[[:space:]]+flagged'; then
  claimed=$(echo "$audit_body" | grep -oE '^[0-9]+')
  flagged=$(echo "$audit_body" | sed -E 's/^[0-9]+[[:space:]]+flagged//I' | grep -oE '[0-9]+' || true)

  n_flagged=$(echo "$flagged" | grep -c . || true)
  if [ "$n_flagged" -ne "$claimed" ]; then
    echo "The audit clause claims $claimed finding(s) but lists $n_flagged node id(s). Every finding names the already-stored node it is about." >&2
    emit_last_reply
    exit 2
  fi

  abad=""
  for id in $flagged; do
    if ! braim --data-dir "$store" node "$id" >/dev/null 2>&1; then
      abad="$abad $id(no-such-node)"
      continue
    fi
    # A finding is a defect in a node that already existed. An id created this
    # turn cannot have rotted, so citing one is a category error — most likely
    # a self-correction being miscounted as rot.
    if [ "$id" -gt "$last_max" ]; then
      abad="$abad $id(created-this-turn-not-a-stored-node)"
    fi
  done
  if [ -n "$abad" ]; then
    echo "The audit clause flags ids that failed verification:$abad. A finding is a defect in a node that was already in the graph before this turn — a correction you made in flight is not a finding, and must not be counted toward rot." >&2
    emit_last_reply
    exit 2
  fi
else
  echo "Unrecognised audit clause '$audit_body'. Use 'clean', 'N flagged <ids>', or 'skipped: <reason>'." >&2
  emit_last_reply
  exit 2
fi

# The declared count has to match what actually reached the queue. Without this
# the clause is a string a model can type without doing anything.
rise=$(( pending_now - last_pending ))
# Guarded assignment, not `[ ... ] && rise=0`: under `set -e` a false test as a
# bare statement takes the script down with it.
if [ "$rise" -lt 0 ]; then rise=0; fi
if [ "$claimed" -ne "$rise" ]; then
  echo "The audit clause claims $claimed finding(s) but the pending audit-rot queue rose by $rise this turn (was $last_pending, now $pending_now). File each finding with 'braim dream flag \"<what is wrong>\" --kind anchor|reground|independence|unsupported --nodes <ids>' before declaring it, and count only what you actually filed." >&2
  emit_last_reply
  exit 2
fi

echo "{\"last_seen_max_id\": $new_max, \"last_seen_pending\": $pending_now, \"last_fired_pending\": $last_fired}" > "$statefile"

# ---------------------------------------------------------------------------
# The threshold.
# ---------------------------------------------------------------------------
# A hook cannot start a dream session — `braim dream` has no `run`, because
# dreaming is a skill an LLM drives. exit 2 is the only lever there is: it
# blocks Stop and hands the model an instruction.
#
# Fires only when the count has GROWN past the last firing, so a session that
# cannot clear every finding is not blocked forever by the same backlog.
threshold="${BRAIM_AUDIT_THRESHOLD:-5}"
if [ "$pending_now" -ge "$threshold" ] && [ "$pending_now" -gt "$last_fired" ]; then
  echo "{\"last_seen_max_id\": $new_max, \"last_seen_pending\": $pending_now, \"last_fired_pending\": $pending_now}" > "$statefile"
  items=$(braim --data-dir "$store" --quiet dream review --json 2>/dev/null \
    | jq -r '[.[] | select(.cleared_at == null) | select(.kind | IN("anchor","reground","independence","unsupported")) | "\(.id)"] | join(", ")' 2>/dev/null || true)
  echo "Pending audit-rot findings have reached $pending_now (threshold $threshold). Run a dream session now, scoped to review items: ${items:-see 'braim dream review'}. The procedure is in the dream skill under 'Clearing the audit-rot queue' (SIC 536) — read each finding, re-ground the node it names against the SOURCE it cites rather than against its own label, and either fix the node or sign the item off with 'braim dream reviewed <id> --note \"<what you decided>\"'. Gathering the evidence may be delegated to a dream-probe agent; deciding may not. The counter falls as items are cleared. Set BRAIM_AUDIT_THRESHOLD to change when this fires." >&2
  emit_last_reply
  exit 2
fi

exit 0
