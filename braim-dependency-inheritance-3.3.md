# BRAIM_DEPENDENCY_INHERITANCE_SPEC §3.3 — Refutation propagation

Drafted 2026-08-10 against `src/graph.rs` at the working tree.

## Implementation status (2026-08-10)

**Landed** — §3.1 and §3.2:

| Change | Site |
|---|---|
| `rank()` documented as an ordering, not a support ladder | `graph.rs:238-262` |
| `from_cap_rank` added — clamps rank 0 to `Unproven`, single enforcement point for "a cap can never yield Invalid" | `graph.rs:275-288` |
| `is_refuted()` added | `graph.rs:290-294` |
| Creation-time inheritance: refuted deps excluded from the cap set instead of assigning `Invalid`; `support_withdrawn_by` / `_at` recorded | `graph.rs:2624-2680` |
| Recompute on `update-deps`: refuted deps excluded, `from_cap_rank` | `graph.rs:958-975` |
| Recompute helper: refuted deps excluded, `from_cap_rank` | `graph.rs:4354-4372` |
| `from_cap_rank` in the already-correct recompute path | `graph.rs:3752` |

`cargo build` clean (one pre-existing `dead_code` warning on `BootstrapReport`).
`cargo test` 90/90 pass. `readers_never_observe_an_inconsistent_shard_set` in
`tests/concurrency.rs` is flaky on timing — it failed on both the patched and the
unpatched baseline in one run and passes in another; not a regression.

Verified behaviour on an isolated graph:

- child with two PRIMARY source types, 0.4 on a refuted dep and 0.6 on a
  surviving one → `proven`, `invalid: false`, `support_withdrawn_by: 3`
- child whose only statement support is refuted, weakly sourced → `unproven`,
  not `invalid`
- attaching a measured `test` source to that node promotes it to `partial`
  normally — withdrawn support does not poison a node against future evidence
- control: a `narrative`-only parent still caps a two-PRIMARY-type child to
  `unproven`, so ordinary capping is unchanged

**Landed** — §3.3:

| Change | Site |
|---|---|
| `recompute_after_refutation` added — recomputes direct dependents, records withdrawal, never writes `Invalid` | `graph.rs:4339-4413` |
| `append_csv_meta` helper | `graph.rs:4415-4428` |
| Auto-resolve (Mechanism A) cascade replaced | `graph.rs:1349-1353` |
| `resolve_contradiction` cascade replaced | `graph.rs:1485-1489` |
| `invalidate_statement` cascade replaced; now returns the ids that actually moved | `graph.rs:3615-3641` |

Four §6 regressions added at `graph.rs:5248-5350`; the pre-existing
`revalidate_skips_invalid_dep_in_cap` was asserting `Invalid` on a cascade
dependent — i.e. asserting the defect — and was rewritten to assert the §3.3
behaviour while still exercising the revalidate path it was written for.
`cargo test` 85/85 unit tests pass.

### Scope correction — §3.3.3 recomputes DIRECT dependents only

The first implementation recomputed the whole transitive closure to fixpoint.
Replayed against a copy of the real graph, that produced **88 status changes**,
nine of them from `proven_strong` straight to `unproven`.

The reason is that only a direct dependent's cap *set* changes when a parent is
refuted — the refuted node leaves it. A deeper dependent's cap set is unchanged;
what moved is its parent's status, and **braim does not propagate status
changes** (verification is computed at creation and never propagates). A fixpoint
over the closure retroactively introduces that propagation and collapses subtrees
to the floor, because it reveals every stored status that was consistent with its
parents at creation time and is not consistent with them now.

So §3.3.3 is narrow by construction: recompute the direct dependents, leave the
rest alone. Re-deriving historical statuses graph-wide is a separate change with
its own spec, and it is not what a refutation should trigger.

### Scope correction — the indirect closure is a count, not a marker

