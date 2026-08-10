---
name: dream
description: Run one overnight dreaming session over a local braim graph — adjudicate candidate node pairs for missing relations, duplicates, and contradictions, and relax load-bearing constraints to find the ones the graph has already outgrown, writing every verdict back under a skeptical evidence protocol. Use when the user asks to dream, run a dream session, ask what-if, or consolidate a graph overnight.
---

# Dreaming: adjudicate candidate pairs

braim's `dream candidates` picks node pairs worth examining. This skill is the
other half: reading each pair, deciding whether a relation is *real*, and writing
the verdict back. braim does structure; you do judgment.

`$1` (optional) = how many pairs to adjudicate this session. Default **15**.
`$2` (optional) = data dir. Default: the project's `.braim`.

## The prior you must hold

An LLM asked "do these two nodes relate?" will nearly always say yes. That
tendency, pointed at a knowledge graph with write access, is a confabulation
pump — it would invert braim's entire purpose, which is a context an agent
cannot silently corrupt.

So the default verdict is **no-relation**. A relation earns a record only when
you can state the connection in one concrete sentence that names something
specific in both nodes. "Both concern billing" is not a relation. "The proration
bug in A is caused by the anniversary-cycle rule stated in B" is.

Expect most pairs to be nothing. If more than roughly 40% of a session's pairs
come back as relations, say so in the report — that is evidence your prior
slipped, not that the graph is unusually rich.

## Setup

```bash
braim dream candidates --limit <N> --json          # the worklist
braim dream candidates --strategy semantic --json  # highest precision first
```

Spend the budget **semantic first, then shared-source, then two-hop**: measured
on a real 714-node graph those yielded 30 / 148 / 5135 candidates respectively,
so two-hop is a reserve, not a starting point.

Dreaming is refused on graphs marked `.braim.central` — a dream is an unreviewed
hypothesis and an unattended central has no reviewer. Work on a local graph.

## After an ingest: work the frontier first

Once a graph has been swept, old-versus-old is where the ledger is dense and the
remaining candidates score flat — braim stops discriminating and every pair looks
alike (measured at 0.018 across 319 survivors, braim ID:1329). New material
changes that, and the pairs worth working are the ones involving the new nodes.

```bash
python3 <skill-dir>/frontier.py .braim <highest-id-before-the-ingest> \
  /path/to/semantic.json /path/to/shared-source.json /path/to/two-hop.json
```

It lists unadjudicated, adjudicable pairs touching anything above that id, and
puts **cross-era** pairs first — a new node against an older one brings two
vintages of evidence together, where frontier-versus-frontier is usually one
ingest talking to itself.

This is a **scheduling** rule, not a yield rule. Recency does not predict a
finding: bucketing 1894 adjudications by the newest node in the pair gives 7.3 /
11.7 / 6.7 / 7.3 percent, and by id-gap 7.8 / 9.4 / 6.7 / 6.5 / 0 percent — flat
either way (braim ID:1330). It earns its place only because after an ingest the
new nodes' pairs are the ones that have not been looked at.

## Pre-pass: measure the highest-exposure unverified nodes

Before adjudicating anything, spend a few minutes settling the claims you are
about to reason *from*. Every pair you judge rests on both nodes' labels, and a
label you have not checked is a premise you are taking on trust.

```bash
python3 <skill-dir>/rank_exposure.py .braim 10 \
  /path/to/semantic.json /path/to/shared-source.json /path/to/two-hop.json
```

It ranks nodes that are **weak** (unproven or contested) *and* **measurable**
(sourced to `code`/`schema`/`config`/`test`) by how many unadjudicated pairs they
sit in. Take the top **two or three** and settle them:

1. Run the command that decides the claim — a `grep -c`, an `ls | wc -l`, a
   `sed` of the cited lines. Read the output.
2. If it confirms the label, record the observation as a first-class source and
   attach it. This is evidence, not fiat — you ran the command:
   ```bash
   braim source add "<what was counted>" --type test \
     --location "test:<exact command> = <exact result>, observed <YYYY-MM-DD>"
   braim statement add-source <node> --source-id <new-source-id>
   ```
3. If it refutes the label, do **not** attach. Raise a contradiction against
   whichever node disagrees, or record a correction statement citing the
   measurement, and leave the original text alone.
4. If the claim cannot be settled from this checkout at all — the cited file is
   missing, the path moved, the evidence only exists off-disk — mark it so the
   ranking stops offering it, and say which correction records the finding:
   ```bash
   braim meta <node> --set measured=unfixable
   braim meta <node> --set measured_note="<why, and the correction's ID>"
   ```
   Without this the node ranks first every round forever, since no measurement
   can change its status (ID:684 topped two consecutive rounds — braim ID:1282).

Two things this pre-pass is **not**:

- It is not a way to lift dependents. Verification is computed at creation and
  never propagates; `add-source` recomputes from sources only and ignores the
  dependency cap. Promoting a parent moves nothing downstream (measured: 13
  promotions moved the unproven count by one — braim ID:1251).
