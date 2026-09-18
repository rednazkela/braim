---
name: dream
description: Run one overnight dreaming session over a local braim graph — adjudicate candidate node pairs for missing relations, duplicates, and contradictions, relax load-bearing constraints to find the ones the graph has already outgrown, and retune existing compounds' weights where rereading their sources justifies it, writing every verdict back under a skeptical evidence protocol. Use when the user asks to dream, run a dream session, ask what-if, or consolidate a graph overnight.
---

# Dreaming: adjudicate candidate pairs

braim's `dream candidates` picks node pairs worth examining. This skill is the
other half: reading each pair, deciding whether a relation is *real*, and writing
the verdict back. braim does structure; you do judgment.

`$1` (optional) = how many pairs to adjudicate this session. Default **15**.
`$2` (optional) = data dir. Default: the project's `.braim`.

`$1` = **`weights`** or **`full-weights`** switches modes entirely: skip
pairs and what-if, and run only the full-graph weight sweep described under
"Full-graph weight sweep" below, over every equal-split compound in the graph
regardless of when it was created. This is the explicit-request trigger that
section requires — an ordinary `/dream` (no arg, or a number) never runs it.

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

Read the mechanical report BEFORE generating candidates. `braim audit` computes
six worklists with no judgement at all, and dreaming used to invoke it nowhere —
so every pass re-derived by hand what was already printed, and two of its
sections had no reader whatsoever:

```bash
braim audit                                       # the mechanical report — run this first
braim dream candidates --limit <N> --json         # the worklist
braim dream candidates --strategy semantic --json # highest precision first
```

What each audit section is worth to a session:

| section | use it for |
|---|---|
| Orphan nodes | pair candidates the generators structurally cannot surface — an unreferenced node is far from everything, so semantic and two-hop both miss it |
| Pending nodes | declared and unintegrated; either wire them or say why not |
| Gap register | zero-path pairs already registered as worth investigating — a candidate list somebody else wrote |
| Deprecated still referenced | a live citation of something retired |
| Refuted causal links | the output of causal re-grounding, and the input to the next one |
| Statements flagged for re-investigation | **a standing worklist, see below** |

`braim audit` exits 0 whatever it finds, so read its sections; do not test its
status.

**The orphan row is not theoretical.** A platform-design run produced six
sibling frames under one parent and `--strategy semantic` returned zero
candidates touching any of them — the generator excludes pairs closer than two
hops, and siblings sharing a parent are exactly that. Audit saw them; the
candidate generators could not.

Spend the candidate budget **semantic first, then shared-source, then two-hop**:
measured on a real 714-node graph those yielded 30 / 148 / 5135 candidates
respectively, so two-hop is a reserve, not a starting point. The exception is a
freshly authored frame set, where shared-source goes first — see the causal
re-grounding and label-shape passes for why proximity inverts there.

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
   braim sources add "<what was counted>" --type test \
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

Pairs are one part of a session. Another is asking what the graph would look
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
braim meta <constraint> --set whatif_walked_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
```

Use the full timestamp, not a bare date. The reopen check compares lexically
against each candidate's `created_at`, and `2026-08-11T09:00:00Z` sorts ABOVE
`2026-08-11` — so a date-only mark reopens on everything written that same day,
including what you just read (braim ID:353).

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

## Weight tuning: retune existing compounds, add nothing

A dream can also revisit a compound already in the graph and ask whether its
`depends_on` split still matches what the sources say — no new pair, no new
statement, just rereading what is already cited. This is the sleep-consolidation
move: the finding was already made; this pass only adjusts how much of it each
dependency is carrying.

**Default scope is today's new nodes, not the whole graph.** Run it after the
pairs (and after what-if, if you ran that too), and keep it to a handful — two
or three compounds a session, same budget as `whatif`. Build the candidate list
from what today actually added, not from every equal-split compound the graph
has ever accumulated:

```bash
python3 -c "
import json, datetime
d = json.load(open('.braim/current.json'))
today = datetime.datetime.utcnow().strftime('%Y-%m-%d')  # or pass today's date explicitly
for nid, n in d['nodes'].items():
    if not n.get('created_at', '').startswith(today):
        continue
    deps = n.get('depends_on') or {}
    if len(deps) < 2:
        continue
    print(nid, n.get('node_type'), dict(deps), n.get('label', '')[:100])
