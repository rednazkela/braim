# Braim Feature Spec: Cross-Source Verification Primitives

**Status:** New feature proposal (three interrelated primitives)
**Companion documents:** `BRAIM_VERIFICATION_FEATURE_SPEC.md` (master); `BRAIM_AUTOPROMOTION_SPEC.md`, `BRAIM_DEPENDENCY_INHERITANCE_SPEC.md`, `BRAIM_NODE_TYPE_CLAIM_FACT_SPEC.md` (prerequisites — all shipped)
**Companion test:** `braim_cross_source_verification_test.sh`
**Date:** 2026-05-22
**Driven by:** cognitivex Goal 1 (Context Building) needs — specifically Goal 1.1 (concept sharing across sources), Goal 1.3 (contradiction handling with third-source resolution)

---

## 1. Scope

This spec defines three interrelated braim primitives required for multi-source context-building workflows:

1. **`source` as first-class node type** — replaces today's string-tag `--sources "code:foo.rs:42"` model with an entity that has its own ID, ingestion timestamp, and is referenceable across statements.
2. **`contradicts` edge between statements** — explicit contention relationship.
3. **`contested` verification state** — between `unproven` and `partial`, triggered by an active contradicts edge, awaiting tie-breaker.

All three ship together because they're tightly coupled: contradicts edges create contested state; contested state resolves when a third source corroborates one side; source-as-entity is required so that "third source" has well-defined identity.

---

## 2. Empirically Verified Gap

Today's braim source model is a string tag:

```json
{
  "sources": ["code:foo.rs:42-58", "doc:bar.md:3.2"]
}
```

This works for single-statement attribution but fails the cognitivex use case:

| Need (cognitivex Goal 1) | Current braim support |
|---|---|
| Multiple statements reference the same source instance | ✗ — sources are strings, no shared identity; same `code:foo.rs:42-58` in two statements is two strings, not one entity |
| Track when a source was ingested | ✗ — no timestamp on source |
| Detect when two statements about the same subject disagree | ✗ — no contradicts relationship |
| Hold a statement in limbo while waiting for a tie-breaker | ✗ — only states are unproven, partial, proven, proven_strong, invalid |
| Resolve contradiction when third source corroborates one side | ✗ — no resolution mechanism |

**Empirically observed in this project:** when Alexander's mother (per his presentation) was anxious about contradicting medical opinions, the journalism standard ("three sources to publish") was applied informally. Braim today encodes the threshold (`proven_strong` requires 3+ PRIMARY types) but cannot represent the *contradiction* state that the threshold is meant to resolve.

---

## 3. Required Behavior

### 3.1 Feature: `source` as first-class node type

**New node_type:** `source`

**Properties:**

| Field | Type | Required |
|---|---|---|
| id | int | yes (auto-assigned) |
| label | string | yes (human-readable identifier) |
| source_type | enum | yes (`code` \| `doc` \| `schema` \| `config` \| `transcript` \| `test` \| `phase_N` \| `agent` \| `narrative` \| `logic` \| `inference`) |
| location | string | optional (file path, URL, doc reference, line range) |
| ingested_at | timestamp | yes (auto-set on create) |
| ingested_by | string | optional (agent name, user ID) |

**CLI:**

```bash
braim source add "Refund design doc section 3.2" \
    --type doc --location "doc:billing_design.md:3.2" \
    --ingested-by "agent:context_building_phase"
# → returns source_id, e.g., ID:5001
```

**Statement reference (backwards-compatible):**

```bash
# New form: source_id references
braim statement add "Refund flow per design" \
    --domains billing --source-ids 5001 --depends "..."

# Legacy form (still supported, auto-creates source entity behind the scenes):
braim statement add "Refund flow per design" \
    --domains billing --sources "doc:billing_design.md:3.2" --depends "..."
```

When legacy `--sources` form is used, braim creates/looks-up a source entity by `(source_type, location)` and references it. Same location → same source entity → counted once for PRIMARY-type diversity.

**Effect on verification:**