§3.3.4 first stamped `support_review_pending` on every indirect dependent. On the
real graph that flagged **596 nodes**, about 45% of it — noise rather than a
worklist. The set is recoverable from `find_cascade_nodes` at any time, so what
is stored on the refuted node instead is the scale and the entry points:

```
refutation_direct_dependents: "433,435,436,438"
refutation_indirect_dependents: "596"
```

### Verified against the real graph

Replaying the exact operation that destroyed 530 nodes — attaching the third
PRIMARY source to ID:1016, auto-resolving against ID:432 — on a copy of
`v0231`:

| | before the patch | after |
|---|---|---|
| newly invalid | 530 + the loser | **`[432]`** — the loser alone |
| total status changes | 531 | **2** (`432 → invalid`, `1016 → proven`) |
| winner ID:1016 | `invalid` | **`proven`**, matching the printed message |
| nodes flagged | none | 4 direct dependents |

§4's migration is not needed on the current graph: it was restored to `v0231`
(1333 nodes, 89 invalid, zero `depends_on_invalidated:432`). The migration text
stands for any graph that still carries cascade-invalidated nodes.

## Additional finding: `update-deps` and `statement add` disagree

`statement update-deps` refuses outright:

```
Error: Dependency ID 3 is invalid — cannot wire a statement to a refuted node
```

`statement add` permits it, and under §3.2 as patched produces a node with
`support_withdrawn_by` recorded. Both behaviours are defensible; having both is
not. This predates the patch and is left for the spec owner, but note that the
strict `update-deps` guard is what makes §4's migration awkward to express as CLI
calls — the migration has to write recomputed statuses directly rather than by
re-wiring.

## 0. Note on the parent document

`src/graph.rs` cites `BRAIM_DEPENDENCY_INHERITANCE_SPEC` at three points — §3.1
at line 240, §3.2 at line 2593, §3.3 at line 3594 — and **no such document
exists in the repository**. `grep -rln DEPENDENCY_INHERITANCE .` returns
`src/graph.rs` alone.

So there is no §3.3 text to amend. What follows states the current behaviour as
the code implements it, then gives replacement normative text. §3.1 and §3.2 are
restated here only where §3.3 cannot be fixed without them — see §5.

## 1. Current behaviour

### 1.1 The rank ladder (§3.1, `graph.rs:163-171`, `:240-250`)

```
Invalid = 0, Unproven = 1, Contested = 2, Partial = 3, Proven = 4, ProvenStrong = 5
```

`Invalid` is the bottom rung of the same ordered ladder that carries the support
levels.

### 1.2 Inheritance at creation (§3.2, `graph.rs:2593-2632`)

Only statement-typed dependencies participate; concept dependencies are skipped.
Then, per the comment at `:2595`:

> Invalid deps propagate fully (mark the new node invalid). Otherwise cap
> source_derived to the weakest statement dep.

A single invalid dependency sets the new node to `Invalid` with
`invalid_reason: depends_on_invalidated:<dep_id>`, regardless of the node's own
sources and regardless of that dependency's weight.

### 1.3 Cascade on an existing node (§3.3)

`find_cascade_nodes` (`:3567-3591`) is an unweighted breadth-first closure over
the reverse `depends_on` relation. It consults neither edge weights nor the
dependents' own sources.

Three call sites consume it and all three assign rather than recompute:

| Site | Trigger |
|---|---|
| `:1313-1323` | contradiction raised |
| `:1462-1472` | contradiction resolved (loser) |
| `:3611-3632` | `invalidate_statement` |

Each writes `verification_status = Invalid`, `node_type = InvalidStatement`,
`invalid = true`, `invalid_reason = depends_on_invalidated:<id>`.

### 1.4 Observed consequence

Resolving one contradiction on 2026-08-10 — ID:432 against ID:1016, over whether
`ModelEventDispatcher::drain` collapses its queue before firing webhooks — moved
the graph from 89 invalid nodes to 620. The cascade from ID:432 alone invalidated
**530** nodes.

