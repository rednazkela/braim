# braim — Semantic Knowledge Graph

@[braim stores concepts, statements, and relationships in a semantic graph with native verification] source: braim --help:1

@[The graph lives in cwd/.braim (override with --data-dir) and persists to current.json] source: braim --help REQUIRED RULES:1

Serves high-quality context to an LLM with an agent-agnostic, multi-source, cross-session approach: a fully traceable, verifiable context window that an agent cannot silently corrupt.

---

## Node Types

@[node_type values: atomic, compound, claim, fact, contested_statement, invalid_statement, source] source: braim --help CORE CONCEPTS

| Symbol | Type | What it is |
|---|---|---|
| `●` | atomic | base concept; label MUST follow `'Concept: description'` format |
| `◉` | compound | groups 2+ atomics via weighted `--depends`; plain space-separated label, exempt from the colon rule |
| `?` | claim | statement with 0 PRIMARY sources — hidden from default queries |
| `▶` | fact | statement with 1+ PRIMARY sources — returned by default |
| `⚠` | contested_statement | disputed via a contradicts edge; hidden unless `--include-contested` |
| `✗✗` | invalid_statement | refuted; hidden unless `--include-invalid` |
| | source | first-class source entity (type + location + ingested_at) |

@[Atomic labels are rejected without a 'Concept: description' colon; spacing around the colon is auto-normalized] source: braim concept add --help Validation Rules

