# Agent-integration policies

These files are the canonical agent-integration contracts that scenarios
21-26 validate. They are not braim features; they are policies an LLM agent
operates under while using braim. In deployment the first two are injected by
Claude Code hooks; the third is the operator's evidence discipline.

## perturn_logging.json

The payload of a Claude Code `UserPromptSubmit` hook. On every substantive
reasoning turn the agent persists its reasoning markers to braim live:

- `#[inference]` -> `braim statement add` with >=2 `--depends` (asymmetric
  weights summing to 1.0, never uniform) and typed sources. Cite the returned
  id as `#[...](ID:N)`.
- `?[unknown]` -> `braim statement add` as an unproven claim with
  `evidence_needed`. Cite as `?[...](ID:N)`.
- After every add: `braim meta <id> --set scope=agent_scratch` (keeps the
  marker in the main graph but filterable out of discovery).
- Before every add: lookup-first (`braim lookup --exact` /
  `braim query --include-claims`) to avoid duplicates.
- Lifecycle: a confirmed `?[]` is promoted via `add-source` (>=2 PRIMARY types
  from different categories auto-promotes to proven, never by fiat), then the
  scratch tag is dropped; a refuted one is `braim statement invalidate`d.
- RE-GROUND AT PROMOTION: before `add-source`, the claim's figures/wording are
  verified against the attaching source document itself — a node label is a
  pointer, not evidence. Label-vs-source disagreement blocks promotion; the
  node is invalidated and re-added with the document's wording. The same
  applies when an `#[inference]` carries figures from a dependency.

Validated by scenario_21 (single-turn logging shape) and scenario_22
(the lifecycle: scratch -> promoted, and conflict -> contested -> resolved).

## compaction_rule.txt

The payload of a Claude Code `PreCompact` hook. At compaction the agent keeps
braim node IDs and the edges joining them (`ID:N --rel--> ID:M`), not prose
that already lives in braim. Orphan facts are pushed to braim first with a
PRIMARY source, then only the new ID is retained. `scope=agent_scratch` is a
meta tag applied only to scratch markers, never to facts meant to persist.

Validated by scenario_23.

## memory_braim_traits.md

The operator's memory evidence-discipline traits (marker system @[]/#[]/?[],
citation binding, verbatim capture, generic-vocabulary ban, fact gate, catalog
bootstrap, completeness, semantic compounds, validated inference, contradiction
handling), updated to the current braim build. The atomic 'Concept: description'
label rule is NOT restated here — braim enforces it natively (commit
edd8183), and oracle block 01 covers it.

Validated by scenario_24 (bootstrap/completeness/fact gate, graph oracle),
scenario_25 (compounds/inference, graph oracle), and scenario_26 (marker and
vocabulary discipline, reply-text oracle on results_26.txt).

## Why these are tested in the agent harness, not as `cargo test`

The braim CLI mechanics (contradict, add-source, meta, invalidate) are
deterministic and already exercised by scenarios 19-20. Scenarios 21-26
test the harder thing: whether an LLM *operating under the policy* produces a
conformant graph. That requires a live agent, so they follow the same
blind-scenario + operator-oracle model as the rest of the suite. The agent is
given the policy file as its operating contract; it is NOT given the oracle.