`verification_status` still computed from distinct PRIMARY *types*. With entities, also track distinct PRIMARY *instances* (two different `doc:` entities = two doc sources). Optional future tier: `corroborated` = 2+ PRIMARY *instances* of the same *type*. Out of scope for this spec; flagged for future consideration.

### 3.2 Feature: `contradicts` edge between statements

**New edge type:** `contradicts`

**CLI:**

```bash
braim statement contradict 42 99 \
    --reason "Statement 42 says X, statement 99 says NOT-X per source 5001" \
    --source 5001
# → creates symmetric contradicts edge; both 42 and 99 move to contested state
```

**Properties of the edge:**

| Field | Type | Required |
|---|---|---|
| from | statement_id | yes |
| to | statement_id | yes |
| reason | string | yes |
| source | source_id | yes (which source revealed the conflict) |
| created_at | timestamp | yes |
| resolved | bool | no (default false; true after resolution) |
| resolution_source | source_id | no (set when resolved) |
| resolution_winner | statement_id | no (which side won) |

**Symmetric:** A contradicts B implies B contradicts A. Stored once, queried both ways.

**Effect on statements:** both `from` and `to` statements transition to `contested` verification_status (per §3.3). Their existing verification status (whatever it was) is preserved as `pre_contested_status` for restoration if the contradiction is invalidated.

### 3.3 Feature: `contested` verification state

**New verification_status value:** `contested`

**Position in ranking:**

```
invalid < unproven < contested < partial < proven < proven_strong
```

`contested` ranks below `partial` because a contested statement has unresolved evidence — not yet trustworthy.

**New node_type:** `contested_statement` (or alternatively, contested statements keep their existing node_type but are filtered by status — design choice; spec says new node_type for symmetry with `claim`/`fact`/`invalid_statement`).

**Trigger:** statement has at least one active (`resolved=false`) `contradicts` edge.

**Behavior:**

| Behavior | Detail |
|---|---|
| Default query visibility | Hidden (treated like `claim`) — must opt in via `--include-contested` |
| Auto-promotion | Cannot promote to partial/proven/proven_strong while contested, regardless of source count |
| Inheritance | Dependents of a contested statement inherit contested state (per existing inheritance rule, contested is between unproven and partial) |
| Invalidation | Allowed; cascades normally |

**Resolution mechanisms:**

**Mechanism A: third PRIMARY source auto-resolution.**

When a new source S of a PRIMARY type is added to one of the contested statements (call it the `supported`), and the source is NOT also added to the other (`unsupported`):

- `supported` resolves: `contested` → recompute from sources (likely `partial`/`proven`/`proven_strong`); restore via `pre_contested_status` baseline plus the new source
- `unsupported` resolves: `contested` → `invalid`, with `invalid_reason: contested_resolved_against_by_source_<S_id>`
- The `contradicts` edge marks `resolved=true`, `resolution_source=<S_id>`, `resolution_winner=<supported_id>`

The "NOT also added to" check matters — if the new source corroborates both (which would be odd but possible), the contradiction stays unresolved and requires manual intervention.

**Mechanism B: explicit resolve command.**

```bash
braim statement resolve-contradiction 42 99 \
    --winner 42 \
    --reason "Source 5005 (a code review) confirms 42 is correct" \
    --source 5005
# → marks 42 as winner (recompute status from sources)
# → marks 99 as invalid with reason "contested_resolved_against"
# → updates contradicts edge resolved=true
```

For when the third-source auto-resolution doesn't fire (e.g., the corroborating source already existed on both, or the resolution is procedural rather than source-driven).

### 3.4 Output format updates

**Trust badges (extending the existing scheme):**

| Status | Badge | Description |
|---|---|---|
| invalid | ✗✗ | refuted |
| unproven | ✗ | claim, no PRIMARY |
| **contested** | **⚠** | unresolved contradiction (NEW) |
| partial | ✓ | 1 PRIMARY |
| proven | ✓✓ | 2+ PRIMARY types |
| proven_strong | ✓✓✓ | 3+ PRIMARY types |

**Query flag (extending the existing scheme):**