- It is not a pair filter. Pairs where one side is weak-and-measurable yield
  findings at **4.8%** against **9.5%** for the rest — testability is
  anti-predictive at the pair level and useful only at the node level (braim
  ID:1272).

Do not accept `braim statement verify-suggest` output as the measurement. It
ranks by graph adjacency, not evidential relevance: asked to promote a claim
about a directory's file count it offered a Prismatic design document, labelled
"Promotion impact: proven" (braim ID:1240). Use it to find *statements worth
measuring*, never as a list of sources worth attaching.

## Per pair

**1. Read both nodes.**
```bash
braim node <a>
braim node <b>
```
If either command fails, the node was merged away earlier in this session.
Record `no-relation` with a note saying so, and move on.

**2. Read the cited sources — actually open them.**
Every node lists `Sources`. Open the files and line ranges with Read/Grep. This
is the step that separates a verdict from a guess, and it is where the overnight
tokens should go. A node label is a pointer, not evidence: if the label and the
source document disagree, the document wins.

**3. Choose exactly one verdict.**

| Verdict | When |
|---|---|
| `no-relation` | the default — nothing specific connects them, **or** the relation is real but already asserted by an existing statement, in which case say so in the note rather than restating it |
| `duplicate` | both assert the same thing about the same subject |
| `contradiction` | both are about the same subject and cannot both be true |
| `proposed` | a real relation, but you could not verify it in sources |
| `verified` | a real relation AND you read PRIMARY sources that establish it |

**4. Act on the verdict.**

`no-relation` — no graph write.

`duplicate` — pick the survivor by **verification status first**, then by
referent count, and only then by label precision. Status is the right primary
key because verification is `MIN(source-derived, weakest statement dependency)`:
the winner's dependency structure sets a ceiling that unioned sources cannot
lift. Choosing the prettier label over the better-verified node demotes the
surviving knowledge — measured, not hypothetical (braim ID:262).

```bash
braim merge-nodes <winner> <loser>
```
The loser's label is destroyed, so if it carried detail the winner lacks, say so
in the report. If the command warns about dependencies only the loser had, **do
not** wire them in yourself; note them for the human.

`contradiction` —
```bash
braim statement contradict <a> <b> --reason "<what specifically conflicts>"
```
Both move to contested. Do not pick a winner; resolution needs a third source.

`proposed` — record the hypothesis, unproven, for a human:
```bash
braim statement add "<relation in one sentence>" \
  --domains "<domain>" --sources "narrative:dream-<YYYY-MM-DD>" \
  --depends "<a>:0.6,<b>:0.4" --assume
braim meta <new-id> --set scope=dream
braim meta <new-id> --set terminal_cause=true
```

`verified` — same, but cite the sources you actually read:
```bash
braim statement add "<relation in one sentence>" \
  --domains "<domain>" --sources "code:<file>:<lines>,doc:<file>:<section>" \
  --depends "<a>:0.6,<b>:0.4" --assume
braim meta <new-id> --set scope=dream
braim meta <new-id> --set terminal_cause=true
```
braim's own math decides the resulting status from PRIMARY-type diversity.
**Never promote by fiat**, never cite a file you did not open, and never reuse a
source string copied from a node label without confirming it in the file.

**Before every `statement add`, query the paths you are about to cite:**
```bash
python3 <skill-dir>/whocites.py .braim "<each --sources path>"
```
Read whatever it prints. If an existing node already carries the claim, record
`no-relation` with a pointer to it instead of writing a second one.

`braim query` is not a substitute here. It matches prose, and two statements
about the same file need not share a single content word — five duplicates in
one session came from querying the new finding's own wording, or from skipping
the check because the finding came straight off a file read (braim ID:1233,
ID:1284). The path is the reliable key, so the check runs on the path.

**5. Record it**, always, whatever the verdict:
```bash
braim dream seen <a> <b> --verdict <verdict> --note "<one line>"
```
This is what stops the next session re-treading the same pair.

## What-if: relax a constraint

Pairs are one half of a session. The other is asking what the graph would look
like if one of its load-bearing statements stopped being true — the technique a
dream applies to a life, applied to a knowledge graph.

Run this **after** the pairs, or instead of them when the user asks for what-if
directly. Two or three constraints is a session; there are never many worth
walking.

```bash
braim dream constraints --limit 10     # rank causes by what rests on them
braim dream whatif <id>                # walk the one you picked
```

Constraints already walked are withheld, so a nightly loop advances instead of
re-offering last night's list. Anything marked `REOPENED` has new evidence
behind it and should be walked first — that is the whole point of the marker.

`constraints` ranks by blast radius scaled by evidence — it cannot tell a
constraint from any other cause, and does not pretend to. Limitation vocabulary
matched 61 of 161 statements on a real graph, mostly false positives, so
`reads_as_limitation` is annotation, never ranking. **You** decide which of the
top entries is actually a constraint someone could lift. Skip the ones that are
just facts about how things are.