"
```

This is a scoping choice, not a shortcut: a graph accumulates multi-dependency
compounds across every session it has ever had, and re-litigating all of them
every night turns a bounded nightly pass into an unbounded standing task. Today's
new nodes are the ones nobody has looked at yet — the same reasoning `frontier.py`
applies to pairs applies here. A full-graph sweep is a real, separate, valuable
thing to do (see below) — just not automatically, every night.

**Every existing split is a candidate, symmetric or not — a weight is not
evidence just because it is already asymmetric.** The original version of this
check only flagged even splits (0.5/0.5, or an even share across more than two),
on the theory that an even split is what a statement gets when nobody has
stated an opinion. That theory only covers one failure mode. Nothing in this
graph verifies that an *asymmetric* split was ever checked against its sources
either — `update-weights`/`update-deps` accept any number that sums to 1.0, and
several of the asymmetric splits fixed this session (422, 429, 434, 448, 452)
were themselves wrong until read closely. An unverified 0.73/0.27 is exactly as
untrustworthy as an unverified 0.5/0.5; treat every existing number as a claim
nobody has re-grounded yet, not as a settled fact.

**Per compound:**

1. `braim node <id>` — read the current split and what it depends on. Note the
   split, but do not treat it as informative either way — it's the thing being
   checked, not evidence for or against itself.
2. `braim node <dep-a>`, `braim node <dep-b>` (and any others) — read each
   dependency's own label, sources, and verification status.
3. Open the actual cited sources — Read/Grep, not the labels. The question is
   narrow: does the compound's claim rest more on what one dependency's source
   demonstrates than the other's? A dependency that is `proven_strong` where the
   other is `partial`, or whose source is what the compound's own wording is
   actually describing, is carrying more of the claim. Derive what the split
   *should* be from this reading alone, before looking back at what it
   currently is.
4. **If you cannot point to the specific sentence or line that justifies a
   different split from what you derived in step 3, leave the existing number
   alone** — whether that number is 0.5/0.5 or 0.9/0.1. The default is
   **no-change**, for the same reason the default pair verdict is `no-relation`:
   a plausible-sounding number is exactly what an LLM asked to reweigh something
   will produce whether or not the evidence asymmetry actually changed since the
   split was set. This cuts both ways now — don't "fix" an asymmetric split
   into a different asymmetric split just to have done something; most existing
   splits, once checked, will turn out fine.
5. If the sources do justify a change, write it — never by fiat, always citing
   what you read:
   ```bash
   braim statement update-weights <id> --weights "<dep-a>:0.7,<dep-b>:0.3"
   # or, if the compound is a concept rather than a statement:
   braim concept update-weights <id> --weights "<dep-a>:0.7,<dep-b>:0.3"
   ```
6. Mark that it was looked at, on the compound itself — not `scope=agent_scratch`.
   That tag drops a node out of `eligible()` entirely, and this compound is
   established knowledge, not something dreaming wrote tonight; excluding it
   would silently pull it out of constraint ranking and future dreaming for
   good. `whatif_walked_at` sets this precedent already — a plain metadata mark,
   not a scope change:
   ```bash
   braim meta <id> --set reweighed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
   ```
   Note the before/after split and the specific evidence in the closing report.
   Flag it for human review if the change reverses which dependency carries the
   majority — that is a structural change a human should see, not just read
   about after the fact:
   ```bash
   braim dream flag "reweighed <id>: <dep-a> <old> to <new>, <dep-b> <old> to <new> - <why>" \
     --kind note --nodes <id>,<dep-a>,<dep-b>
   ```

`reweighed_at` exists for the same reason `whatif_walked_at` does: without a
mark, any multi-dependency compound looks like a candidate every single night
even when nothing about its sources has changed since the last look.

### Full-graph weight sweep — on request only, never part of the nightly default

Dropping the today-only filter and scanning every multi-dependency compound
the graph has ever accumulated is a real mode, not a bigger version of the
same mode. On a ~460-node graph this candidate list is **359** nodes once the
equal-split filter is dropped (versus 38 when it only caught even splits) — an
order of magnitude bigger, because most of a graph's compounds are already
asymmetric and none of them were exempt from being wrong. Some of that 359 is
old demo/test/spec-bootstrap fixtures with no real domain content to reason
about (labels like `TestB TestC` or May-era `Voice Charge` demo data — check
`braim query` or the node's own domains for this before spending time on it,
and skip anything that reads as fixture rather than fact); the rest is a real,
multi-session audit, not a single sitting.

Run this **only when invoked with `$1 = weights` / `full-weights`, or when the
user explicitly asks for a full weight sweep or audit** — never as a default
step of an ordinary `/dream`, and never just because the today-only candidate
list came up empty. An empty today-only list is a legitimate "nothing to
reweigh tonight," the same as an empty pair list. This mode is standalone: it
does not run pairs or what-if first.

```bash
python3 -c "
import json
d = json.load(open('.braim/current.json'))
for nid, n in d['nodes'].items():
    if n.get('status') != 'active':
        continue
    if n.get('metadata', {}).get('reweighed_at'):
        continue
    deps = n.get('depends_on') or {}
    if len(deps) < 2:
        continue
    print(nid, n.get('node_type'), n.get('verification_status'), dict(deps), n.get('label', '')[:100])