| Flag | Returns |
|---|---|
| (default) | facts only (`partial`/`proven`/`proven_strong`) |
| `--include-claims` | + claims (`unproven`) |
| `--include-contested` | + contested (NEW) |
| `--include-invalid` | + invalid statements |
| `--include-claims --include-contested --include-invalid` | full audit view |

### 3.5 `--help` additions (one new section, one rule)

**New REQUIRED RULE (rule 10):**

```
10. When two statements about the same subject disagree, mark them contested
    via 'statement contradict' rather than asserting one as fact. Resolve via
    a third PRIMARY source (auto) or 'statement resolve-contradiction' (manual).
```

**New section "CONTRADICTION RESOLUTION":**

```
CONTRADICTION RESOLUTION (when sources disagree):
  Two statements about the same subject can be marked contested:
    braim statement contradict <stmt_A> <stmt_B> --reason "..." --source <S>
  Both move to 'contested' state — hidden from default queries.

  Resolution:
    • Add a third PRIMARY source to one side → auto-resolves to fact;
      the unsupported side becomes invalid (cascades to its dependents).
    • Or explicit: braim statement resolve-contradiction <winner> <loser>
      --reason "..." --source <S>

  Contested statements:
    • Cannot promote past 'contested' until resolved
    • Inherit contested state into their dependents (via existing inheritance rule)
    • Surface via 'braim query <term> --include-contested'
```

---

## 4. Test File Contract

The companion test `braim_cross_source_verification_test.sh` MUST:

1. Use an isolated temp `--data-dir`
2. Test source entity creation, lookup, sharing across statements
3. Test contradicts edge creation, symmetric visibility, both-sides contested
4. Test contested state behavior: default-hidden, no auto-promotion, inheritance
5. Test auto-resolution via third PRIMARY source
6. Test explicit resolve-contradiction command
7. Test edge cases: contradicting a proven statement; 3-way contradictions; resolved edge no longer contests
8. Clean up; exit 0 if all pass, 1 if any fail

---

## 5. Test Matrix

### 5.1 Source entity tests

| Test | Setup | Assertion |
|---|---|---|
| S1 | `source add` with type=code, location=foo.rs:42 | returns source_id; node has type=source, source_type=code |
| S2 | Two statements use the same legacy `--sources "code:foo.rs:42"` | both reference the SAME source entity (deduplication) |
| S3 | Statement created with `--source-ids 5001,5002` | verification_status computed from those entities' types |
| S4 | `node <source_id>` | returns source's fields including ingested_at |

### 5.2 Contradicts edge tests

| Test | Setup | Assertion |
|---|---|---|
| C1 | `statement contradict 42 99 --reason X --source S` | edge created; both statements' verification_status = contested |
| C2 | C1 + query "term mentioned in 42" (default) | neither 42 nor 99 returned (hidden) |
| C3 | C1 + query with `--include-contested` | both 42 and 99 returned with ⚠ badge |
| C4 | C1 + try to promote 42 by adding PRIMARY source matching only 42 | 42 auto-resolves to fact; 99 auto-invalidates; edge marked resolved |
| C5 | C1 + explicit `resolve-contradiction 42 99 --winner 42 --source S2` | 42 resolves; 99 invalidates; edge marked resolved |
| C6 | C1 + statement that depends on 42 | dependent inherits `contested` state |

### 5.3 Contested state tests

| Test | Setup | Assertion |
|---|---|---|
| ST1 | Create proven statement, then `statement contradict` it with another | proven statement demotes to contested; pre_contested_status=proven preserved |
| ST2 | ST1 + resolve in favor of the formerly-proven statement | restores to proven (or recomputes if new source added) |
| ST3 | ST1 + resolve against the formerly-proven statement | becomes invalid; cascades to dependents |
| ST4 | 3-way: A contradicts B; B contradicts C; A and C agree | A and B contested; C contested via B; resolution propagates as expected |

### 5.4 `--help` and CLI surface tests

| Test | Setup | Assertion |
|---|---|---|
| H1 | `braim --help` | contains REQUIRED RULE 10 and CONTRADICTION RESOLUTION section |
| H2 | `braim statement --help` | lists `contradict` and `resolve-contradiction` subcommands |
| H3 | `braim query --help` | lists `--include-contested` flag |