@[A claim/fact's node_type is derived from its verification status, never set by hand] source: code:src/graph.rs NodeType::from_verification_status

## Source Taxonomy and Verification

@[All sources MUST carry a typed prefix] source: braim --help REQUIRED RULES:6

| Tier | Prefixes | Effect |
|---|---|---|
| PRIMARY | `code:` `doc:` `schema:` `config:` `transcript:` `test:` | counts toward verification |
| SECONDARY | `phase_N:` `agent:` `narrative:` | context only |
| TERTIARY | `logic:` `inference:` | derivations only |

@[Verification is auto-calculated from PRIMARY type DIVERSITY: 0 → unproven, 1 → partial, 2 distinct types → proven, 3+ → proven_strong] source: braim --help VERIFICATION STATUS

@[Final status = MIN(source-derived status, weakest statement dependency); concept dependencies are excluded from inheritance] source: braim --help INHERITANCE RULE

#[Two `doc:` citations stay partial — promotion needs PRIMARY types from different categories, never more of the same] based_on: @[type diversity rule] + @[observed: doc+doc statement remained partial after add-source]

## Weights

@[--depends weights must sum to 1.0; --domains and --sources are free-count (no parity required)] source: braim --help REQUIRED RULES:7

@[Use ASYMMETRIC weights when dependencies have unequal importance; default-even split means "no opinion" and is a code smell when one dep is clearly more central] source: braim --help REQUIRED RULES:9

@[Weight propagation is multiplicative along paths; query/perspective/proximity scores depend on it] source: braim --help REQUIRED RULES:9

@[Repeated identical --domains entries draw a duplicate-domain warning (write still succeeds; --strict-domains rejects)] source: braim statement add --help Validation Rules

## Commands

```text
concept    add | delete | update-weights |      atomics + compounds
           update-deps
statement  add | verify | verify-suggest |      claims/facts lifecycle
           add-source | contradict |
           resolve-contradiction | invalidate |
           update-weights | update-deps | delete
source     add                                  first-class source entities
lookup | query | proximity | perspective        discovery and navigation
similar                                         semantic search + dedup (embeddings builds)
node | list | domains | audit | meta            inspection and metadata
version    save | list | restore                checkpoints
serve | import | migrate-node-types             viewer, cross-project, migration
```

### Creating knowledge

```bash
# atomic — 'Concept: description' is enforced
braim concept add "Invoice: document requesting payment for goods or services" \
  --domains payment --sources "doc:spec.md:3"

# compound — plain label, 2+ atomic deps, weights sum 1.0
braim concept add "Credit Card Payment" --domains payment \
  --sources "code:card.rs" --depends "1:0.6,2:0.4"

# statement — typed sources, weighted deps
braim statement add "Payment requires Invoice" \
  --domains payment --sources "code:rules.rs:14,doc:spec.md:3" \
  --depends "1:0.7,3:0.3"
```

@[Statement text mentioning adjacent concept names is validated: if a matching compound exists but is absent from --depends the write errors; if no compound exists braim suggests one; --assume bypasses] source: braim statement add --help + code:src/graph.rs validate_statement_concepts

### Verifying and resolving

```bash
braim statement verify-suggest 42          # PRIMARY-typed candidate sources for a claim
braim source add "Treasurer ledger 1995" --type doc \
  --location "doc:treasurer_ledger_1995.pdf:7"          # → source entity ID:N
braim statement add-source 42 --source-id N             # attach; recomputes status
```

@[A source entity referenced by multiple statements counts once for PRIMARY diversity] source: braim source add --help

@[braim node <id> lists attached entities under "Source entities:" with type and location] source: braim node output

**Contested workflow** — when two statements disagree, never pick a winner by hand:

```bash
braim statement contradict 63 125 --reason "opening date conflict"
# → both become contested_statement, hidden from default queries

braim statement add-source 63 --source-id <photo_id>
# → Mechanism A: if the source is PRIMARY-typed and the other side lacks it,
#   auto-resolution fires — winner recomputes, loser becomes invalid_statement,
#   the contradicts edge is marked resolved
```

@[If the new source corroborates both sides, no auto-resolution; use statement resolve-contradiction] source: braim statement add-source --help

@[Invalidating a statement CASCADES to all transitive dependents] source: braim --help REQUIRED RULES:8

### Discovery

```bash
braim lookup "Fee" --exact          # matches the name part of 'Fee: ...' labels
braim query "funding"               # bidirectional; FACTS ONLY by default
braim query "fee" --include-claims --include-contested --include-invalid
braim perspective "Digital Loan" "Remote Lending Capability"   # directed paths
braim proximity "Member" "Late Fee"                            # shortest connection
```

@[Default queries return facts only; claims (0 PRIMARY), contested, and invalid nodes need their explicit flags] source: braim lookup --help

@[perspective registers zero-path pairs in the gap register for investigation] source: braim perspective output

### Semantic similarity (optional)

Requires an embeddings build: `cargo build --release --features embeddings` (pulls fastembed/ONNX; needs rustc >= 1.88).

```bash
braim similar "measuring how similar two texts are"   # nearest labels by MEANING
braim similar "Cosine: vector angle measure" --dedup  # write-time duplicate check, floor 0.8
braim concept add "..." --check-dupes                 # same check inline on add
braim audit --semantic                                # near-duplicates + label echoes
```

@[similar finds nodes by meaning even with zero shared words, where lexical query returns nothing] source: braim similar --help

@[query falls back to semantic suggestions automatically when concept traversal finds nothing] source: code:src/main.rs query_semantic_fallback

@[audit --semantic flags unconnected near-duplicate pairs (cosine >= 0.80) and label echoes — statements restating the label of one of their own dependencies (>= 0.75)] source: braim audit --help

@[Everything semantic is ADVISORY: it augments, never overrides, the verification lifecycle; the index is a sidecar at .braim/embeddings.json and only changed labels re-embed] source: braim similar --help

@[Non-feature builds keep every other command; similar and audit --semantic explain the rebuild needed] source: code:src/main.rs run_similar (not(feature)) stub

### Metadata

```bash
braim meta 6500 --set scope=agent_scratch   # first-class metadata on any node
braim list --meta scope=agent_scratch       # filter by metadata
```

#[The scope=agent_scratch pattern keeps agent reasoning markers in the main graph but filterable out of discovery] based_on: @[meta set/list] + @[default facts-only queries already hide 0-PRIMARY claims]

### Maintenance

```bash
braim audit                      # orphans, pending nodes, gap register, dangling refs
braim audit --semantic           # + near-duplicate pairs and label echoes (embeddings builds)
braim domains                    # domain inventory — check before adding (rule 4)
braim version save "checkpoint"  # rule 3: checkpoint after each batch
braim version restore 12         # overwrites current.json; save first
braim serve --port 3000          # interactive graph viewer (physics/animation toggle)
braim import /other/.braim --domain-map "Finance:Billing" --only-proven
```

@[Never use jq or other tools directly on current.json] source: braim --help REQUIRED RULES:2

---

## Agent Integration

@[policies/ holds the agent-integration contracts: perturn_logging.json (UserPromptSubmit hook payload), compaction_rule.txt (PreCompact hook payload), memory_braim_traits.md (evidence discipline)] source: policies/README.md

Core disciplines the policies encode:

- **Lookup-first**: `braim lookup --exact` / `query --include-claims` before every add — duplicates are the documented failure mode. On embeddings builds, follow with semantic dedup: `braim similar "<label>" --dedup` or `--check-dupes` on the add; a hit >= 0.8 means reuse, not add.
- **Markers**: `@[verbatim fact]` with typed citation, `#[inference]` with 2+ asymmetric deps, `?[unknown]` with evidence_needed; exactly one marker per claim.
- **Re-grounding**: a braim node label is a pointer, not evidence. Figures and quotes are verified against the cited source document; label-vs-document disagreement means the document wins and the node gets contradicted or invalidated — at promotion time especially.
- **Promotion never by fiat**: claims become facts only through `add-source` with genuinely diverse PRIMARY types.

## Tests

@[tests/ holds a 26-scenario blind-agent suite: prompts in scenario_NN.txt, operator-side checks in oracle.txt, procedure in run.txt] source: tests/README.txt

@[Scenarios 01-08 cover base features, 09-14 real-world usage violations, 15-20 cross-source verification primitives, 21-23 the hook policies, 24-26 the evidence-discipline traits (26 scored on the saved reply text)] source: tests/oracle.txt SCORING

#[The suite tests whether an LLM operating under the policies produces a conformant graph — agent-behavioral, so it runs live sub-agents rather than cargo test] based_on: @[blind scenario + operator oracle design] + @[policies handed to agents as operating contracts]

---

## Persistence

@[Graph persists to .braim/current.json after every mutation; named checkpoints via braim version save; restore overwrites current state without auto-saving] source: braim version restore --help
