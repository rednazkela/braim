# Contradiction Resolution Needs a Third Outcome: Scoped, Not Just Winner/Loser

`braim statement contradict` and its two resolution paths — explicit
`resolve-contradiction` and the automatic three-source Mechanism A — both
assume every raised contradiction is a genuine disagreement about the same
fact, resolvable by picking one side true and invalidating the other. That
assumption is false whenever two statements are both true but describe the
same mechanism under different conditions. Today that case has no correct
outcome: the human either force-picks a loser (wrong) or leaves the
`contradicts` edge unresolved forever.

## What happened (the case that surfaced this)

@[Import Source Discard: on a dedup hit import_graph maps the duplicate to
the existing node id and discards the duplicate node's sources, never
merging them into the target] source: braim ID:179, code:src/graph.rs:3178
(anchor stale; current logic at src/graph.rs:4150-4165)

@[L1 shipped: braim import --full ... unions duplicate sources with status
recompute ...] source: braim ID:235, code:src/graph.rs:3057

A dream session raised `braim statement contradict 179 235`, reasoning that
235's shipped union-of-sources behavior superseded 179's discard claim.

@[union_sources_into is only invoked from three call sites, each gated `if
full && self.union_sources_into(...)`] source: code:src/graph.rs:4164,
code:src/graph.rs:4237, code:src/graph.rs:4323

That gate means both statements are true at once: a **default** import
(`full == false`) still discards a duplicate's sources exactly as ID:179
says; a **`--full`** import unions them exactly as ID:235 says. They
describe different modes of the same function, not a disagreement.

@[The 179/235 contradiction raised earlier this session was premature ...
The two are not mutually exclusive, they describe different modes] source:
braim ID:471, code:src/graph.rs:4164, code:src/graph.rs:4237

## Where the tooling broke

Confirming ID:179 with a corroborating source (`statement add-source 179
--source-id 472`) pushed it to 3 PRIMARY-typed sources. Mechanism A fired
automatically:

```
⚡ Auto-resolved contradiction (Mechanism A):
  Winner ID:179 → proven
  Loser  ID:235 → invalid
```

ID:235 — true, just scoped to `--full` — was invalidated by a mechanism that
never asked whether the two statements actually conflicted. `braim statement
revalidate 235` repaired the node's `verification_status`, but the
`contradicts` edge itself is permanent:

```json
{"from": 179, "to": 235, "resolved": true,
 "resolution_winner": 179, "resolution_source": 472, ...}
```

There is no CLI path to correct this record. `resolve-contradiction` itself
has the identical problem: it only accepts `--winner`, and its documented
effect always sets the loser's `verification_status` to `invalid` and
cascades to dependents — there is no way to tell it "both of these are
right, they just don't overlap."

## Proposed fix

Add a `--both-stand` outcome, mutually exclusive with `--winner`, to
`braim statement resolve-contradiction`:

```
braim statement resolve-contradiction <a> <b> --both-stand \
  --reason "<why they coexist without conflict>"
```

Effect:
- `contradicts` edge → `resolved: true`, `resolution_kind: "scoped"` (new
  field, distinct from today's implicit `resolution_kind: "winner"`),
  `resolution_reason` recorded.
- Neither statement's `verification_status`, `node_type`, or dependents are
  touched. This is the load-bearing difference from today's only path.
- `braim node <id>` prints the resolution kind so a scoped resolution is
  visually distinguishable from a winner/loser one.

And Mechanism A (the auto-resolution inside `add-source` on reaching 3
PRIMARY types) needs a guard: if the statement being corroborated has a
*live, unresolved* `contradicts` edge, do not silently invalidate the other
side. At minimum, print what today's code already prints but **stop short of
mutating the loser** — require an explicit
`resolve-contradiction --winner|--both-stand` call to actually settle it.
Accumulating unrelated corroboration for one side of a contradiction is not
evidence that the contradiction itself has been adjudicated.

## Acceptance criteria

- `resolve-contradiction <a> <b> --both-stand --reason "..."` on a live
  contradiction: both statements keep their pre-resolution
  `verification_status`; the edge is marked resolved with
  `resolution_kind: scoped`; no dependents are touched.
- `--winner` and `--both-stand` are rejected together (clap-level mutual
  exclusion, matching the existing `--set` vs `--add`/`--remove` pattern on
  `update-deps`).
- `add-source` reaching the 3-PRIMARY-type threshold on a statement with an
  unresolved `contradicts` edge no longer auto-invalidates the other side;
  it reports the corroboration and prompts for an explicit resolution call.
- `braim node <id>` output for a resolved contradiction shows
  `resolution_kind` alongside the existing winner/source fields.
- A regression test mirroring this exact case: two statements about the same
  function gated on a boolean flag, both true, corroborating one must not
  invalidate the other.

## Why this matters beyond one pair

@[Fifth anchor-drift instance this session ... in three of five cases the
underlying bug had ALSO already been fixed while the bug-description node
stayed unproven ...] source: braim dream review queue item 8

Constraint-walking and contradiction-raising both run on a per-session
sampling basis — an agent reads two statements, checks sources, and commits
a verdict without necessarily re-deriving every conditional in the code
underneath. That is by design: the alternative is not adjudicating anything.
But it means the "these look contradictory" judgment will sometimes be wrong
in exactly this scoped-truth way, and the graph needs a way to record "this
was investigated and it's not actually a conflict" that does not require
sacrificing one true statement to say so.