Of those 530:

- **85** carried two or more distinct PRIMARY source types of their own, which
  §3.2's own source rule scores as `Proven`. Their sources were never consulted.
- **53** had the refuted ancestor reachable only through **≤50%** of their
  dependency weight, majority weight surviving. ID:610 was invalidated at a
  tainted weight fraction of **0.40**; its content — an `after_or_equal` rule at
  `CreateInvoiceMutation.php:59` — had been measured and attached the same day.
- **ID:1016, the winner of the resolution**, was invalidated by the cascade of
  its own resolution. The path is `1016 → 438 → 432`.
  `resolve_contradiction` promotes the winner at `:1425-1441` and then
  invalidates it at `:1462-1477`.

## 2. The defect

`Invalid` and `Unproven` answer different questions.

- **`Invalid`** = *refuted*. Evidence bears on this statement and contradicts it.
- **`Unproven`** = *unsupported*. Nothing establishes this statement yet.

A statement whose parent was refuted has **lost support**. It has not been
refuted. Nothing was learned about it.

§3.1 places `Invalid` at rank 0 of the support ladder, so `MIN(source_derived,
weakest dep)` — the correct operation everywhere else in braim — yields `Invalid`
whenever any dependency is refuted. §3.2 and §3.3 then encode that as the
intended behaviour. The single conflation at §3.1 is the root; §3.3 is where it
does the most damage.

Two consequences follow, and both were observed:

1. **Refutation is not a support level, so it must not be inherited through a
   support-capping operation.** Inheriting it destroys independently sourced
   knowledge (the 85 nodes).
2. **A refuted node can still carry true descendants.** ID:432 was wrong about
   two specific things: that the queue dedupes to final state before webhooks,
   and that the entry method is `begin()`. A descendant touching neither is
   unaffected in substance. That judgement is not derivable from graph topology.

## 3. Normative replacement for §3.3

### 3.3.1 Refutation is direct-evidence-only

A statement transitions to `Invalid` **only** by:

- `invalidate_statement` with an explicit reason, or
- losing a contradiction resolution.

`Invalid` MUST NOT be assigned by dependency inheritance, at creation or on
cascade. This supersedes §3.2's "Invalid deps propagate fully".

### 3.3.2 Refuted dependencies are excluded, not floored

When computing a statement's dependency cap, a dependency at `Invalid` is
**excluded from the cap set** rather than contributing rank 0. The cap is the
minimum rank over the statement's **non-refuted** statement dependencies.

If every statement dependency is refuted, the cap set is empty and the cap does
not apply — the statement's status is its `source_derived` value alone, exactly
as for a statement with no statement dependencies.

### 3.3.3 Cascade recomputes; it does not assign

On a statement becoming refuted, its transitive dependents (the existing
`find_cascade_nodes` closure) are **recomputed** under §3.3.2, not assigned. For
each dependent:

```
new_status = min_rank(source_derived, cap over non-refuted statement deps)
```

The result MUST NOT be `Invalid`. A dependent that loses all support settles at
`Unproven`, which is the correct reading: unsupported, not false.

### 3.3.4 Withdrawn support is recorded, and it is a worklist

Each recomputed dependent records the withdrawal on the node:

- `metadata["support_withdrawn_by"]` — comma-joined ids of the newly refuted
  dependencies it reached
- `metadata["support_withdrawn_at"]` — timestamp

Nodes carrying `support_withdrawn_by` are **enqueued for adjudication**, not
decided. §3.3 owns the mechanical recompute; whether the dependent's text
actually rests on the refuted claim is a reading, and belongs to a reviewer or a
dream session.

Rationale: this is the only place a judgement is required, and topology cannot
supply it. Emitting a worklist preserves the case in §2.2 — a refuted parent
leaving true children, some of which later evidence attaches to.