"
```

`reweighed_at` is what makes this tractable across more than one invocation:
every node checked — reweighed or deliberately left alone — gets it set (per
step 6 above), the same way `whatif_walked_at` lets constraint-walking advance
instead of re-offering the same list. A full sweep is a ledger to work down
across sessions, not a single unbounded pass; treat one invocation's worth
(however many you get through with real per-compound reading, not a rubber
stamp) as a session's progress, and let the next full-weights invocation pick
up where this one left off. Don't skip the mark on a fixture node just because
it took ten seconds to dismiss — an unmarked node looks like a fresh candidate
forever, same failure mode `braim ID:1282` already documents for the pair
pre-pass.

Weights never affect verification status — status is source-type diversity
capped by the weakest dependency's status, not the depends_on split — so
retuning a split cannot promote or demote anything. It changes how much of a
compound's identity each dependency is read as carrying, nothing else.

## Label shape: one claim per statement, on an absolute threshold

A statement's label is what `braim query` returns and what `statement
contradict` operates on. Both of those break the same way when the label
carries several claims instead of one: retrieval pays for prose the graph
exists to compress, and a node asserting five things can only be contested
wholesale, dragging its true sentences down with its false one. That is the
Boolean-conjunction gap (braim ID:2352) showing up as a data problem rather
than a modelling one — a paragraph label IS an implicit AND with no way to
address its parts.

Run this **after** the pairs, in the same slot as weight tuning, and keep it to
the same budget of two or three:

```bash
python3 <skill-dir>/label_shape.py .braim 10
```

The threshold is **absolute: more than 400 characters AND more than two
sentences**. It used to be the graph's own p90, and that was wrong in a way
worth keeping written down — a relative threshold is self-defeating on a
drifting graph, because every paragraph written raises p90 and raises the bar
for catching the next one. Measured on profserv it offered 6 of 49 recent
nodes while their median sat below the threshold it computed (braim ID:2375).

The absolute numbers are derived, not picked. This graph's founding convention
over its first 479 statements was **one sentence and at most 407 characters,
without a single exception**, so >2 sentences is already twice the widest thing
that convention ever produced. The script prints that founding measurement on
every run so the threshold stays auditable against the graph it is applied to.
Nodes already carrying `label_shape_at` are withheld, the same way
`whatif_walked_at` and `reweighed_at` withhold their own.

**Per node offered:**

1. `braim node <id>` — read the whole label.
2. **Count the claims, not the sentences.** A long label stating ONE claim with
   its figures inline is not a violation; four sentences elaborating a single
   assertion are fine. The violation is a label a reader could split into parts
   that would need separate contradiction.
3. If it is one claim, mark it and move on — that is the expected outcome and
   costs nothing:
   ```bash
   braim meta <id> --set label_shape_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
   ```
4. If it is several claims, flag it and mark it. Dreaming may not edit a
   statement's text, so decomposition is the human's call, not yours:
   ```bash
   braim dream flag "ID:<id> carries <n> separable claims in one label: <name them>. Decomposing needs <k> statements with the evidence moved to typed sources" \
     --kind note --nodes <id>
   braim meta <id> --set label_shape_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
   ```

**This is `note`, never audit rot.** The four audit kinds are about a claim's
grounding — whether its source says what it says, whether that source could
have refuted it. Label shape is about form, and a badly shaped label can be
perfectly well grounded. Filing it as `unsupported` inflates the rot counter
with something no evidence check can clear (braim ID:2358).

Two traps, both measured:

- **Do not cherry-pick the baseline, and do not let the drift set it either.**
  Comparing recent writes against an early slice makes drift look like a
  personal regression; comparing them against the whole graph lets the drift
  hide inside its own average. The founding convention is the fixed point, and
  the count that matters is per-month: 177 of 189 over-threshold nodes came
  from 2026-08 (braim ID:2358), which no relative threshold would have shown.
- **Long labels help dedup, they do not hurt it.** `braim similar` on a full
  draft label surfaced a duplicate at 0.860; the one-sentence distillation of
  the same claim scored 0.555 and missed it entirely. Shortening the label is
  the right goal for retrieval and contradiction, and the wrong lever for
  duplicates — run the dedup check on the draft text, never on a summary of it
  (braim ID:2359).

## Causal re-grounding: test the edges, never only add them

Every other pass reads because_of and none repairs it. Dreaming's own procedure
names `why-add` twice and `why-test` and `why-remove` not at all, so a wrong
parent it meets is a parent it keeps — while `constraints` ranks by how many
statements rest on a node through those same edges, nine and ten levels deep.
A causal chain built cheaply therefore does not just sit in the graph: it
inflates whatever sits at its head into a load-bearing constraint, which is the
exact thing what-if exists to find.

This is the repair pass. It is the only mode that touches an existing edge, and
like weight tuning it defaults to leaving things alone.

**Scope is the same as weight tuning's**: two or three edges a session, run
after the pairs. A full sweep is a real thing to do on request, not nightly.

```bash
python3 <skill-dir>/suspect_edges.py .braim 10
```

The ranking is one discriminator with two corroborators, and the discriminator
was measured before it was trusted. When an author writes "this happened
because of that", the cause is usually also one of the reasons the node exists,
so the parent appears in the child's `depends_on` — true for 303 of 340 edges
on this graph. The other 37 assert a cause while the node is built out of
something else entirely. Those rank; the rest are withheld.

**What was tried first and rejected, because the same trap waits for anyone
who re-derives this.** Ranking on id gap and shared creation hour — the shape
of writing a parent immediately before its child — flagged 282 of 340 edges,
and its top three hits were genuine sequential reasoning chains. A real cause
very often IS written just before its consequent. Proximity marks the normal
shape of thinking, not a defect, and a ranker that flags 83% of a graph has
told you nothing. Both signals survive only as corroboration on an edge already
suspect for standing outside `depends_on`.

**Per edge:**

1. `braim node <child>` and `braim node <parent>` — read both, and read the
   child's dependency set. The question is narrow: does the child still hold
   if the parent had never been true?
2. Run the test braim already implements, and record it either way:
   ```bash
   braim why-test <child>          # the cause is confirmed
   braim why-test <child> --fail   # the consequent stands without it
   ```
   A PASS is a result. It lands in `test_source`, withholds the edge from every
   future run of this pass, and is the outcome you should expect most often.
3. Only on a FAIL, reassign. **A failed test already marks the link invalid**
   — it prints "causal link marked invalid; statements unchanged" and the edge
   drops out of every later pass on its own, so `why-remove` is NOT a step here.
   That command is for detaching a link you want to reassign without having
   refuted it, which this pass never does.
   ```bash
   braim why-add <child> --because <the real cause> --source "narrative:regrounded-<YYYY-MM-DD>"
   # or, when there is no cause and the node is an observation:
   braim meta <child> --set terminal_cause=true
   ```
   **Do not invent a parent to replace the one you removed.** A node with no
   real cause is terminal, and `terminal_cause=true` says so honestly. Reaching
   for the next plausible node is how the original bad edge was written.
4. Mark it, whatever the outcome:
   ```bash
   braim meta <child> --set causal_regrounded_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
   ```
   Use the full timestamp, for the same reason `whatif_walked_at` does.

**Flag rather than reassign when the real parent is not in the graph.** A
consequent whose actual cause was never written is a gap, not a bad edge:

```bash
braim dream flag "<child> has no cause in the graph; its stated parent fails the inverse test" \
  --kind anchor --nodes <child>,<parent>
