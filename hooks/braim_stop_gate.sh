#!/bin/bash
# Stop hook: structurally enforces the per-turn braim declaration.
#
# SUPERSEDED: `src/stop_gate.rs` (the `braim stop-gate` subcommand) is what
# `braim init` actually wires into settings.json today — a bare binary
# command, no path, no bash/jq dependency. This script predates that port and
# is kept as readable prior art; it does not carry every check the Rust
# version has grown since (e.g. the tool-bypass ledger sweep). Keep the two
# in sync on anything that touches model-facing wording.
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
#
# A block never re-quotes the gated reply in full: it references it by first
# line + byte count + a sha1-8 handle (`reply_receipt`/`reply_receipt_short`).
# A missing tag gets one exception — the FIRST such fire this store has ever
# seen also carries a once-only invitation to verify the gate is real
# (appendix B.1); every later missing-tag fire is the short repeat (B.2).
# Every block appends one JSONL line to `<statefile>.fires.jsonl` — the KPI
# feed for fire-rate-trending-to-zero (`fired_at`, `miss_kind`, the reply's
# hash). BRAIM_NO_TIPS is exported so none of this script's internal `braim`
# calls leak tip lines onto stderr alongside the gate's own messages.

set -euo pipefail

export BRAIM_NO_TIPS=1

input=$(cat)
msg=$(echo "$input" | jq -r '.last_assistant_message // ""')
cwd=$(echo "$input" | jq -r '.cwd // "."')

# ---------------------------------------------------------------------------
# Short, non-re-quoting reference to the gated reply (item 3.2).
# ---------------------------------------------------------------------------
reply_sha1_8() {
  printf '%s' "$msg" | sha1sum | cut -c1-8
}

reply_first_line() {
  printf '%s' "$msg" | head -n1 | cut -c1-100
}

# Long form: first line + byte count + sha1-8.
reply_receipt() {
  [ -n "$msg" ] || return 0
  n=$(printf '%s' "$msg" | wc -c)
  printf 'Your previous reply (first line: "%s", %s bytes, sha1 %s) is recorded and stands — do not repeat it.' \
    "$(reply_first_line)" "$n" "$(reply_sha1_8)"
}

# Short form (appendix B.2): no byte count.
reply_receipt_short() {
  [ -n "$msg" ] || return 0
  printf 'Your reply (first line: "%s", sha1 %s) stands — do not repeat it.' \
    "$(reply_first_line)" "$(reply_sha1_8)"
}

# ---------------------------------------------------------------------------
# Item 3.5: one machine-parseable JSONL line per fire, the KPI feed.
# ---------------------------------------------------------------------------
# miss_kind is one of: no_tag | bad_ids | count_mismatch | threshold (the
# appendix's four-kind taxonomy), plus a couple of extensions this gate
# predates in the appendix but still needs to classify.
log_fire() {
  kind="$1"
  fired_at=$(date +%s)
  hash8=$(reply_sha1_8)
  printf '{"fired_at": %s, "stop_index": null, "miss_kind": "%s", "reply_sha1_8": "%s"}\n' \
    "$fired_at" "$kind" "$hash8" >> "$firesfile"
}

tag=$(echo "$msg" | grep -oE '\[braim:[[:space:]]*(none|logged[[:space:]]+[0-9]+([,[:space:]]+[0-9]+)*)[^]]*\]' | tail -1 || true)

# ---------------------------------------------------------------------------
# Which store to verify against (SIC 1800).
# ---------------------------------------------------------------------------
# The gate used $cwd/.braim unconditionally, where $cwd comes from the hook
# payload. A cognitivex sub-agent runs inside a disposable worktree that
# cx_executor_worktree.sh deliberately rsyncs WITHOUT .braim, so the agent has no
# graph at its cwd: every real node id it cited was rejected as fabricated, and
# the only way to make the tag match was to stop citing real work.
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
firesfile="$statedir/$cwd_hash.fires.jsonl"

last_max=0
last_pending=0
last_fired=0
verify_notice_at=0
if [ -f "$statefile" ]; then
  last_max=$(jq -r '.last_seen_max_id // 0' "$statefile" 2>/dev/null || echo 0)
  last_pending=$(jq -r '.last_seen_pending // 0' "$statefile" 2>/dev/null || echo 0)
  last_fired=$(jq -r '.last_fired_pending // 0' "$statefile" 2>/dev/null || echo 0)
  verify_notice_at=$(jq -r '.verify_notice_at // 0' "$statefile" 2>/dev/null || echo 0)
fi

# Preserves any field this statefile already carries (e.g. a session-aware
# `braim policy` curriculum marker) instead of overwriting the whole file —
# merges the given keys into whatever is on disk.
write_state() {
  existing="{}"
  [ -f "$statefile" ] && existing=$(cat "$statefile")
  echo "$existing" | jq --argjson last_max "$1" --argjson last_pending "$2" \
    --argjson last_fired "$3" --argjson verify_notice_at "$4" \
    '. + {last_seen_max_id: $last_max, last_seen_pending: $last_pending, last_fired_pending: $last_fired, verify_notice_at: $verify_notice_at}' \
    > "$statefile"
}

