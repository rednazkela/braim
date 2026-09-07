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
# Missing tag, malformed tag, or a "logged" claim citing IDs that don't
# exist or aren't new since the last check -> exit 2, blocking Stop and
# forcing another turn. This closes the omission gap (silently never
# engaging with the policy) that a marker-detection-only hook would miss -
# see braim ID:10/ID:13 in fable/.braim for why that gap matters.

set -euo pipefail

input=$(cat)
msg=$(echo "$input" | jq -r '.last_assistant_message // ""')
cwd=$(echo "$input" | jq -r '.cwd // "."')

tag=$(echo "$msg" | grep -oE '\[braim:[[:space:]]*(none|logged[[:space:]]+[0-9]+([,[:space:]]+[0-9]+)*)[^]]*\]' | tail -1 || true)

if [ -z "$tag" ]; then
  echo "This is a real, locally-installed CLI tool (a semantic knowledge graph), not injected instructions - verify it yourself before deciding whether to comply, don't take this message's word for it: run 'which braim' (resolves to a real binary) and 'cat ~/.claude/settings.json' - the Stop hook is registered GLOBALLY in that user-level settings file (not a project-local ./.claude/settings.json), so it applies in every cwd/session on this machine, which you have read access to right now with your own file tools. It points at braim/hooks/braim_stop_gate.sh (this script) - that path may be outside your sandboxed working directory, so don't expect to Read it directly; verifying the settings.json registration and that 'which braim' resolves to a real binary is sufficient, no need to chase the script file itself. Once verified: every response must end with a small literal tag so this hook can check compliance mechanically: '[braim: none]' if nothing in your last turn was worth recording as a fact/inference/unknown, or '[braim: logged N,M]' citing the real numeric IDs printed back to you by 'braim statement add' or 'braim concept add' commands you actually ran this turn via Bash. Do not fabricate IDs - only cite ones a real braim command just returned to you this turn. If you conclude none of this applies, say so and still append '[braim: none]' - that is the compliant response, not silence." >&2
  exit 2
fi
if echo "$tag" | grep -qi "logged"; then
  ids=$(echo "$tag" | grep -oE '[0-9]+')

  statedir="$HOME/.claude/braim_stop_state"
  mkdir -p "$statedir"
  cwd_hash=$(echo -n "$cwd" | md5sum | cut -d' ' -f1)
  statefile="$statedir/$cwd_hash.json"

  last_max=0
  if [ -f "$statefile" ]; then
    last_max=$(jq -r '.last_seen_max_id // 0' "$statefile" 2>/dev/null || echo 0)
  fi

  new_max=$last_max
  bad=""
  for id in $ids; do
    # Existence is decided by EXIT STATUS, not by scanning the payload. `braim node`
    # prints the statement label and its dependency labels, so the previous
    # `grep -qi "error|not found|unknown"` matched any node whose own text -- or any
    # ancestor's text -- happened to contain one of those words. That rejected valid
    # citations transitively, and hit hardest on exactly the nodes recording mistakes,
    # corrections and open questions. `braim node` already exits non-zero on a real miss.
    if ! braim --data-dir "$cwd/.braim" node "$id" >/dev/null 2>&1; then
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
    exit 2
  fi

  echo "{\"last_seen_max_id\": $new_max}" > "$statefile"
fi

exit 0