```

`braim audit` reports refuted links under "Refuted causal links (because_of
failed inverse test)" — so a FAIL you record is visible to every later session
without needing this pass to run again.

Report the edges tested, the verdicts, and the reassignments separately from
the no-changes. As with weights, **no-change is the expected outcome** and a
session that tested three edges and confirmed all three has done the work.

## Anything a human must see goes in the review queue

The closing report lives in the model's context and does not survive
compaction, so anything that exists only there is lost by morning. A night run
unattended must leave its review items on disk:

```bash
braim dream flag "<what a human should look at>" --kind <k> --nodes <ids>
```

`--kind` is one of `merge`, `unraised`, `duplicate`, `rate`, `note`.

**Flag it the moment it happens**, not at the end. These are the cases — every
one of them used to be report-only:

- `merge-nodes` warned about dependencies only the loser carried. Never wire
  them in yourself; flag them with both ids.
- A merge destroyed a label that held detail the winner lacks. Flag what was
  lost.
- Something looked like a contradiction and you could not ground it well enough
  to raise. Flag it rather than dropping it — being unsure is the point.
- The relation rate exceeded ~40%, or a graph looks exhausted for a mode.
- Any judgement call you would want a second opinion on.

Nodes need no flag: they are already findable with `braim list --meta
scope=dream`. Flag the things that are **not** nodes.

## Clearing the re-investigation backlog

`braim dream flag` is not the only queue. When a cause is invalidated, braim
marks every statement that rested on it and prints them under **"Statements
flagged for re-investigation (cause invalidated below)"**. Those entries are a
worklist with the same shape as audit rot and, until this section existed, no
reader at all: nine sat on this graph citing a single invalidated cause, some
for weeks, because the flag is raised automatically and cleared by nobody.

This backlog GROWS from dreaming's own work. Causal re-grounding refutes links
on purpose, and every refutation marks that link's dependents. A pass that
produces flags and never clears them is a pass that manufactures debt.

**Per flagged statement:**

1. `braim node <id>` and read the cause that was invalidated. The flag says the
   ground moved; it does not say the statement is wrong.
2. Ask the narrow question: **does this statement still hold on its own
   sources?** A statement whose cause was refuted but whose own evidence stands
   is fine — it was mis-parented, not mistaken.
3. Act on the answer:
   - **holds on its own sources** → re-parent it to the real cause, or mark it
     `terminal_cause=true` if it has none. Same rule as causal re-grounding:
     do NOT invent a parent to fill the hole.
   - **held only because its cause held** → `braim statement invalidate <id>
     --reason "<the cause it rested on was refuted, and its own sources do not
     carry it>"`. This is the case the flag exists for.
   - **cannot be settled from this checkout** → `braim dream flag` it with kind
     `anchor` and say what evidence would settle it.
