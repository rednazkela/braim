#!/bin/bash
# Stop hook: blocks a narrow, curated list of genuinely superfluous filler
# phrases - not a blanket hedge-word ban.
#
# Deliberately narrow by design, not by oversight. The original
# straightforward-personality trait banned hedging outright, but its own
# worked examples show the actual intent was narrower: don't hedge in
# prose, mark uncertainty explicitly instead (?[unknown]) - hedging and
# the marker system were meant to pair, not compete. A blanket ban on
# words like "likely"/"possibly" would force false confidence exactly
# where real uncertainty exists, fighting the evidence-discipline goal
# this same graph already tracks (goal taxonomy B1).
#
# Also deliberately narrow because of a directly-observed failure mode:
# a Stop hook that fires on every turn and repeats an unchanging blocking
# message pushed a blind instance into treating it as a prompt-injection
# attempt and refusing outright (see braim ID:25 in fable/.braim). Style
# violations are far more frequent than a missing braim tag ever was, so
# a stricter gate here would hit that same escalation risk much harder.
# Keeping the blocklist small and the check purely mechanical (exact
# phrase match, not semantic judgment) keeps the fire rate low.
#
# Scope: filler/throat-clearing only. Genuine hedge words (likely,
# probably, may, might, could) are NOT blocked - that's for the marker
# system to carry, not this hook.

set -uo pipefail

input=$(cat)
msg=$(echo "$input" | jq -r '.last_assistant_message // ""')

# Case-insensitive, whole-phrase matches only - short list, reviewed for
# false-positive risk before adding anything new.
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

hit=""
for phrase in "${BLOCKLIST[@]}"; do
  if echo "$msg" | grep -qiE "$phrase"; then
    hit="$phrase"
    break
  fi
done

if [ -n "$hit" ]; then
  echo "Response contains filler phrase matching '$hit'. This is a narrow, curated blocklist of superfluous filler (not a hedge-word ban - genuine uncertainty should still be marked via ?[unknown], not suppressed). Rewrite without it and continue." >&2
  exit 2
fi

exit 0
