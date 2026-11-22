# The Turn Audit and the Rot Counter: Closing the Loop Between a Careless Turn and a Dream Session

braim already enforces *that* a turn declares what it wrote. It does not
enforce that the turn checked what it wrote against anything. The Stop gate
verifies a tag exists and cites new node IDs; nothing verifies that the
claims behind those IDs were grounded, or that the nodes they lean on are
still standing. Meanwhile the graph is known to rot on its own:

@[Anchor decay reached ID:285 itself: its own cited anchors (graph.rs:3017
for register_gap's duplicate check, :3924 for the dedup key) have since
drifted again -- register_gap now sits at src/graph.rs:3203 and the dedup
key construction at src/graph.rs:4147/4219, in a file that grew from 5080 to
5832 lines since 285 measured it on 2026-08-08 -- so this graph's
line-anchored evidence decays on an ongoing basis, not as a closed
two-instance list] source: braim ID:461 (fact, proven)

Three distinct drift instances are on record (ID:277, ID:285, ID:333). Each
was found by a human or an agent happening to reread the anchor, never by
the system noticing. Dreaming is the mechanism that *would* notice, but
today it fires only when someone remembers to ask for it.

This proposal closes that loop: every turn audits, findings accrue in a
durable counter, and the counter firing is what schedules the dream.

## 1. The countable unit is a defect in a stored node

The obvious implementation — increment on every audit flag — inverts the
signal it is meant to carry.

A turn that catches its own unsupported claim before asserting it is a turn
where the audit *worked*. The claim never entered the graph. Nothing rotted.
Incrementing a rot counter on that event means the counter climbs fastest
when the agent audits hardest, and sits at zero when the agent skips
auditing entirely. That is the same self-selection failure the verification
discipline already has to guard against, now with a numeric incentive
attached: an agent that knows five findings triggers a dream session has a
standing reason to find four.

**A finding is a defect discovered in a node that is already stored.** Not
a correction made in flight. The distinction is mechanical, not a judgement
call — a finding names an existing node ID, an in-flight correction does
not.

Four countable kinds, each already a documented failure mode in this graph:

| kind | what it means | precedent |
|---|---|---|
| `anchor` | a cited `code:`/`doc:` anchor no longer holds what the node says it holds | ID:461, ID:463 |
| `reground` | a node's label disagrees with the document it cites | doc:policies/memory_braim_traits.md:19-21 |
| `independence` | a promotion rests on sources that are not independent of each other | `braim-source-independence-and-citation-volatility.md` Gap 1 |
| `unsupported` | a node carries a verification status its current sources no longer justify | — |

In-flight self-corrections are still declared in the turn tag, for the
record. They do not increment.

## 2. The count lives in reviews.json, not current.json

The request as stated puts the counter in `current.json`. Two things block
that, and a third makes it unnecessary.

@[current.json top-level keys: because_of, contradicts, dictionary, gaps,
id_to_domain, next_id, nodes, version] source: `jq -r 'keys[]'
.braim/current.json`

There is no counter slot, so this is a new field on the serialized graph
struct — a binary change to add, and a schema change every existing graph
file has to tolerate on load.

@[Never use jq or other tools directly on current.json] source: `braim
--help`, REQUIRED RULES rule 2

So the Stop hook cannot increment it in the shell. Every increment needs a
new `braim` write command, and every increment takes the write lock on the
whole graph on a path that runs after literally every turn.

Neither cost buys anything, because the storage already exists:

@[The queue lives in reviews.json beside the graph. Read it with `braim
dream review`, sign an item off with `braim dream reviewed <id>`.] source:
`braim dream flag --help`

@[This is what to read after an unattended night. It survives context
compaction because it is a file beside the graph, not prose in a report.]
source: `braim dream review --help`

@[{"id":1,"kind":"note","note":"reweighed 453: 445 0.5 to 0.8, 446 0.2 to
wait","nodes":[453,445,446],"raised_at":"2026-08-15T01:00:28Z","cleared_at":"2026-08-15T01:00:37Z","cleared_note":"typo
in the note text, superseded by items 2 and 3 below"}] source: `jq -c '.[0]'
.braim/reviews.json`

Every field the counter needs is already there: a stable id, a kind, the
node IDs implicated, a raised timestamp, and a cleared timestamp that makes
the count self-limiting rather than monotonic.

**The findings count is `braim dream review` filtered to the four audit
kinds, pending only.** It is a query, not a field. Per-graph scoping is
inherited for free, because `reviews.json` already sits inside each graph's
`--data-dir` — which is what "each braim current.json tracks its own" was
asking for.

**Change required**: `braim dream flag --kind` currently accepts @[merge |
unraised | duplicate | rate | note] source: `braim dream flag --help`. Add
the four audit kinds, and add a `--count` mode to `braim dream review` that
prints a pending tally a hook can read without parsing JSON.

## 3. The turn audit is a declaration, not a detection

The hook cannot judge whether a turn audited itself. This is settled design
in this repo, stated in the gate's own header:

@[Design: mandatory-declaration, not detect-and-verify. A dumb shell script
cannot judge "was this turn substantive enough to need a braim write" -
that's a semantic call only the model can make reliably. So this hook
doesn't try. It requires the model to make that call itself, EVERY turn, and
emit the result as a small structured tag] source:
code:hooks/braim_stop_gate.sh

"Did this turn's output survive a check against its sources" is a strictly
harder semantic call than the one that header declines to make. The same
answer applies: the model declares, the hook verifies mechanically what is
mechanically verifiable.

So the tag grammar extends. Today:

```
[braim: logged 517,518]
[braim: none]
```

Proposed, with an audit verdict as a second clause:

```
[braim: logged 517,518 | audit: clean]
[braim: logged 517,518 | audit: 2 flagged 66,71]
[braim: none | audit: skipped: no claim leaned on a stored node]
```

What the hook can check without semantics:

- the audit clause is present and well-formed, or Stop is blocked (same
  omission-closing move the `logged` clause already makes)
- every ID cited after `flagged` resolves via `braim node <id>` and is a
  node that already existed *before this turn* — a finding by definition
  names a stored node, so a fresh ID in that position is a category error
  and is rejected
- the flagged count matches the rise in pending audit-kind items in
  `reviews.json` since the last check — a turn claiming two findings that
  filed none is rejected, which is what stops the clause degrading into a
  cosmetic string

The hook's existing state file already carries the high-water mark this
needs: @[{"last_seen_max_id": 519}] source:
`~/.claude/braim_stop_state/<cwd_hash>.json`. A `last_seen_pending` field
sits beside it.

`audit: skipped` is a legitimate verdict and must stay cheap to emit. A turn
that asserts nothing resting on a stored node has nothing to audit, and
forcing a fabricated verdict out of it is how the clause starts lying.

## 4. The threshold fires by blocking, because that is the only lever

@[Commands: candidates, constraints, whatif, flag, review, reviewed, log,
seen] source: `braim dream --help`

There is no `braim dream run`. Dreaming is a skill at
`.claude/skills/dream/SKILL.md` that drives those read-only primitives; a
shell script cannot invoke it, and giving the binary the power to spawn an
LLM session is a much larger change than this proposal wants.

The gate already owns the one mechanism that makes a model do something:

@[Missing tag, malformed tag, or a "logged" claim citing IDs that don't
exist or aren't new since the last check -> exit 2, blocking Stop and
forcing another turn] source: code:hooks/braim_stop_gate.sh

So the trigger is the same mechanism with a second predicate. After the tag
check passes, the hook reads the pending audit-kind count. At or above the
threshold it exits 2 with an instruction to run the dream skill against the
flagged node set — and, critically, names the specific pending item IDs, so
the session that follows is scoped to the rot that accumulated rather than
being an open-ended walk.

The threshold is configurable and defaults to 5, per the request. It belongs
next to the central path braim already records at `init` time, not hardcoded
in the script.

## 5. Clearing is what keeps the counter honest

A monotonic counter fires once and then either resets to zero (losing the
record) or fires forever (losing the signal). `reviews.json` already has the
field that avoids both: `cleared_at`.

- the count is **pending items only**; a signed-off item stays in the file
  for audit and stops counting
- a dream session that resolves a finding calls `braim dream reviewed <id>`,
  which is the existing sign-off path — no new clearing mechanism
- a finding a session examines and dismisses is signed off with a
  `cleared_note`, same as today
- the counter therefore falls as a direct consequence of the work the
  trigger demanded, and the next threshold crossing measures rot accumulated
  *since* that session

## Summary of proposed changes

1. Add four audit kinds (`anchor`, `reground`, `independence`,
   `unsupported`) to `braim dream flag --kind`. A finding names an
   already-stored node; in-flight self-corrections are declared but not
   counted.
2. Add `braim dream review --count` emitting a pending tally per kind, so a
   hook reads the counter without parsing `reviews.json`.
3. Extend the Stop tag grammar with an `audit:` clause (`clean` /
   `N flagged <ids>` / `skipped: <reason>`), mandatory-declaration in the
   same shape as the existing `logged` clause.
4. Extend `braim_stop_gate.sh` to verify the audit clause: well-formedness,
   flagged IDs resolve and pre-date this turn, and the claimed count matches
   the rise in pending audit items. Track `last_seen_pending` beside
   `last_seen_max_id`.
5. Extend the same hook to exit 2 when pending audit findings reach the
   threshold (default 5, configurable), instructing the model to run the
   dream skill scoped to the pending item IDs.
6. No change to `current.json`, to the verification lifecycle, or to the
   `MIN(source-derived, weakest dependency)` status math.

## Open questions

- **Is `audit: skipped` load-bearing enough to abuse?** It is the pressure
  valve that keeps the clause honest, and it is also the obvious escape
  hatch. The hook cannot check a skip reason semantically. One mitigation
  worth testing: reject `skipped` on any turn whose `logged` clause is
  non-empty, since a turn that wrote to the graph did lean on it.
- **Does the threshold trigger interact badly with a long unattended run?**
  A session that trips the threshold, dreams, and keeps working can trip it
  again in the same session. A per-session fire-once guard is the obvious
  answer; whether that hides real accumulation is not settled.
- **Where does the threshold live?** `braim init` already records the
  central path. Whether it grows a general config surface or the threshold
  rides an environment variable is undecided.