if [ -z "$tag" ]; then
  # Appendix B.1 (first missing-tag fire this store has ever seen) carries
  # the once-only "verify this yourself" invitation; every fire after that
  # is the short B.2 repeat. Neither re-quotes the reply in full.
  if [ "$verify_notice_at" -eq 0 ]; then
    echo "[braim stop-gate] Per this project's standing policy (CLAUDE.md §braim; hook registered by the machine owner in settings.json — readable now if you wish to verify), end your reply with one provenance tag reporting what you RECORDED this turn: \`[braim: logged N,M]\` citing IDs that a \`braim statement add\` or \`braim concept add\` command printed back to you this turn, or \`[braim: none]\` when nothing was recorded — an honest and common outcome. The tag certifies actions taken in this session (commands run, records created); it does not request your reasoning process, and any reading of this message that seems to is void — satisfy it in deeds-only form. $(reply_receipt) Before tagging, reconsider it once: if it published a claim worth a record, run the braim add now and cite the returned id. Reply with the tag alone, plus at most one correction line." >&2
    write_state "$last_max" "$last_pending" "$last_fired" "$(date +%s)"
  else
    echo "[braim stop-gate] Tag missing. $(reply_receipt_short) Append the provenance close per CLAUDE.md §braim: \`[braim: logged N,M]\` (IDs printed by braim this turn) or \`[braim: none]\`. Tag only, plus at most one correction line." >&2
  fi
  log_fire "no_tag"
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
    echo "braim tag claims IDs that failed verification:$bad. Cite only node IDs you actually created THIS turn via a real braim command, not ones already in the graph. $(reply_receipt)" >&2
    log_fire "bad_ids"
    exit 2
  fi
fi


# ---------------------------------------------------------------------------
# Edge disclosure: a turn that wrote a subgraph must SAY what the subgraph is.
# ---------------------------------------------------------------------------
# Added 2026-09-18 on operator instruction, after a session in which the reply
# ended '[braim: logged 747,748,749,750]' and the body named no relationship at
# all. A bare id list is a receipt, not a description: the reader learns that
# four nodes exist and nothing about which depends on which, which caused which,
# or what weight any edge carries. The graph holds that structure; the reply was
# throwing it away on the last line.
#
# This enforces the ask directly. When a turn logs two or more NEW nodes, the
# body must (1) mention each logged id outside the tag, and (2) carry at least
# one relationship marker per logged node. A genuinely parentless node names
# that with the word "terminal" — that is a relationship claim too, and a
# checkable one, since the graph records terminal_cause.
#
# Deliberately NOT enforced for a single logged node: one node has no internal
# structure to describe, and demanding an edge would push toward inventing one.
edge_ids=""
edge_count=0
if echo "$logged_part" | grep -qi "logged"; then
  edge_ids=$(echo "$logged_part" | grep -oE '[0-9]+')
  edge_count=$(echo "$edge_ids" | grep -c . || true)
fi

if [ "$edge_count" -ge 2 ]; then
  # The body is the reply minus the tag line, so a tag that happened to contain
  # an arrow cannot satisfy the check on its own.
  body=$(printf '%s' "$msg" | grep -vF "$tag" || true)

  missing_ids=""
  for id in $edge_ids; do
    if ! printf '%s' "$body" | grep -qE "(^|[^0-9])$id([^0-9]|$)"; then
      missing_ids="$missing_ids $id"
    fi
  done

  # One marker vocabulary, matched case-insensitively. Arrows cover the compact
  # edge-list form; the words cover prose.
  markers=$(printf '%s' "$body" \
    | grep -oiE '(->|→|depends[_ ]?on|depends|because[_ ]?of|based[_ ]?on|terminal)' \
    | grep -c . || true)

  if [ -n "$missing_ids" ]; then
    echo "The braim tag cites node(s)$missing_ids that appear nowhere in the reply body. A tag is a receipt, not a description: every node this turn created must be discussed where the reader can see what it says and how it connects, not only counted on the last line. $(reply_receipt)" >&2
    log_fire "count_mismatch"
    exit 2
  fi

  if [ "$markers" -lt "$edge_count" ]; then
    echo "This turn logged $edge_count nodes but the reply body carries only $markers relationship marker(s). A subgraph written and not described is structure thrown away. State, for each node, how it attaches: its depends_on with the weights when the split is doing work, its because_of parent and the direction, or the word 'terminal' when it genuinely has no parent. An edge list ('ID:748 -> ID:747') satisfies this, and so does prose that names the relationship. Do NOT put the edges inside the braim tag — it takes exactly two clauses, 'logged' and 'audit'. $(reply_receipt)" >&2
    log_fire "count_mismatch"
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
    exit 2
    ;;