4. Clear it, and record what you decided. **braim raises this flag with a
   metadata key of its own, `because_of_reinvestigate`, and `braim audit` reads
   THAT key — not any marker a skill invents.** Setting a `reinvestigated_at`
   alone leaves the count exactly where it was; verified by doing it and
   watching nine stay nine.
   ```bash
   braim meta <id> --set reinvestigated_note="<what you decided and why>"
   braim meta <id> --set reinvestigated_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
   braim meta <id> --unset because_of_reinvestigate    # this is what clears it
   ```
   The note and the timestamp are for the next reader; the unset is what lowers
   the count. Do the unset LAST, so an interrupted session leaves the flag up
   rather than clearing an item it never decided.

   This differs from every other withholding marker in this skill —
   `whatif_walked_at`, `reweighed_at`, `label_shape_at`, `causal_regrounded_at`
   are read by the skill's own scripts, which is why inventing them works. This
   queue is braim's, so clearing it is braim's key.

**Budget it like the other passes**: two or three a session. A backlog that
accumulated over weeks does not need clearing in one night, and the statements
at the bottom of it have been wrong-or-fine for that long already.

**Expect "still holds" to be the common verdict.** An invalidated cause is
evidence about the EDGE, not about the node. Treating the flag as a verdict on
the statement is how a correct finding gets invalidated for its parent's sins —
the same collateral damage that multi-claim labels cause, arriving by a
different route.

