# braim — Semantic Knowledge Graph

@[braim stores concepts, compound concepts, and statements as weighted nodes] source: src/graph.rs:Node struct

@[The graph lives in .braim/ and persists to .braim/current.json] source: src/main.rs:main()

@[Every node must track: domain (Vec<String>), sources (Vec<String>), dependency weights, verification status] source: src/graph.rs:Node struct

---

## Core Concepts

### Weights Sum to 1.0

@[All dependencies at a node must sum to exactly 1.0] source: src/graph.rs:add_compound(), add_statement()

#[This constraint is a semantic completeness guarantee: if weights cannot sum to 1.0, the node is either missing dependencies or not well-defined] based_on: @[weights sum constraint]

### Verification: Rule of 3 Truths

@[Statements require 3+ independent sources to be Proven] source: src/graph.rs:VerificationStatus enum

@[Verification statuses are: Unproven (1 source), Partial (2 sources), Proven (3+ sources)] source: src/graph.rs:verify_statement()

@[Each statement tracks verified_by: HashMap<String, Option<String>> to record which domains provided evidence] source: src/graph.rs:Node struct

#[A statement with only 1 source is Unproven because no independent corroboration exists] based_on: @[Unproven=1 source], @[independent verification requirement]

### Node Types

@[Atomic: irreducible concept with no dependencies] source: src/graph.rs:NodeType::Atomic

@[Compound: concept composed of 2+ existing units; weights declare contribution of each dependency] source: src/graph.rs:NodeType::Compound

@[Statement: declared relationship between exactly 2 units] source: src/graph.rs:NodeType::Statement

@[Inferred: statements with sources=["inferred"], derived relationships not claimed by any explicit source] source: src/main.rs:StatementAdd parse

### Invalidation

@[Statements can have 1+ dependencies: 1 dependency = property assertion (e.g. "Pets are allowed"), 2+ = relational claim] source: user clarification

@[Statements can be flagged invalid: bool, invalid_reason: Option<String>, invalidated_at: Option<String>] source: src/graph.rs:Node struct

@[Invalidation preserves node and history; deletion removes permanently] source: src/main.rs:DeleteNode, DeleteStatement branches

---

## Commands

### Add Atomic Concept

@[Atomics require exactly 1 domain and 1 source] source: src/graph.rs:add_concept()

```bash
braim concept add "Charge" --domains "Billing" --sources "docs"
braim concept add "Account" --domains "Instance" --sources "code"
```

### Add Compound Concept

@[Compounds depend on existing node IDs; weights must sum to 1.0] source: src/graph.rs:add_compound()

```bash
braim concept add "Voice Charge" --domains "Billing,Billing" --sources "code,code" --depends "1:0.5,3:0.5"
```

### Add Statement

@[Statements require 1+ dependencies; weights must sum to 1.0 (1-dependency = unary assertion, 2+ = relational)] source: src/graph.rs:add_statement()

@[Multiple domain-source pairs enable Rule of 3 verification] source: src/graph.rs:verify_statement()

```bash
# Explicit statement from a source
braim statement add "Voice charge applies to Account" \
  --domains "Invoice,Invoice" --sources "code,code" \
  --depends "2:0.5,4:0.5"

# Inferred statement (auto-marked; cannot be verified)
braim statement add "Monthly charges reflect usage" \
  --domains "computation,computation" \
  --depends "7:0.5,9:0.5" \
  --inferred
```

### Verify Statement

@[Add domain evidence: `braim statement verify <id> <domain> --note "explanation"`] source: src/main.rs:StatementCommands::Verify

```bash
braim statement verify 5 "Docs" --note "Section 3.2"
braim statement verify 5 "Schema" --note "billing_rules table"
braim statement verify 5 "Tests" --note "test_charge.rs:42"
# Statement 5 now Proven (3 independent domain verifications)
```

### Invalidate Statement

@[Mark a statement as refuted while preserving history] source: src/graph.rs:invalidate_statement()

```bash
braim statement invalidate 5 --reason "Billing rules changed Q3 2026"
```

### Update Weights

@[update-weights allows refining weights on existing compounds/statements after creation] source: src/graph.rs:update_weights()

@[Cannot change number of dependencies; must provide all dependencies with new weights] source: src/graph.rs validation

@[Weights must still sum to 1.0 after update] source: src/graph.rs validation

```bash
# Refine initial equal-weight distribution
braim concept update-weights 4 --weights "1:0.3,3:0.7"

# Adjust relational importance in statement
braim statement update-weights 5 --weights "2:0.7,4:0.3"

# Update unary assertion weight (remains 1.0)
braim statement update-weights 6 --weights "2:1.0"
```

Use case: LLM creates statements with 1/count weights; humans can later adjust based on semantic analysis.

