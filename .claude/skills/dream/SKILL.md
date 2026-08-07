---
name: dream
description: Run one overnight dreaming session over a local braim graph — adjudicate candidate node pairs for missing relations, duplicates, and contradictions, writing every verdict back under a skeptical evidence protocol. Use when the user asks to dream, run a dream session, or consolidate a graph overnight.
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
| `no-relation` | the default — nothing specific connects them |
| `duplicate` | both assert the same thing about the same subject |
| `contradiction` | both are about the same subject and cannot both be true |
| `proposed` | a real relation, but you could not verify it in sources |
| `verified` | a real relation AND you read PRIMARY sources that establish it |

**4. Act on the verdict.**

`no-relation` — no graph write.

`duplicate` — pick the survivor: better-sourced first, then more precise label,
then more referenced. Then:
```bash
braim merge-nodes <winner> <loser>
```
If it warns about dependencies only the loser had, **do not** wire them in
yourself; note them in the report for the human.

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

**5. Record it**, always, whatever the verdict:
```bash
braim dream seen <a> <b> --verdict <verdict> --note "<one line>"
```
This is what stops the next session re-treading the same pair.

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

## Report

Finish with a short prose summary the user can read at breakfast:

- pairs adjudicated, and the verdict counts
- every `verified` and `duplicate` with its node ids, since those changed the graph
- anything that looked like a contradiction you were not confident enough to raise
- the relation rate, flagged if it exceeded ~40%
- what to review: `braim list --meta scope=dream`

State plainly if the session found nothing. A night that produces no relations is
a correct outcome, not a failed run.
