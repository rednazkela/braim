# braim because_of Edge — Tests

Status: Implemented (tests 1–17). Companion to `braim-because-of-edge.md`. Tests 1–17 are covered by `cargo test` (`because_of_tests` in `src/graph.rs`) and verified via the CLI; test 18 is deliberately not implemented — see its note below.

Notation per test: Given / When / Then.

## 1. Edge creation, basic

Given: two existing statements A (consequent) and B (cause).
When: `braim why-add A --because B --source "narrative:investigation_2026-06-19"`.
Then: a directional `because_of` edge from A to B is recorded; A's outgoing-because_of count = 1.

## 2. Single-cardinality enforcement

Given: A already has one outgoing `because_of` edge (A → B).
When: `braim why-add A --because C`.
Then: command rejects with error stating A has an existing cause B; suggests `braim statement contradict B C` if causes compete, or removing the existing edge first.

## 3. Node-type guard

Given: an atomic concept K and a statement A.
When: `braim why-add A --because K` (cause is a concept, not a statement).
Then: command rejects with error stating `because_of` accepts only statement endpoints.

## 4. Traversal returns ordered chain

Given: chain A → B → C → D where D has no outgoing `because_of`.
When: `braim why A`.
Then: output lists A, B, C, D in order; marks D as `root_cause` (implicit terminal).

## 5. Root cause termination by status

Given: chain A → B → C where C has no outgoing `because_of` and verification status = proven.
When: `braim why A`.
Then: C is reported as root cause; output includes verification status.

## 6. Root cause termination, unproven leaf

Given: chain A → B → C where C has no outgoing `because_of` but verification = unproven.
When: `braim why A`.
Then: C is reported as terminal but flagged "candidate root cause, unverified"; suggests adding sources or extending the chain.

## 7. Inverse test passes

Given: edge A → B; B is the asserted cause of A.
When: `braim why-test A` and user attests "yes, A does not occur when B is absent".
Then: a `test:` source is recorded on the A → B edge; status of the causal claim upgrades per source-derived rules.

## 8. Inverse test fails

Given: edge A → B; user attests "A still occurs when B is absent".
When: `braim why-test A` with failing attestation.
Then: edge marked invalid with reason "inverse test failed"; cascade rules apply downstream as per existing invalidation semantics; A is suggested for re-investigation.

## 9. Maximum depth, soft warn

Given: a chain of length 7 starting at A.
When: `braim why-add` extending the chain to length 8.
Then: command succeeds with stderr warning "chain depth >= 7, consider whether this is converging on a root cause".

## 10. Maximum depth, hard reject

Given: a chain of length 10 starting at A.
When: `braim why-add` extending the chain to length 11.
Then: command rejects with error "chain depth limit reached"; suggests reviewing the chain for stalling.

## 11. Cycle detection

Given: chain A → B → C.
When: `braim why-add C --because A`.
Then: command rejects with error "cycle detected: A → B → C → A".

## 12. Contradicts integration on competing causes

Given: A → B exists. User suspects C also causes A.
When: `braim statement contradict B C --reason "competing causes for A"` then `braim why A`.
Then: traversal reports B as primary cause and flags `[contested with C — see contradicts edge]`; does not stop traversal.

## 13. Cascade on cause invalidation

Given: A → B → C → D. D invalidated.
When: `braim statement invalidate D --reason "..."`.
Then: cascade per existing braim rules: dependents of D in the `depends_on` graph are demoted; the `because_of` chain above D (A, B, C) is marked needs-reinvestigation but not auto-invalidated.

## 14. Verification inheritance, both endpoints proven

Given: A and B both have `proven` status; edge A → B has no `test:` source yet.
When: `braim why A`.
Then: edge displayed as `partial` causal claim, awaiting inverse test for promotion.

## 15. Verification inheritance, weakest endpoint caps status

Given: A is `proven`, B is `unproven`; edge A → B exists.
When: `braim why A`.
Then: edge displayed as `unproven` causal claim regardless of A's status.

## 16. Query isolation from depends_on (why-walk only)

Given: A `depends_on` B (compositional) and A `because_of` C (causal).
When: `braim why A`.
Then: the `why` walk follows only `because_of`; it does not include B.

Note: `braim perspective` and `braim proximity` now traverse BOTH `depends_on`
and `because_of` (cause → consequent, weight 1.0, refuted links skipped) — this
reverses the original "perspective/proximity remain depends_on-only" rule. The
`why` walk itself is still `because_of`-only (it never follows `depends_on`).
Because `because_of` endpoints are statements, concept-to-concept perspective /
proximity results are unchanged; the composition shows up only on paths through
statement nodes. `query` stays `depends_on`-only.

## 17. Source typing on the edge

Given: `braim why-add A --because B --source "narrative:hypothesis_2026-06-19"`.
Then: edge persisted with that source typed prefix; missing prefix → command rejects with the existing source-typing error.

## 18. Out-of-scope filtering — NOT IMPLEMENTED

Given: nodes A, B both tagged `scope=out_of_scope`; edge A → B exists.
When: default `braim query` is run.
Then: A, B, and the edge are filtered out — consistent with existing `agent_scratch` filtering behavior.

Note: deliberately not implemented. Scope-based query filtering is orthogonal to the `because_of` edge mechanics and would change core `query` semantics (which today hides only 0-PRIMARY claims, not arbitrary scope tags). Tracked separately.