## Clearing the audit-rot queue

`braim dream flag` takes two families of `--kind`, and the five above are only
one of them. The other four are **audit rot**, raised by the per-turn audit
rather than by a night run:

| kind | what was found |
|---|---|
| `anchor` | a node's own state is wrong — stale status, a tie that no longer holds |
| `reground` | the node's label disagrees with the source it cites |
| `independence` | a claim rests on a source that cannot refute it |
| `unsupported` | a claim carries no source that establishes it |

`braim_stop_gate.sh` blocks at `BRAIM_AUDIT_THRESHOLD` (default 5) with *"Run a
dream session now, scoped to review items: …"*. That is this procedure, and it
is the only thing that lowers the counter.

**The clearing procedure, per item:**

1. `braim dream review` — read the item and the node ids it names.
2. **Gather evidence before deciding.** Re-read the SOURCE the node cites, not
   the node's label. A label is a pointer; it can carry an upstream error, and
   re-citing it is how a wrong figure survives.
3. Decide, and act:
   - label disagrees with its source → fix the node (`braim meta <id> --set
     correction="…"`, or `statement invalidate` when the claim itself is refuted)
   - the item is right but the node is superseded → record `superseded_by`
   - the item does not hold → say so in the note; a finding you did not act on
     is not tidied away by resolving it
4. `braim dream reviewed <id> --note "<what you decided and why>"`

**Delegate the gathering, never the deciding.** Step 2 is open-the-file-and-quote
work: mechanical, verifiable in one command, and carrying no authority over what
the evidence means. Spawn ONE `dream-probe` (haiku) at the start of the session
and message it per item rather than spawning per item — a spawn costs ~42.6k
tokens, a follow-up message ~681. Steps 3 and 4 stay with you: they change the
graph, so they belong to the tier that will be held to them.

A probe returns quoted text and an `agree: yes|no|NOT FOUND`. If it reports
NOT FOUND, that is evidence, not a failure to try — treat a missing citation as
the finding.

Reading the queue back, on any later session or after any compaction:

```bash
braim dream review                    # pending, oldest first
braim dream review --all              # including what was already signed off
braim dream reviewed <id> --note "<what you did>"
```

Cleared items are kept rather than deleted — what a human decided is itself
worth keeping.

The pair verdicts are durable too, and now readable:

```bash
braim dream log --limit 20            # newest first, with the adjudicator's note
braim dream log --since 2026-08-10
braim dream log --verdict verified
```