---

## 6. Implementation Status Checklist (for maintainer)

| Behavior | Current | Required |
|---|---|---|
| `source` node_type | ✗ missing | **add per §3.1** |
| `braim source add` CLI | ✗ missing | **add per §3.1** |
| Legacy `--sources` auto-creates source entity | ✗ N/A yet | **add per §3.1 backwards-compat** |
| Statement `--source-ids` parameter | ✗ missing | **add per §3.1** |
| `contradicts` edge type | ✗ missing | **add per §3.2** |
| `braim statement contradict` CLI | ✗ missing | **add per §3.2** |
| `contested` verification_status value | ✗ missing | **add per §3.3** |
| Default query hides contested | ✗ N/A yet | **add per §3.3** |
| `--include-contested` query flag | ✗ missing | **add per §3.3** |
| Third-source auto-resolution | ✗ missing | **add per §3.3 Mechanism A** |
| `braim statement resolve-contradiction` CLI | ✗ missing | **add per §3.3 Mechanism B** |
| Inheritance cap to contested | ✗ missing | **extend per existing inheritance rule** |
| `--help` updates (rule 10 + new section) | ✗ missing | **add per §3.5** |
| ⚠ trust badge for contested | ✗ missing | **add per §3.4** |

---

## 7. Order of Implementation (Recommended)

1. **`source` as node_type + CLI + storage** (§3.1) — foundational, all else depends on it
2. **Legacy `--sources` deduplication backwards-compat** (§3.1) — ensures no break to existing usage
3. **`contradicts` edge schema + `statement contradict` CLI** (§3.2) — depends on §3.1 (edge has source)
4. **`contested` verification_status value + ranking + node_type** (§3.3) — depends on §3.2 (triggered by contradicts edge)
5. **Default query hiding + `--include-contested` flag** (§3.3) — depends on §3.3
6. **Third-source auto-resolution logic** (§3.3 Mechanism A) — depends on §3.1 + §3.2 + §3.3
7. **Explicit `resolve-contradiction` CLI** (§3.3 Mechanism B) — depends on §3.2 + §3.3
8. **Inheritance propagation of contested** — extends existing inheritance rule
9. **`--help` updates** (§3.5) — last; encodes the shipped behavior

---

## 8. Backwards Compatibility

| Existing behavior | Effect of this spec |
|---|---|
| Statements with string `--sources` | Auto-creates source entities; same string → same entity (dedup). No break. |
| Existing `verification_status` values | Unchanged. `contested` is additive, not replacing. |
| Existing query default (facts only) | Extended to also hide contested (treated like claim). One-line change. |
| Existing `--include-*` flags | Unchanged. `--include-contested` is additive. |
| Existing inheritance rule | Extended: any contested dep caps dependent at contested. |
| Existing `node_type` values | Unchanged. `source` and (optionally) `contested_statement` are additive. |

---

## 9. Migration Concern

Existing production braim (e.g., rutanaranja's 2509-node graph) has string-tag sources. After this spec ships:

- One-shot migration: walk all statements, create source entities for each unique `(source_type, location)` tuple, replace string references with source_id references. Approximately ~3000 unique sources expected; bounded work.
- No contested edges exist today; nothing to migrate there.
- Recommended: ship `braim migrate-sources` CLI command, same pattern as `migrate-node-types`.

---

## 10. Out of Scope (for this spec)

- **Concept extraction from raw documents** (cognitivex Goal 1.4 ingestion pipeline). Different layer — that's "how do you go from a doc to braim concepts," not "how does braim store sources."
- **Deduplication of concepts with different names** (cognitivex open question). Matching algorithm is its own design problem.
- **Weights during context building** (cognitivex open question). Existing weight machinery handles this; cognitivex prompts can encourage asymmetric weights but no braim primitive change needed.
- **Future tier `corroborated`** (multiple PRIMARY *instances* of the same *type*). Flagged in §3.1 for later consideration; not blocking cognitivex.
- **Cross-owner federation primitives** (multi-organization sharing). Different scope entirely; ecosystem-layer concern.