### Delete Node or Statement

@[--force flag bypasses dependency safety check] source: src/main.rs:DeleteStatement

```bash
braim concept delete 1
braim statement delete 5 --force
```

### Import from Another Project

@[Import full dependency chains with auto ID remapping] source: src/graph.rs:import_graph()

@[Deduplicates on exact match: atomics/compounds by (name, domain), statements by (text, dependency_ids)] source: src/graph.rs:import_graph() dedup logic

@[Resets verification_status to Unproven; verified_by cleared] source: src/graph.rs:import_graph()

@[Domain mapping allows importing from projects with different domain taxonomies] source: src/main.rs:Import domain_map

```bash
# Basic import (imports all nodes)
braim import /path/to/other/.braim

# Import with domain mapping (Finance → Billing)
braim import /path/to/other/.braim --domain-map "Finance:Billing"

# Filter by domain
braim import /path/to/other/.braim --filter-domain "Billing"

# Import only Proven nodes
braim import /path/to/other/.braim --only-proven

# Multiple domain mappings
braim import /path/to/other/.braim \
  --domain-map "Finance:Billing" \
  --domain-map "Records:Audit"
```

### Look Up Concept

@[Returns all nodes containing concept, ranked by propagated weight] source: src/graph.rs:lookup()

```bash
braim lookup "Voice"
```

### Query Multiple Concepts

@[Returns common ancestor node connecting all terms; score 1.0 means exact match] source: src/graph.rs:query()

```bash
braim query "Voice,Charge,Account"
```

### Find Paths Between Concepts

@[Returns all semantic paths ranked by weight; registers gap if no path exists] source: src/graph.rs:perspective()

```bash
braim perspective "Voice" "Account"
```

### List Nodes

```bash
braim list
braim list --domain "Billing"
braim list --type "statement"
```

### Version Management

```bash
braim version save "added monthly voice charge rule"
braim version list
braim version restore 2
```

### Audit

@[Shows orphan nodes, pending concepts, and zero-path gaps] source: src/graph.rs:audit()

@[Audit output groups statements by verification status: Proven, Partial, Unproven, Inferred, and Invalidated sections] source: src/main.rs:print audit formatting

```bash
braim audit
```

### Serve Web Viewer

@[Embedded HTTP server on port 8000 (configurable with --port)] source: src/main.rs:serve_viewer()

```bash
braim serve
braim serve --port 3000
```

@[Opens interactive visualization in web browser] source: viewer.html

@[Viewer uses color coding: green=Atomic, blue=Compound, green=Proven, amber=Partial, gray=Unproven, purple=Inferred, red=Invalid] source: viewer.html getStatementColor()

---

## Workflow: Adding Domain Knowledge

Example: "A monthly voice charge applies to an account"

```bash
# 1. Check what exists
braim list

# 2. Add atomics (each with domain and source)
braim concept add "Monthly" --domains "Billing" --sources "code"
braim concept add "Voice" --domains "Services" --sources "code"
braim concept add "Charge" --domains "Billing" --sources "code"
braim concept add "Account" --domains "Instance" --sources "code"

# 3. Add compound (reuse or create)
braim concept add "Voice Charge" --domains "Billing,Billing" --sources "code,code" --depends "3:0.5,1:0.5"

# 4. Add statement from initial source
braim statement add "Monthly voice charge applies to Account" \
  --domains "Rules,Rules" --sources "code,code" \
  --depends "2:0.5,5:0.5"

# 5. Verify with independent sources
braim statement verify 6 "Docs" --note "Billing section 4.1"
braim statement verify 6 "Schema" --note "charges table migration"

# 6. Refine weights (optional)
# Initial: LLM uses equal split. Review reveals Voice Charge is more important
braim statement update-weights 6 --weights "2:0.3,5:0.7"

# 7. Save checkpoint
braim version save "added monthly voice charge rule"
```

---

## Workflow: Importing Knowledge from Another Project

```bash
# 1. Check what exists in target
braim list

# 2. Import with domain mapping (Finance domain in source → Billing in target)
braim import /colleague/project/.braim \
  --domain-map "Finance:Billing" \
  --filter-domain "Billing" \
  --only-proven

# 3. Review imported nodes
braim list --domain "Billing"

# 4. Verify imported statements in your context
braim statement verify 10 "YourDocs" --note "Verified against internal policy"

# 5. Save checkpoint
braim version save "imported billing concepts from colleague project"
```

---

## Graph Structure

| Symbol | Node type | Dependencies | Weight constraint |
|--------|-----------|---|---|
| `●` | Atomic | none | N/A |
| `◉` | Compound | 2+ existing nodes | sum = 1.0 |
| `▶` | Statement | 1+ units (1=property, 2+=relation) | sum = 1.0 |