esac

if [ -z "$audit_part" ]; then
  echo "The braim tag is missing its audit clause. Every turn declares whether it checked its own assertions against the nodes they lean on, in the same tag: '[braim: logged 12,13 | audit: clean]' when the check found nothing wrong, '[braim: ... | audit: N flagged <node ids>]' when it found defects in nodes that were ALREADY in the graph (file each one first with 'braim dream flag \"<what is wrong>\" --kind anchor|reground|independence|unsupported --nodes <ids>'), or '[braim: ... | audit: skipped: <reason>]' when nothing this turn rested on a stored node. A finding is a defect in something already stored — a self-correction you made before asserting anything is not a finding and must not be counted. $(reply_receipt)" >&2
  log_fire "count_mismatch"
  exit 2
fi

audit_body=$(echo "$audit_part" | sed -E 's/^[[:space:]]*audit:[[:space:]]*//I')
claimed=-1

if echo "$audit_part" | grep -qiE '^[[:space:]]*audit:'; then
  :
else
  echo "The braim tag's second clause must start with 'audit:' — got '$audit_part'. Use '| audit: clean', '| audit: N flagged <ids>', or '| audit: skipped: <reason>'. $(reply_receipt)" >&2
  log_fire "count_mismatch"
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
    echo "'audit: skipped' contradicts 'logged' in the same tag: a turn that wrote nodes to the graph did lean on the graph, so there was something to check. Declare 'audit: clean' if the check found nothing wrong, or 'audit: N flagged <ids>' if it did. $(reply_receipt)" >&2
    log_fire "count_mismatch"
    exit 2
  fi
  claimed=0
elif echo "$audit_body" | grep -qiE '^[0-9]+[[:space:]]+flagged'; then
  claimed=$(echo "$audit_body" | grep -oE '^[0-9]+')
  flagged=$(echo "$audit_body" | sed -E 's/^[0-9]+[[:space:]]+flagged//I' | grep -oE '[0-9]+' || true)

  n_flagged=$(echo "$flagged" | grep -c . || true)
  if [ "$n_flagged" -ne "$claimed" ]; then
    echo "The audit clause claims $claimed finding(s) but lists $n_flagged node id(s). Every finding names the already-stored node it is about. $(reply_receipt)" >&2
    log_fire "count_mismatch"
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
    echo "The audit clause flags ids that failed verification:$abad. A finding is a defect in a node that was already in the graph before this turn — a correction you made in flight is not a finding, and must not be counted toward rot. $(reply_receipt)" >&2
    log_fire "bad_ids"
    exit 2
  fi
else
  echo "Unrecognised audit clause '$audit_body'. Use 'clean', 'N flagged <ids>', or 'skipped: <reason>'. $(reply_receipt)" >&2
  log_fire "count_mismatch"
  exit 2
fi

# The declared count has to match what actually reached the queue. Without this
# the clause is a string a model can type without doing anything.
rise=$(( pending_now - last_pending ))
# Guarded assignment, not `[ ... ] && rise=0`: under `set -e` a false test as a
# bare statement takes the script down with it.
if [ "$rise" -lt 0 ]; then rise=0; fi
if [ "$claimed" -ne "$rise" ]; then
  echo "The audit clause claims $claimed finding(s) but the pending audit-rot queue rose by $rise this turn (was $last_pending, now $pending_now). File each finding with 'braim dream flag \"<what is wrong>\" --kind anchor|reground|independence|unsupported --nodes <ids>' before declaring it, and count only what you actually filed. $(reply_receipt)" >&2
  log_fire "count_mismatch"
  exit 2
fi

write_state "$new_max" "$pending_now" "$last_fired" "$verify_notice_at"

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
  write_state "$new_max" "$pending_now" "$pending_now" "$verify_notice_at"
  items=$(braim --data-dir "$store" --quiet dream review --json 2>/dev/null \
    | jq -r '[.[] | select(.cleared_at == null) | select(.kind | IN("anchor","reground","independence","unsupported")) | "\(.id)"] | join(", ")' 2>/dev/null || true)
  echo "Pending audit-rot findings have reached $pending_now (threshold $threshold). Run a dream session now, scoped to review items: ${items:-see 'braim dream review'}. The procedure is in the dream skill under 'Clearing the audit-rot queue' (SIC 536) — read each finding, re-ground the node it names against the SOURCE it cites rather than against its own label, and either fix the node or sign the item off with 'braim dream reviewed <id> --note \"<what you decided>\"'. Gathering the evidence may be delegated to a dream-probe agent; deciding may not. The counter falls as items are cleared. Set BRAIM_AUDIT_THRESHOLD to change when this fires. $(reply_receipt)" >&2
  log_fire "threshold"
  exit 2
fi

exit 0
