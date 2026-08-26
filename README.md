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
           add-source | delete-source |
           update-sources | contradict |
           resolve-contradiction | invalidate |
           revalidate | update-weights |
           update-deps | delete
source     add                                  first-class source entities
lookup | query | proximity | perspective        discovery and navigation
why-add | why | why-test | why-remove           causal chains (because_of, Five Whys)
similar                                         semantic search + dedup (default build)
node | list | domains | audit | meta            inspection and metadata
version    save | list | restore                checkpoints (per-domain when sharded)
init | policy                                   team bootstrap + agent policy payloads
dream      candidates | constraints | whatif |  pair/constraint discovery, review queue,
           flag | review | reviewed | log |     ledger — overnight LLM adjudication
           seen
merge-nodes                                     fold a duplicate into its survivor, unioning evidence
export | shard | rename-domain                  publish a domain, per-domain storage, governance
serve | import | migrate-node-types |           viewer, cross-project, migration,
migrate-refutation                              pre-3.3 refutation-cascade repair
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
braim sources add "Treasurer ledger 1995" --type doc \
  --location "doc:treasurer_ledger_1995.pdf:7"          # → source entity ID:N
braim statement add-source 42 --source-id N             # attach; recomputes status
braim statement delete-source 42 --source-id N          # detach; recomputes status (can demote)
braim statement update-sources 42 --remove "doc:spec.md:5" --add "doc:spec.md:12-18"
                                                         # fix a wrong citation IN PLACE — no new ID,
                                                         # no broken depends_on/because_of/contradicts edges
```

@[A source entity referenced by multiple statements counts once for PRIMARY diversity] source: braim sources add --help

@[delete-source and update-sources leave invalid and contested statements untouched — those states come from invalidation/contradiction, not source diversity, so editing sources there does not revive or alter status] source: code:src/graph.rs delete_source_from_statement, update_statement_sources

@[update-sources refuses an edit that would leave a statement with zero string sources AND zero attached source entities; --set is exclusive of --add/--remove; new strings must carry a typed prefix like statement add requires] source: braim statement update-sources --help

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

### Causal chains (Five Whys)

A `because_of` edge links one statement to its cause (consequent → cause). It is directional and unweighted — distinct from compositional `depends_on` and from `contradicts`. `perspective` and `proximity` traverse it (cause → consequent, full weight) alongside `depends_on`; because endpoints are always statements, a concept-to-concept query is unaffected. `query` still follows `depends_on` only.

```bash
braim why-add 42 --because 17 --source "narrative:investigation"  # record consequent → cause
braim why 42                          # walk the chain to the root cause
braim why-test 42                     # inverse test PASSED (cause confirmed)
braim why-test 42 --fail              # inverse test FAILED (refutes the link, not the statements)
braim why-remove 42                   # detach 42's cause so it can be reassigned
braim why-add 42 --because 73         # re-point 42 at a new cause
```

@[because_of accepts only statement endpoints; one outgoing edge per statement (competing causes go through contradicts); cycles are rejected; chain depth >= 7 warns and > 10 rejects] source: braim why-add --help

@[Reassigning a cause means why-remove then why-add — why-remove drops the active edge, or a refuted edge if no active one remains] source: braim why-remove --help

@[perspective and proximity traverse because_of cause-to-consequent at weight 1.0; refuted edges are skipped; query stays depends_on-only] source: code:src/graph.rs dfs because_of branch

@[audit surfaces four causal-edge findings: refuted links, statements flagged for re-investigation, untested links, and unverified root causes] source: code:src/graph.rs audit because_of findings

@[A causal edge inherits the weakest endpoint's status; when both endpoints are proven the claim is partial until a passing inverse test promotes it to proven] source: code:src/graph.rs edge_causal_status

@[Invalidating a cause flags every consequent above it (metadata because_of_reinvestigate) for re-investigation but does not auto-invalidate them] source: code:src/graph.rs flag_because_of_reinvestigation

### Semantic similarity (default build)

Ships by default (pulls fastembed/ONNX; needs rustc >= 1.88) — omit it with `cargo build --release --no-default-features` for a dependency-light binary.

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
braim meta 6500 --inc recurrence            # increment a numeric key, prints new value
braim meta 6500 --unset scope               # remove a key entirely
braim list --meta scope=agent_scratch       # filter by metadata
```

#[The scope=agent_scratch pattern keeps agent reasoning markers in the main graph but filterable out of discovery] based_on: @[meta set/list] + @[default facts-only queries already hide 0-PRIMARY claims]

### Maintenance

```bash
braim audit                      # orphans, pending, gaps, dangling refs, causal-edge health
braim audit --semantic           # + near-duplicate pairs and label echoes (default build)
braim domains                    # domain inventory — check before adding (rule 4)
braim version save "checkpoint"  # rule 3: checkpoint after each batch
braim version restore 12         # overwrites current.json; save first
braim serve --port 3000          # interactive graph viewer (physics/animation toggle)
braim import /other/.braim --domain-map "Finance:Billing" --only-proven
braim migrate-refutation         # repair pre-3.3 refutation-cascade collateral (dry by default, --apply to write)
```

@[Never use jq or other tools directly on current.json] source: braim --help REQUIRED RULES:2

### Getting set up

```bash
braim init --team --central ~/.braim_central   # local graph + agent policy hooks
braim policy perturn                           # what the hooks inject
```

@[init installs UserPromptSubmit and PreCompact hooks that call `braim policy`, so the settings file carries no absolute paths or shell tools and works unchanged on Linux, macOS, and Windows] source: code:src/bootstrap.rs install_hooks