@[Weight propagation is multiplicative along paths] source: src/graph.rs:lookup() weight multiplication

Example: Query "Voice" across graph

| Node | Path | Score |
|---|---|---|
| Voice Charge | Voice → Voice Charge | 0.5 |
| Monthly Voice Charge | Voice → Voice Charge → Monthly Voice Charge | 0.45 |
| Statement | Voice → Voice Charge → Statement | 0.25 |

---

## Verification Status Reference

| Status | Sources | Meaning |
|---|---|---|
| Unproven | 1 | Single source; no independent corroboration |
| Partial | 2 | Two domains provide evidence; threshold not met |
| Proven | 3+ | Rule of 3 satisfied; multiple independent domains confirm |
| Inferred | N/A | Derived relationship; not claimed by external source |
| Invalid | (any) | Refuted or retracted; flagged with reason and timestamp |

@[Inferred statements cannot be verified and retain sources=["inferred"]] source: src/graph.rs:add_statement() --inferred branch

---

## Import Feature

### What Gets Imported

@[Full dependency chains: atomics → compounds → statements in dependency order] source: src/graph.rs:import_graph()

@[Nodes with missing dependencies are automatically skipped] source: src/graph.rs:import_graph() dependency check

### ID Remapping

@[Source IDs automatically remap to target IDs to avoid collisions] source: src/graph.rs:import_graph() id_mappings

@[Manifest shows mapping for all imported nodes] source: src/main.rs:Import handler

### Deduplication

@[Exact match only: atomics/compounds match by (name, domain), statements by (text, dependency_ids)] source: src/graph.rs:import_graph()

@[Duplicate detection is case-insensitive for names] source: src/graph.rs:import_graph() to_lowercase()

@[Duplicates are skipped; target version is always kept] source: src/graph.rs:import_graph() dedup logic

### Domain Mapping

@[Maps source domains to target domains before deduplication] source: src/graph.rs:import_graph() domain remapping

#[Domain mapping enables deduplication across projects with different domain taxonomies] based_on: @[domain remapping applied before dedup]

Example: Source project uses "Finance", target uses "Billing"
```bash
braim import /source --domain-map "Finance:Billing"
# Now Finance concepts map to Billing, enabling dedup by (name, Billing)
```

### Verification Reset

@[All imported nodes have verification_status reset to Unproven] source: src/graph.rs:import_graph()

@[verified_by HashMap is cleared for all imported nodes] source: src/graph.rs:import_graph()

#[Verification is reset because constants changes to source projects could invalidate claimed truths] based_on: @[reset behavior], user requirement

### Filtering

@[--filter-domain: only imports nodes with specified domain] source: src/main.rs:Import filter_domain

@[--only-proven: only imports nodes with verification_status=Proven] source: src/main.rs:Import only_proven

#[Strict filtering: if a node's dependencies are filtered out, the node is skipped] based_on: @[dependency check in import_graph]

---

## Conventions

@[Always check before adding: run `braim list` or `braim lookup <term>` to avoid duplicates] source: evidence capture discipline rule

@[Add atomics before compounds before statements; dependencies must exist first] source: src/graph.rs dependency validation

@[If 3+ nouns needed in statement, compose related nouns into compound first, then use compound in statement] source: src/graph.rs statement validation

@[Single-concept statements are valid assertions: "Pets are allowed" depends on (Pets: 1.0)] source: user clarification

@[Save named checkpoint after meaningful batch of additions] source: braim version save command

---

## Integration with rctags

?[How should code symbols map to braim compounds?] evidence_needed: rctags query output, braim compound structure alignment

#[Code entities (functions, types, endpoints) typically become compounds depending on multiple atomic domain concepts] based_on: rctags symbol discovery patterns + semantic graph composition rules

Example workflow:
```bash
# Find code symbol
rctags query "ProcessPayment"

# Check if compound exists
braim query "ProcessPayment"

# If missing, create compound
braim concept add "ProcessPayment" --domains "code,code" --sources "rctags,rctags" --depends "ID1:0.5,ID2:0.5"
```

---

## Graph Inspection

@[Gap register automatically records zero-path pairs between active concepts] source: src/graph.rs:perspective() gap registration

Run `braim audit` to review:
- Orphan nodes (active, unreachable)
- Pending concepts (declared, unconnected)
- Zero-path gaps (concepts with no connecting paths)

#[Gap register guides investigation: which concept pairs should be connected?] based_on: @[perspective auto-gaps], @[audit output grouping]

---

## Persistence

@[Graph persists to .braim/current.json after every mutation] source: src/main.rs:save_graph()

@[Use `braim version save "description"` to create named checkpoints] source: src/graph.rs:version save command

@[Restore with `braim version restore <number>`] source: src/graph.rs:version restore command