### 3.3.5 The winner exclusion is a consequence, not a special case

Under §3.3.1 the winner of a contradiction resolution cannot be refuted by the
cascade of that same resolution, because the cascade no longer assigns `Invalid`
to anything. No `if dep_id == winner_id` guard is required.

The winner MAY legitimately depend on the loser, and after recomputation its
status is whatever §3.3.2 yields — possibly a demotion, never a refutation.

### 3.3.6 Invariants

1. `invalid == true` implies `invalid_reason` names direct evidence — an
   `invalidate_statement` reason or a resolution — never `depends_on_invalidated`.
2. No operation assigns `Invalid` to a node other than the direct target of
   §3.3.1.
3. Recomputation is deterministic from `sources` and `depends_on`. No
   pre-cascade status needs storing; `pre_contested_status` (`graph.rs:365`)
   remains scoped to contested transitions.
4. A node's status never depends on the *order* in which cascades ran.
   Recompute-from-inputs gives this; assignment did not.

## 4. Migration

`depends_on_invalidated:*` is fully re-derivable, so no information is lost by
recomputing it away.

1. Select every node whose `invalid_reason` begins `depends_on_invalidated:`.
   On the working graph: **530** nodes, all from ID:432.
2. Clear `invalid`, `invalid_reason`, `invalidated_at`, and set
   `node_type` back to its statement-family value.
3. Recompute each under §3.3.2. Order does not matter (invariant 4), but two
   passes are cheaper than fixpoint iteration on a graph this size.
4. Stamp `support_withdrawn_by` per §3.3.4 so the affected set is reviewable.
5. Report the count. Silent repair of 530 nodes would be indistinguishable from
   silent corruption.

Migration is idempotent: a second run selects nothing.

## 5. Sections this change reaches

§3.3 cannot be fixed alone.

- **§3.1** (`graph.rs:240-250`) — `rank()` must stop treating `Invalid` as rank 0
  of the support ladder. Either remove it from `rank()` and make the support
  floor `Unproven`, or keep the numeric value and forbid `from_rank(0)` from ever
  being the result of a cap. The first is cleaner; the second is a smaller diff.
- **§3.2** (`graph.rs:2593-2632`) — delete "Invalid deps propagate fully" and
  apply §3.3.2's exclusion at creation. Without this, a statement created against
  a refuted dependency is still born `Invalid`, and the cascade fix only defers
  the problem to the next `statement add`.

## 6. Test obligations

1. Resolve a contradiction whose winner transitively depends on the loser. Assert
   the winner is not `Invalid` and holds its promoted status. Regression for the
   ID:1016 case.
2. Refute a node with a dependent carrying two distinct PRIMARY source types.
   Assert the dependent keeps its source-derived status. Regression for the 85.
3. Refute a node reachable through 0.4 of a dependent's weight where the 0.6
   parent is `Proven`. Assert the dependent is demoted, not refuted. Regression
   for ID:610.
4. Refute a node that is a dependent's only support, where the dependent has no
   sources. Assert `Unproven`, not `Invalid`.
5. Attach a promoting source to a node from test 4. Assert it promotes normally —
   a withdrawn-support node is not poisoned against future evidence.
6. Run the §4 migration twice. Assert the second run is a no-op.
7. Refute two ancestors of one dependent in either order. Assert identical final
   status. Invariant 4.

## 7. Open question for the spec owner

§3.3.4 says withdrawn-support nodes are *enqueued*, without saying where.

`Contested` already means "needs adjudication", but it is defined as two
statements disagreeing, and a withdrawn-support node has no counterparty. Reusing
it would overload the state and pollute `--include-contested`.

The alternatives are a distinct `verification_status` variant, or metadata alone
plus a `braim list --meta support_withdrawn_by` convention with no status change.
Metadata alone is the smaller change and keeps the status ladder honest; a
variant is more discoverable. This is a design call, not a correctness one, and
it is deliberately left open.