## Rules that keep the graph sound

- Weights in `--depends` must sum to 1.0 and should be **asymmetric** — equal
  weights assert no opinion about which node carries the relation.
- A statement must express a relationship between both nodes. A sentence that
  only elaborates one of them is not a relation.
- Never `--force`, never delete a node, never resolve a contradiction, never
  edit an existing statement's text. Dreaming adds hypotheses and consolidates
  duplicates; it does not rewrite established knowledge.
- `update-weights` and the causal-edge trio (`why-test`, `why-remove`, then a
  replacement `why-add`) are the only mutations of existing structure dreaming
  is allowed. Neither touches a statement's text. A failed `why-test` invalidates
  the link itself, so `why-remove` is never needed after one; detaching a cause
  you have not tested is the same unforced judgement the pass exists to undo,
  and dreaming does not do it.
  It changes a `depends_on` split, never a statement's text, and only through
  the cited-evidence procedure in "Weight tuning" above — never a bare number
  chosen because it "feels" more balanced now than it did when it was written.
- Everything you create carries `scope=dream` so a human can review the whole
  session with `braim list --meta scope=dream`.
- **Bookkeeping about the session itself carries `scope=agent_scratch` instead** —
  a note that you merged two nodes, rewired a dependency or resolved a
  contradiction. It is a record of what you did, not a finding about the domain.
  Untagged, it becomes an ordinary statement: give it a `why-add` and it starts
  ranking as a load-bearing cause, so the session's own housekeeping competes
  with real constraints. Three such nodes made up 3 of 11 ranked causes on
  profserv before they were tagged (braim ID:354). `eligible()` already excludes
  `agent_scratch`; the tag is the whole mechanism, so the only failure mode is
  forgetting it.
- A what-if output additionally carries `counterfactual=true`, and nothing else
  ever does. Tagging an ordinary finding that way removes it from dreaming and
  from export for good; leaving it off a hypothesis lets a hypothesis publish as
  a finding, and lets the next night pair against it as though it were true.
- `counterfactual=true` takes a node — and everything depending on it — out of
  the dreaming pool entirely. It will not be offered as a pair, will not count
  toward any constraint's leverage, and cannot itself be walked. That is what
  stops a session from compounding its own hypotheses.

## Report

Finish with a short prose summary the user can read at breakfast. It is a
convenience, **not** the record — everything below that matters must already be
in the graph, the ledger, or the review queue before you write a word of it:

- the pre-pass: which nodes you measured, the command and result for each, and
  whether the measurement confirmed or refuted the label
- pairs adjudicated, and the verdict counts
- every `verified` and `duplicate` with its node ids, since those changed the graph
- anything that looked like a contradiction you were not confident enough to raise
- the relation rate, flagged if it exceeded ~40%
- constraints walked: which ones, whether each turned out stale, and for the ones
  that held, the single change you named — separate the stale findings from the
  counterfactuals, because only the first kind is a finding
- compounds reweighed: which ones, the before/after split, and the specific
  evidence that justified each change — and how many candidates you looked at
  but left alone, since no-change is the expected outcome here too
- label shape: how many nodes the check offered, how many held one claim and
  were simply marked, and every node flagged for decomposition with the claims
  you could separate out of it
- the audit read at Setup: what each section held going in, so a section that
  grew during the session is visible against where it started
- re-investigation flags cleared: which statements, and whether each still held
  on its own sources or fell with its cause — those are different outcomes and
  a count of "cleared" hides which one happened
- causal edges tested: which ones, the verdict on each, and any reassignment —
  kept separate from the confirmations, since a PASS is the expected outcome
- what to review: `braim dream review` first — it is the only place the
  report-only observations live — then `braim list --meta scope=dream` for the
  nodes, and `braim list --meta counterfactual=true` for the hypotheses, which
  never leave this graph

State plainly if the session found nothing. A night that produces no relations is
a correct outcome, not a failed run.