### Staleness first — this is the part that produces findings

`whatif` prints staleness signals before anything else. Work them first and be
willing to stop there.

A signal is a statement citing the same PRIMARY source **file** as the
constraint, written later, evidenced at least as well, with no contradiction
linking the pair yet. Ranking is by shared rare wording, not by the file alone —
on a 5000-line hub file the file by itself ranks the whole neighbourhood (braim
ID:332). The constraint's own consequents are excluded, so anything listed is
genuinely off its chain.

Open the sources on both sides. If current evidence supersedes the constraint:

```bash
braim statement contradict <constraint> <superseder> --reason "<what specifically changed>"
braim dream seen <constraint> <superseder> --verdict contradiction --note "<one line>"
```

**Then stop.** There is no counterfactual to imagine about a constraint that no
longer holds — you found an obsolete fact being served as a current one, which is
the whole yield of this mode (braim ID:324). ID:186 and ID:189 sat in this graph
in exactly that state, both `partial`, unlinked, for weeks.

A ranked signal is a lead, not a verdict. Most will be statements that merely
touch the same file.

### If the constraint still holds

Then, and only then, relax it. `whatif` gives you the two things the walk is for:
what **rests on** the constraint (nearest first — those are the statements that
come into play) and what it **serves** (the root goal the chain ends at).

For each statement resting on it, say what it becomes once the constraint is
lifted: **unchanged**, **weaker**, or **void**. Most are unchanged; say so. Then
name the single change that would most move the root goal — one, not a list.

Write at most one statement per constraint, and tag it:

```bash
braim statement add "<what becomes possible, in one sentence>" \
  --domains "<domain>" --sources "narrative:whatif-<YYYY-MM-DD>" \
  --depends "<constraint>:0.7,<the-statement-it-unblocks>:0.3" --assume
braim meta <new-id> --set counterfactual=true
braim meta <new-id> --set scope=dream
braim why-add <new-id> --because <constraint> --source "narrative:whatif-<YYYY-MM-DD>"
```

`counterfactual=true` is load-bearing. Export strips those nodes **and everything
depending on them** at the import boundary, and reports the count — a what-if is
unverifiable by construction, since no source can prove that removing a
constraint would improve an outcome (braim ID:322, ID:333). Never remove the tag
to publish one, and never cite a PRIMARY source on a counterfactual: the sources
would be real and the claim still would not be.

### Close the walk

Mark the constraint, **whether or not** it produced anything. A walk that found
nothing is a result:

```bash
braim meta <constraint> --set whatif_walked=true
braim meta <constraint> --set whatif_walked_at=<YYYY-MM-DD>
```

`constraints` reads both. A walked constraint drops off the list and is reported
as withheld, not silently omitted — and it comes back on its own, flagged
`REOPENED`, once a statement arrives that the walk could not have seen. That is
the staleness probe pointed at the walk itself, so it reopens on the same
evidence a fresh walk would find.

**Set the date.** Without `whatif_walked_at` there is nothing to compare against
and the constraint stays closed permanently — it will show in the withheld count
and never reopen.

`--include-walked` is the way back in when you want one anyway. The dream ledger
is keyed on pairs and cannot hold a single-node walk, which is why the marker
lives on the node.

## Rules that keep the graph sound

- Weights in `--depends` must sum to 1.0 and should be **asymmetric** — equal
  weights assert no opinion about which node carries the relation.
- A statement must express a relationship between both nodes. A sentence that
  only elaborates one of them is not a relation.
- Never `--force`, never delete a node, never resolve a contradiction, never
  edit an existing statement's text. Dreaming adds hypotheses and consolidates
  duplicates; it does not rewrite established knowledge.
- Everything you create carries `scope=dream` so a human can review the whole
  session with `braim list --meta scope=dream`.
- A what-if output additionally carries `counterfactual=true`, and nothing else
  ever does. Tagging an ordinary finding that way quarantines it for good;
  leaving it off a hypothesis lets a hypothesis publish as a finding.

## Report

Finish with a short prose summary the user can read at breakfast:

- the pre-pass: which nodes you measured, the command and result for each, and
  whether the measurement confirmed or refuted the label
- pairs adjudicated, and the verdict counts
- every `verified` and `duplicate` with its node ids, since those changed the graph
- anything that looked like a contradiction you were not confident enough to raise
- the relation rate, flagged if it exceeded ~40%
- constraints walked: which ones, whether each turned out stale, and for the ones
  that held, the single change you named — separate the stale findings from the
  counterfactuals, because only the first kind is a finding
- what to review: `braim list --meta scope=dream`, and
  `braim list --meta counterfactual=true` for the hypotheses, which never leave
  this graph

State plainly if the session found nothing. A night that produces no relations is
a correct outcome, not a failed run.
