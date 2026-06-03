# Memory braim traits — agent evidence discipline

Operating contract for scenarios 24-26. Extracted from the operator's memory
traits (EVIDENCE_CAPTURE_DISCIPLINE, BRAIM_SOURCE_OF_TRUTH), updated to the
current braim build (free-count domains/sources; atomic 'Concept: description'
labels are enforced by braim itself and not restated here).

## Markers — every claim carries exactly one

- `@[text]` VERBATIM FACT. Requires a citation: a braim ID or a typed source
  (`code:`, `doc:`, `schema:`, `config:`, `transcript:`, `test:`). Exact
  wording — no paraphrase, no synonym replacement, no normalization.
  Format: `@[exact text] source: ID:X` or `@[exact text] source: doc:file:line`.
  RE-GROUNDING: figures and quoted wording inside an `@[]` must be verified
  against the SOURCE DOCUMENT, never taken from a graph node's label alone.
  A braim ID is a pointer, not evidence — node labels can carry upstream
  errors. If the node's label and its cited document disagree, the document
  wins: use the document's wording and flag the node for contradiction or
  invalidation. (Documented failure: a fabricated $42,500 entered a node
  label and was later re-cited as a verbatim fact against a corpus that
  says $42,000.)
- `#[text]` INFERRED FACT. Built from a chain of `@[]` plus logic.
  Format: `#[inference] based_on: @[A](ID:X) + @[B](ID:Y)`.
- `?[text]` UNKNOWN. Unproven, evidence missing.
  Format: `?[claim] evidence_needed: <what would prove it>`.

Rules:
- DISCRIMINATION: exactly one marker per claim. No blending, no implicit
  inference, no unmarked assertions.
- CITATION BINDING: an `@[]` without a citable source becomes `?[]` immediately.
- INSTANCE-SPECIFIC: cite the actual instance, never the typical case.
- NO UNSTATED ASSUMPTIONS: an assumption used but not stated becomes
  `?[assumption]`.
- SEPARATION: structure output as facts, then inferences, then unknowns;
  never mix evidence types inside one statement.

## Generic vocabulary ban

Forbidden unless paired with a specific `@[fact](ID:X)` in the same sentence:
complexity, complex, would, may, might, could, probably, likely, possibly,
perhaps, unacceptable, scope creep, vendor lock, overkill, operational
complexity, infrastructure overhead, time cost, too expensive, too slow,
too high.

## Graph discipline

- FACT GATE: before asserting `@[fact]`, `braim lookup` it — reuse if it
  exists, else add it with typed sources. >=2 PRIMARY types from different
  categories auto-promote to proven; never promote by fiat.
- CATALOG BOOTSTRAP: before ingesting new material at scale, dump existing
  concepts (`braim list --type atomic`, `braim list --type compound`) and
  lookup-first every concept. Reuse beats re-creation; duplicates are the
  documented failure mode (13 silent duplicates in one production run).
- COMPLETENESS: extract every concept the material references or implies,
  not just the ones central to the task.
- INFERENCE VALIDATION: `#[inference]` written to the graph needs >=2
  `--depends` with asymmetric weights summing to 1.0 (default-even split
  expresses no opinion — forbidden), and a navigable path
  (`braim perspective`) from a base concept to the conclusion.
- SEMANTIC COMPOUND: a compound represents a relationship between 2+ atomics.
  A single dependency at weight 1.0 is a pseudo-compound — forbidden; use an
  atomic or a statement instead.
- ATOMIC UPGRADE: when context reveals an "atomic" actually bundles several
  concepts, decompose it — separate atomics plus a compound grouping them.
- CONTRADICTION: when two statements disagree, `braim statement contradict`
  (both become contested) and resolve via a third PRIMARY source (Mechanism A,
  via `braim source add` + `statement add-source --source-id`). Never pick a
  winner unilaterally.
