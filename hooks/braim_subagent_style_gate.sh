#!/bin/bash
# SubagentStop hook: enforces length + filler-phrase discipline on
# subagent output specifically.
#
# Why this exists as a SEPARATE hook from braim_style_gate.sh: confirmed
# 2026-08-20 that UserPromptSubmit never fires for subagents at all (only
# for top-level session prompts), and SubagentStart/SubagentStop don't
# support additionalContext injection - there is no way to proactively
# tell a subagent about a style trait via hooks. SubagentStop CAN block
# (exit 2), so the only available lever is reactive: check the subagent's
# actual output, force a rewrite if it violates the standard, same
# mechanism as the Stop-gate, different (and only available) event.
#
# Checks two things a plain "be terse" instruction in a subagent's own
# prompt can't reliably self-enforce: a hard length ceiling, and the same
# curated filler blocklist used for the top-level session (kept identical
# on purpose - one standard, two enforcement points).

set -uo pipefail

input=$(cat)
msg=$(echo "$input" | jq -r '.last_assistant_message // ""')

MAX_CHARS=1200

len=${#msg}
if [ "$len" -gt "$MAX_CHARS" ]; then
  echo "Subagent response is $len characters, over the $MAX_CHARS-character limit. Condense to the essential findings only - drop restated context, preamble, and closing summary. Rewrite shorter and continue." >&2
  exit 2
fi

BLOCKLIST=(
  "i apologize"
  "i'm sorry for"
  "as an ai"
  "it'?s worth noting that"
  "it'?s important to note that"
  "it'?s important to mention"
  "i hope this helps"
  "please let me know if"
  "feel free to"
  "at the end of the day"
)

for phrase in "${BLOCKLIST[@]}"; do
  if echo "$msg" | grep -qiE "$phrase"; then
    echo "Subagent response contains filler phrase matching '$phrase'. Rewrite without it and continue." >&2
    exit 2
  fi
done

exit 0
