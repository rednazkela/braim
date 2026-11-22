#!/bin/bash
# PreCompact hook: auto-checkpoints the local braim graph before compaction.
#
# Confirmed 2026-08-19: PreCompact can only block (exit 2) or allow (exit 0)
# - it cannot inject additionalContext into the compaction/summarization
# process the way UserPromptSubmit can. That kills the original design in
# compaction_braim_discipline.md ("HARD enforcement = a PreCompact hook
# injecting these rules into the summarizer") - there's no lever to shape
# how the compactor writes its summary.
#
# This does something more reliable instead: rather than hoping a
# compaction summary is braim-ID-relational (an LLM-compliance bet this
# session has repeatedly shown fails for advisory mechanisms), it removes
# the LLM from the safety-critical path entirely. Whatever the compacted
# prose ends up saying, the actual graph state is preserved in a real,
# retrievable version snapshot - `braim version save` - taken
# unconditionally the moment compaction is about to happen. No judgment
# call, no compliance to bet on.
#
# Never blocks compaction on its own account - a checkpoint failure is
# logged to stderr (visible to the user) but does not stop the user's
# actual workflow. Blocking stays available (exit 2) for a future,
# separate policy decision, not exercised here.

set -uo pipefail

input=$(cat)
cwd=$(echo "$input" | jq -r '.cwd // "."')

if [ -d "$cwd/.braim" ]; then
  out=$(braim --data-dir "$cwd/.braim" version save "auto-checkpoint before compaction $(date -u +%Y-%m-%dT%H:%M:%SZ)" 2>&1)
  rc=$?
  if [ $rc -ne 0 ]; then
    echo "braim precompact checkpoint FAILED (compaction proceeding anyway): $out" >&2
  fi
fi

exit 0
