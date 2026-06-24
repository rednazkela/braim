# braim because_of Edge — Five Whys Methodology Support

Status: Implemented. Commands `why-add`, `why`, `why-test`, `why-remove` ship in the braim binary; see `src/graph.rs` (BecauseOfEdge, `why_*` methods) and `src/main.rs` (WhyAdd/Why/WhyTest/WhyRemove). `perspective`/`proximity` traverse the edge and `braim audit` reports four causal-edge findings (see Audit Integration). Test 18 (scope-based query filtering) is deliberately not implemented — it is orthogonal to the edge mechanics and would change core `query` semantics; tracked separately.

## Motivation

Support 5 Whys (Sakichi Toyoda, Toyota Production System): iterate "why?" from a symptom to its root cause.

Current braim edges do not express directional causality:

- `depends_on` — compositional. A is built from B with weight w; weights sum to 1.0.
- `contradicts` — conflict. A and B mutually exclusive.

A new `because_of` edge is proposed to mean "A occurs because B is true".

## Spec Decisions

| Decision | Recommendation | Rationale |
|---|---|---|
| Node scope | statements only | causes are claims about state-of-affairs, not raw entities |
| Cardinality | 1 outgoing `because_of` per statement | forces investigation discipline; branching via existing `contradicts` between competing cause statements |
| Weights | unweighted directional link | each link asserts principal cause, not partial contribution; weights belong on `depends_on` |
| Root termination | implicit: no outgoing `because_of` plus source-derived `proven` status | no explicit terminal flag |
| Validity check | `braim why-test <id>` — user attests consequent fails without cause, logged as `test:` source on the edge | inverse test, classical 5 Whys validation |

## Proposed Commands

- `braim why-add <consequent_id> --because <cause_id> [--source ...]` — add a `because_of` edge.
- `braim why <statement_id>` — walk the `because_of` chain to root cause; return ordered list of statements.
- `braim why-test <consequent_id>` — record inverse-test verification on the outgoing `because_of` edge.
- `braim why-remove <consequent_id>` — detach a statement's outgoing `because_of` edge so its cause can be reassigned (single-cardinality means the old edge must be removed before re-pointing via `why-add`). Removes the active edge, or a refuted edge if none is active.

## Verification Inheritance

A `because_of` edge inherits the verification floor of its endpoints. Both endpoints proven → causal claim is partial until inverse test confirms; inverse-test confirmation upgrades to proven causal claim.

## Audit Integration

`braim audit` reports four `because_of`-derived findings (computed in one pass over `state.because_of`), surfacing unfinished investigations:

- **Refuted links** — edges a failing `why-test --fail` marked invalid; a refuted causal claim left in place.
- **Re-investigation flags** — statements carrying `because_of_reinvestigate` because a cause below them was invalidated (`flag_because_of_reinvestigation`).
- **Untested links** — active edges with no `test:` source; unvalidated causal hypotheses awaiting the inverse test.
- **Unverified root causes** — terminal chain ends whose verification is below `proven`.

These are additive audit dimensions; they do not change the existing orphan / pending / gap / deprecated checks. Statements are never orphans (they always carry concept dependencies), so the orphan check is unaffected by causal edges.

## Open Questions

- Should `because_of` compose with `depends_on` in proximity / perspective queries? RESOLVED: yes. `perspective` and `proximity` traverse `because_of` (cause → consequent, weight 1.0, refuted links skipped) alongside `depends_on`; `query` stays `depends_on`-only. Because `because_of` endpoints are statements, concept-to-concept results are unchanged — the composition only manifests when a path runs through statement nodes. (Reverses the original isolation recommendation.)
- Should a maximum chain depth be enforced? Suggest soft warn at 7+ links, hard reject at 10+.
- How do `contradicts` and branching causes interact in traversal? Suggest `braim why` returns all candidates and flags contested branches.

## Out-of-Scope Items

This spec lives in `/home/naranja/planes/specs/` for visibility but is not a planes migration deliverable. The methodology applies to braim as a tool, not to the planes product. Corresponding braim nodes (IDs 158–164) are tagged `scope=out_of_scope`.