@[The merge is safe: existing settings and foreign hooks are preserved, re-running is idempotent, and an unparseable settings file is refused rather than clobbered] source: test:src/bootstrap.rs

#[Adoption is solo-first because a teammate starting out has no graphs to consume; what ships day one is the setup that already works alone] based_on: @[greenfield bootstrap decision] + @[the policy hooks automate the discipline a human would otherwise have to learn]

### Federation — working graphs publish into a central braim

```bash
braim shard                                   # split storage into domains/<name>-<hash>.json
braim import /other/.braim --full             # trusted self-import: keeps verification, edges, sources
braim export billing                          # publish a domain (target from init --central)
braim merge-nodes 42 99                       # fold a duplicate into 42, unioning its evidence
braim rename-domain Billing braim_demo        # governance: re-home a domain across the graph
```

@[Sharded storage keeps ONE merged in-memory view; version save then writes per-domain snapshots that are the pin artifacts the mount manifest references] source: doc:braim-mount-manifest.md

@[Export defaults to a PARTIAL floor — a statement needs at least one PRIMARY source to publish, so two people each holding one source type can both publish and corroborate in central] source: code:src/main.rs Export

@[Concurrent writers are serialised by a cross-process lock taken before loading; all writes are atomic renames; readers stay lock-free via a seqlock] source: code:src/graph.rs FileLock

### Dreaming — overnight relation discovery

```bash
braim dream candidates --strategy semantic --json   # ranked pairs worth an LLM's judgement
braim dream seen 42 99 --verdict duplicate          # ledger, so nights advance
braim list --meta scope=dream                       # morning review
```

@[Candidate generation is read-only and refused on graphs marked .braim.central — a dream is an unreviewed hypothesis and an unattended central has no reviewer] source: code:src/dream.rs refuse_if_central

#[Strategies differ by two orders of magnitude in selectivity, so budget runs semantic-first] based_on: @[measured yields on a 714-node graph: 30 semantic / 148 shared-source / 5135 two-hop]

**Relaxing constraints** — a second dream mode, alongside pair discovery: rank the causes most statements rest on, then walk one to see what actually moves.

```bash
braim dream constraints --limit 10           # rank because_of causes by blast radius
braim dream whatif 186                       # walk one: what rests on it, what it serves, staleness signals
braim meta 210 --set counterfactual=true     # tag a hypothesis written from a relaxation — export refuses these
```

@[dream constraints scores by transitive statement count reaching a cause through because_of, discounted (not excluded) when the cause is unproven] source: braim dream constraints --help

@[dream whatif reports staleness signals first — a later statement citing the same PRIMARY source, evidenced at least as well, with no contradiction linking the two yet — since that half of what-if dreaming is provable and the improvement half is not] source: braim dream whatif --help

**Review queue** — the part of a night's output that isn't a node survives the session:

```bash
braim dream flag "merge 412 warned about deps only the loser had" --kind merge --nodes 412,88
braim dream review                           # pending items, oldest reasoning first
braim dream reviewed 3 --note "wired the dependency by hand"
braim dream log --verdict verified --limit 20   # read back what a night adjudicated (dreams.json)
```

@[The review queue (reviews.json) exists because a closing report lives in the model's context and does not survive compaction — the part of the night most needing eyes was the part that evaporated] source: braim dream flag --help

The adjudication loop itself is `.claude/skills/dream/` — install with
`ln -sfn <repo>/.claude/skills/dream ~/.claude/skills/dream`.

---

## Agent Integration

@[policies/ holds the agent-integration contracts: perturn_logging.json (UserPromptSubmit hook payload), compaction_rule.txt (PreCompact hook payload), memory_braim_traits.md (evidence discipline)] source: policies/README.md

Core disciplines the policies encode:

- **Lookup-first**: `braim lookup --exact` / `query --include-claims` before every add — duplicates are the documented failure mode. Follow with semantic dedup: `braim similar "<label>" --dedup` or `--check-dupes` on the add; a hit >= 0.8 means reuse, not add.
- **Markers**: `@[verbatim fact]` with typed citation, `#[inference]` with 2+ asymmetric deps, `?[unknown]` with evidence_needed; exactly one marker per claim.
- **Re-grounding**: a braim node label is a pointer, not evidence. Figures and quotes are verified against the cited source document; label-vs-document disagreement means the document wins and the node gets contradicted or invalidated — at promotion time especially.
- **Promotion never by fiat**: claims become facts only through `add-source` with genuinely diverse PRIMARY types.

## Tests

@[tests/ holds a 34-scenario blind-agent suite: prompts in scenario_NN.txt, operator-side checks in oracle.txt, procedure in run.txt] source: tests/oracle.txt SCORING

@[Scenarios 01-08 cover base features, 09-14 real-world usage violations, 15-20 cross-source verification primitives, 21-23 the hook policies, 24-26 the evidence-discipline traits (26 scored on the saved reply text), 27-30 the because_of causal edge and Five Whys commands (why-add/why/why-test/why-remove) plus their traversal in perspective/proximity, 31-34 the dream commands (candidates/constraints/whatif/flag/review/reviewed/seen/log)] source: tests/oracle.txt SCORING

#[The suite tests whether an LLM operating under the policies produces a conformant graph — agent-behavioral, so it runs live sub-agents rather than cargo test] based_on: @[blind scenario + operator oracle design] + @[policies handed to agents as operating contracts]

---

## Persistence

@[Graph persists to .braim/current.json after every mutation; named checkpoints via braim version save; restore overwrites current state without auto-saving] source: braim version restore --help
