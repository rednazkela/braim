mod bootstrap;
mod dream;
mod graph;
mod manifest;
mod tips;
// Without the embeddings feature the module compiles but only its pure helpers
// are reachable from tests; suppress dead-code noise in that configuration.
#[cfg_attr(not(feature = "embeddings"), allow(dead_code))]
mod embed;

use clap::{Parser, Subcommand};
use dream::{Candidate, DreamOptions, Strategy};
use graph::{Braim, NodeType, AddSourceResult, VerificationStatus};
use std::collections::HashMap;

#[derive(Parser)]
#[command(name = "braim")]
#[command(about = "BRAIM: Semantic knowledge graph CLI for building verified domain models")]
#[command(long_about = "BRAIM stores concepts, statements, and relationships in a semantic graph with native verification.\n\n\
REQUIRED RULES:\n\
  1. braim always uses cwd/.braim (or override with --data-dir)\n\
  2. Never use jq or other tools directly on current.json\n\
  3. Use 'braim version save' after each batch of commands to checkpoint work\n\
  4. Look for domains before adding nodes (use 'braim domains' to avoid fragmentation)\n\
  5. Atomic labels use 'Concept: description' format (e.g., 'Library: public lending institution');\n\
     compound names use space-separated words, never snake_case or camelCase (e.g., 'Credit Card Payment')\n\
  6. All sources MUST have typed prefix: code:, doc:, schema:, config:, transcript:, test:, phase_N:, agent:, narrative:, logic:, inference:\n\
  7. --depends weights must sum to 1.0. --domains and --sources are free-count (no parity required)\n\
  8. Invalidating a statement CASCADES to all transitive dependents — review impact before applying\n\
  9. Use ASYMMETRIC --depends weights when dependencies have unequal importance.\n\
     Default-even split (e.g. 0.5,0.5 or 0.25×4) means \"no opinion about importance\"\n\
     — a code smell when one dep is clearly more central. Weights propagate\n\
     multiplicatively along paths; query/perspective/proximity scores depend on them.\n\
10. When two statements about the same subject disagree, mark them contested\n\
    via 'statement contradict' rather than asserting one as fact. Resolve via\n\
    a third PRIMARY source (auto) or 'statement resolve-contradiction' (manual).\n\n\
SOURCE TYPES (verification determined by PRIMARY type diversity):\n\
  PRIMARY (independent evidence):   code:, doc:, schema:, config:, transcript:, test:\n\
  SECONDARY (derived or contextual): phase_N:, agent:, narrative:\n\
  TERTIARY (logical derivation):     logic:, inference:\n\n\
VERIFICATION STATUS (auto-calculated; can only be LOWERED by dependencies, never raised):\n\
  Source-derived (from your --sources):\n\
    ✗   Unproven         (0 PRIMARY types)   → claim, not trusted\n\
    ⚠   Contested        (active contradiction) → hidden; use --include-contested\n\
    ✓   Partial          (1 PRIMARY type)    → fact, use with caution\n\
    ✓✓  Proven           (2 PRIMARY types)   → fact, verified\n\
    ✓✓✓ ProvenStrong     (3+ PRIMARY types)  → fact, strongly verified\n\
    ✗✗  Invalid          (via 'statement invalidate' or lost contradiction) → refuted\n\n\
  INHERITANCE RULE: final status = MIN(source-derived, weakest statement dependency)\n\
    • Any unproven --depends statement caps your new statement at unproven\n\
    • Any invalid  --depends statement makes your new statement invalid (full propagation)\n\
    • Concept dependencies (atomic/compound) are excluded from inheritance\n\n\
  To raise verification: add more PRIMARY-typed sources from different types.\n\
  Use 'braim statement verify-suggest <ID>' to get concrete candidate sources.\n\n\
CORE CONCEPTS (node_type values):\n\
  • atomic                — base concept ('Payment', 'Invoice')\n\
  • compound              — groups 2+ atomics (must have >1 dependency)\n\
  • claim                 — unproven statement, hidden from default queries\n\
  • fact                  — partial/proven/proven_strong statement, returned by default\n\
  • contested_statement   — disputed; hidden unless --include-contested; resolves to fact or invalid\n\
  • invalid_statement     — refuted statement, hidden unless --include-invalid\n\
  • source                — first-class source entity (type + location + ingested_at)\n\n\
TAXONOMY & VALIDATION (prevent common hygiene issues):\n\
  All validations default to WARN (write succeeds + stderr notice). Use --strict-* flags to REJECT.\n\
  \n  Duplicate source strings:\n\
    Issue: Same source repeated (doc:a.md,doc:a.md,doc:a.md).\n\
    Impact: Inflates source diversity counts; looks like 3 sources but proves nothing new.\n\
    Solution: Use distinct citations (line numbers: doc:a.md:10, doc:a.md:45).\n\
    Flag: --strict-sources (statement add, concept add)\n\
  \n  PRIMARY+TERTIARY mix:\n\
    Issue: Combining evidence (code:, doc:) with derivations (inference:, logic:) on same node.\n\
    Impact: Muddies verification semantics—unclear if evidence or reasoning drives the status.\n\
    Solution: Keep evidence separate. Record reasoning in label or as dependent inference statement.\n\
    Flag: --strict-sources (statement add, concept add)\n\
  \n  Duplicate domain entries:\n\
    Issue: Same domain repeated (payment,payment,payment).\n\
    Impact: Inflates occurrence counts; obscures true domain membership for queries.\n\
    Solution: Use distinct domains (e.g., payment,operations,finance).\n\
    Note: Warning suppressed in single-domain graphs where uniform repetition is expected.\n\
    Flag: --strict-domains (statement add) to reject instead of warn.\n\
  \n  Stale gap register entries:\n\
    Automatic: Gap register auto-clears when statements connect previously-separate concepts.\n\
    Example: Gap registered between A↔B; add statement --depends \"A:0.5,B:0.5\" → gap cleared.\n\
    Caveat: Heuristic only. Connecting statement may not resolve semantic gap (investigate if unsure).\n\
  \n  Multi-word atomic decomposition hints:\n\
    Automatic: Adding atomic \"Library Card\" when \"Library\" and \"Card\" exist triggers hint.\n\
    Hint: \"Consider as compound depending on Library (ID:X) and Card (ID:Y)\"\n\
    Impact: Compound form enables proper traversal and weight propagation in queries.\n\n\
DEPENDENCY WEIGHTS (express importance, drive traversal scoring):\n\
  --depends \"ID:weight,ID:weight,...\" weights MUST sum to 1.0\n\
  Each weight is the share of importance that dependency carries in this node.\n\n\
  Effect on traversal commands:\n\
    query <term>          → results ranked by edge weight to the queried concept\n\
    perspective A → B     → product of edge weights along A→B path\n\
    proximity A → B       → shortest path with cumulative multiplicative weight\n\n\
  Asymmetry expresses real semantics:\n\
    Credit Card depends on \"Credit:0.3,Card:0.7\"\n\
      → query \"Card\" ranks Credit Card at 0.7  (Card more central)\n\
      → query \"Credit\" ranks Credit Card at 0.3  (Credit less central)\n\
    Credit Card depends on \"Credit:0.5,Card:0.5\"\n\
      → both queries rank Credit Card at 0.5  (no semantic differentiation)\n\n\
  Multiplicative dissolution along chains:\n\
    Card → Credit Card (weight 0.5) → Credit Card Charge (weight 0.5)\n\
    → perspective Card → Credit Card Charge = 0.5 × 0.5 = 0.25\n\
    → Card is \"dissolved\" in Credit Card Charge — present but not central.\n\n\
  When to use asymmetric weights:\n\
    • Compound has a \"primary\" atomic + \"modifier\" atomics\n\
      → primary 0.6-0.8, modifiers split the remainder\n\
    • Statement primarily about one concept but references others\n\
      → main concept gets the bulk, others 0.1-0.2 each\n\
    • Statement combines equal partners\n\
      → even split is correct (don't force asymmetry where none exists)\n\n\
  Default-even is NOT always wrong — only when used as the lazy default for\n\
  semantically asymmetric relationships.\n\n\
QUERY DEFAULTS (filter flags compose orthogonally):\n\
  (default)                                  → facts only\n\
  --include-claims                           → facts + claims\n\
  --only-claims                              → claims only (overrides default)\n\
  --include-contested                        → facts + contested_statements\n\
  --include-invalid                          → facts + invalid_statements\n\
  --include-claims --include-invalid         → full audit view\n\
  --min-trust partial|proven                 → filter by verification level\n\
  --primary-only                             → only statements with ≥1 PRIMARY source\n\n\
  Concepts (atomic/compound) are always returned regardless of these flags.\n\n\
SEMANTIC SIMILARITY (included in the default build):\n\
  braim similar \"<text>\"            → nearest labels by MEANING (zero shared words ok)\n\
  braim similar \"<label>\" --dedup   → write-time duplicate check (floor 0.8)\n\
  --check-dupes on concept/statement add → same check inline, advisory warn\n\
  braim query ... (no hits)          → automatic semantic fallback suggestions\n\
  braim audit --semantic             → near-duplicate pairs (>=0.80) + label echoes:\n\
                                       statements restating their own dependency (>=0.75)\n\
  All ADVISORY — augments, never overrides, the verification lifecycle.\n\
  Index is a sidecar (.braim/embeddings.json); only changed labels re-embed.\n\n\
WORKFLOW:\n\
  # Discover existing graph\n\
  braim domains\n\
  braim query \"Payment\"\n\n\
  # Create concept and statement\n\
  braim concept add \"Refund\" --domains billing --sources \"code:refund.rs\"\n\
  braim statement add \"Refund extends Payment\" \\\n\
    --domains billing --sources \"code:refund.rs,doc:billing.md\" \\\n\
    --depends \"1:0.5,2:0.5\"\n\
  # → Status: PROVEN (2 PRIMARY types); --domains is free-count, 1 domain ok with 2 sources\n\n\
  # Upgrade an unproven statement\n\
  braim statement verify-suggest 42\n\n\
  # Query\n\
  braim query \"Refund\"                     # facts only\n\
  braim query \"Refund\" --min-trust proven  # high-trust only\n\
  braim query \"Refund\" --include-claims    # broader exploration\n\n\
  # Inspect + checkpoint\n\
  braim node 42\n\
  braim version save \"milestone description\"\n\n\
CONTRADICTION RESOLUTION (when sources disagree):\n\
  Two statements about the same subject can be marked contested:\n\
    braim statement contradict <stmt_A> <stmt_B> --reason \"...\"\n\
  Both move to 'contested' state — hidden from default queries.\n\n\
  Resolution:\n\
    • Add a third PRIMARY source to one side → auto-resolves to fact;\n\
      the unsupported side becomes invalid (cascades to its dependents).\n\
    • Or explicit: braim statement resolve-contradiction <winner> <loser>\n\
      --reason \"...\"\n\n\
  Contested statements:\n\
    • Cannot promote past 'contested' until resolved\n\
    • Surface via 'braim query <term> --include-contested'\n\n\
CAUSAL CHAINS (Five Whys):\n\
  Link a statement to its cause (consequent → cause), directional and unweighted:\n\
    braim why-add <consequent> --because <cause> [--source \"narrative:...\"]\n\
  Walk the chain to its root cause:\n\
    braim why <statement_id>\n\
  Validate a link with the classical inverse test:\n\
    braim why-test <id>          # cause confirmed (PASS)\n\
    braim why-test <id> --fail   # consequent persists without cause (refutes the link)\n\
  Reassign a cause (cardinality keeps one per statement — remove, then re-add):\n\
    braim why-remove <id>        # detach the current cause\n\
    braim why-add <id> --because <new_cause>\n\
  Rules: statement endpoints only; one outgoing cause per statement (competing\n\
  causes go through contradicts); cycles rejected; depth >= 7 warns, > 10 rejects.\n\
  perspective/proximity traverse because_of (cause -> consequent, full weight)\n\
  alongside depends_on; query stays depends_on-only. Concept-to-concept paths are\n\
  unaffected since because_of endpoints are statements.\n\n\
FOR AGENTS:\n\
  This help text is the authoritative usage contract. Re-read it after any prompt-context\n\
  reset or when uncertain. Constraints here are structural; constraints in your prompt are\n\
  aspirational.\n\n\
  Common mistakes:\n\
    • Paraphrasing verbatim claims in --label (preserve exact wording)\n\
    • Adding SECONDARY sources to look verified (only PRIMARY types raise verification)\n\
    • Creating 'compound' with single dependency at weight 1.0 (use atomic or a statement)\n\
    • Expecting query to return your just-created unproven statements (use --include-claims)\n\
    • Invalidating a load-bearing node without checking cascade size first\n\
    • Using default-even weights (0.5/0.5, 0.25×4) when one dependency is clearly\n\
      more central — express importance via asymmetric weights so query results,\n\
      perspective paths, and proximity scores reflect actual semantic structure.\n\
      See DEPENDENCY WEIGHTS section.\n\
    • Putting line numbers in source node labels (braim source add 'file.rs:42')\n\
      — node labels are stable identity; put ranges in --sources metadata strings.\n\
    • Deleting and recreating a compound to change dependencies — use\n\
      'braim concept update-deps <id> --add/--remove/--set' to preserve node ID\n\
      and all referencing statements.")]
struct Cli {
    #[arg(global = true, long, default_value = ".braim")]
    data_dir: String,

    #[arg(global = true, long, help = "Suppress tips and non-error stderr output")]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(subcommand, about = "Manage atomic and compound concepts")]
    Concept(ConceptCommands),
    #[command(subcommand, about = "Create and manage statements (claims linking concepts)")]
    Statement(StatementCommands),
    #[command(subcommand, about = "Manage source entities")]
    Source(SourceCommands),
    #[command(about = "Find a single concept by name or ID", long_about = "Lookup: Find exact or fuzzy match for a concept by name or ID.\n\nExamples:\n  braim lookup Payment                           # Exact or fuzzy match with related nodes\n  braim lookup charge                            # Fuzzy: finds 'Charge', 'Charge Service'\n  braim lookup Payment --exact                   # Fast exact match only (O(1) lookup)\n  braim lookup Payment --no-related              # Skip related node enumeration\n  braim lookup payment --include-claims          # Show both facts and claims\n\nBy default, shows FACTS only (verified statements with ≥1 PRIMARY source).\nUse --include-claims to show CLAIMS (unproven statements with 0 PRIMARY sources).\n\nOutput shows:\n  • Badge (✓✓, ✓, ✗) indicating verification status\n  • ID, domains, label\n  • Immediate neighbors: nodes this concept depends on (up to 10)\n  • Immediate neighbors: nodes that reference this concept (up to 10)\n\nPerformance:\n  --exact: Fast path, skips fuzzy matching. Use when you know the exact name.\n  --no-related: Skip related node enumeration for instant results (shows concept only).\n  Without flags: Full lookup with fuzzy matching and up to 10 neighbors per category.")]
    Lookup {
        term: String,
        #[arg(long, help = "Include claims (unproven statements) in results")]
        include_claims: bool,
        #[arg(long, help = "Only show claims (unproven statements)")]
        only_claims: bool,
        #[arg(long, help = "Include invalid statements (excluded by default)")]
        include_invalid: bool,
        #[arg(long, help = "Exact match only (skip fuzzy matching)")]
        exact: bool,
        #[arg(long, help = "Skip related nodes enumeration")]
        no_related: bool,
    },
    #[command(about = "Query for bidirectional connections between concepts", long_about = "Query: Find ANY connection paths between multiple concepts (bidirectional).\n\nFormat: comma-separated list of terms\n\nExamples:\n  braim query \"Payment,Invoice\"                           # All connections\n  braim query \"Payment\" --min-trust proven               # Only verified (2+ PRIMARY)\n  braim query \"Payment\" --primary-only                   # Only PRIMARY sources\n  braim query \"Payment\" --include-claims                 # Show facts and claims\n  braim query \"charge,account,payment\" --only-claims     # Unproven statements only\n\nBy default: returns FACTS only (verified statements with ≥1 PRIMARY source).\n\nTrust Levels:\n  --min-trust partial  = Partial + Proven + ProvenStrong (≥1 PRIMARY source)\n  --min-trust proven   = Proven + ProvenStrong (≥2 PRIMARY sources)\n\nUse 'perspective' for directed paths from term1 → term2.")]
    Query {
        terms: String,
        #[arg(long, help = "Include claims (unproven statements) in results")]
        include_claims: bool,
        #[arg(long, help = "Only show claims (unproven statements)")]
        only_claims: bool,
        #[arg(long, help = "Filter by minimum verification level: partial, proven")]
        min_trust: Option<String>,
        #[arg(long, help = "Only show nodes with at least 1 PRIMARY source")]
        primary_only: bool,
        #[arg(long, help = "Include invalid statements (excluded by default)")]
        include_invalid: bool,
        #[arg(long, help = "Include contested statements (hidden by default)")]
        include_contested: bool,
        #[arg(long, help = "If concept-graph traversal finds nothing, fall back to embedding search by meaning (default build; absent only with --no-default-features)")]
        semantic: bool,
    },
    #[command(about = "Find shortest connection between two concepts", long_about = "Proximity: Find the shortest path connecting term_a to term_b.\n\nExamples:\n  braim proximity Payment Invoice\n  braim proximity \"Voice Charge\" Account\n\nShows hop count and intermediate concepts.\n\nTraverses both depends_on (compositional, weighted) and because_of (causal,\nunweighted — followed cause → consequent at full weight, refuted links skipped).\nbecause_of endpoints are statements, so concept-to-concept paths are unaffected;\npass statement IDs to follow a causal chain.")]
    Proximity {
        term_a: String,
        term_b: String,
    },
    #[command(about = "Show directed paths from one concept to another", long_about = "Perspective: Show how concept A influences/leads to concept B (directed).\n\nExamples:\n  braim perspective Payment Account\n  braim perspective Invoice PaidStatus\n\nUnlike Query (bidirectional), Perspective only shows paths in A→B direction.\nUses multiplicative weight propagation: relationship_strength = product of edge weights along path.\n\nTraverses both depends_on and because_of (causal) edges. because_of is followed\ncause → consequent at full weight (1.0, unweighted); refuted causal links are\nskipped. Since because_of endpoints are statements, concept-to-concept paths are\nunchanged — pass statement IDs to trace a causal chain. (query stays depends_on-only.)")]
    Perspective {
        term_a: String,
        term_b: String,
    },
    #[command(about = "Inspect a node by ID with full details and related nodes", long_about = "Node: Display detailed information about a specific node by ID.\n\nUsage:\n  braim node 42           # Show node 42's details\n  braim node 42 --related # Show node 42 + all nodes it depends on/is depended by\n\nOutputs: ID, label, type, domains, sources, verification status, and dependency graph.\nUse node lookup to map names to IDs first: braim lookup \"concept name\"")]
    Node {
        id: u32,
        #[arg(long, help = "Show nodes that depend on or are depended by this node")]
        related: bool,
    },
    #[command(subcommand, about = "Manage versioning/checkpoints of the knowledge graph")]
    Version(VersionCommands),
    #[command(about = "List all domains in the graph with concept counts", long_about = "Domains: Discover all existing domains to avoid creating duplicates.\n\nUsage:\n  braim domains\n\nOutput: Alphabetical list of domains with count of concepts in each.\n\nWhy: LLMs and users should check existing domains before creating new ones.\nSlightly different domain names (e.g., 'payment' vs 'payments', 'Payment Domain')\nfragment the graph and reduce discoverability. Always reuse existing domains.")]
    Domains,
    #[command(about = "Audit the graph for consistency, gaps, and verification issues", long_about = "Audit: Scan the entire graph for problems and verification status.\n\nChecks:\n  • Orphan nodes (active, unreferenced, no dependencies)\n  • Pending nodes (declared but unintegrated)\n  • Statements grouped by verification status:\n      ✓✓✓ ProvenStrong (3+ PRIMARY sources)\n      ✓✓ Proven (2+ PRIMARY sources)\n      ✓ Partial (1 PRIMARY source)\n      ✗ Unproven (0 PRIMARY sources)\n  • Invalid statements (refuted claims)\n  • Deprecated nodes still referenced\n  • Gap register: zero-path relationships\n  • Weight constraint violations (must sum to 1.0)\n  • Causal-edge (because_of) health:\n      - Refuted links: edges a failed inverse test marked invalid\n      - Re-investigation flags: statements above an invalidated cause\n      - Untested links: active because_of edges with no inverse test\n      - Unverified roots: chains bottoming out below proven\n\nOutput organization:\n  1. Orphan nodes needing integration\n  2. Pending nodes (incomplete)\n  3. Gap register (missing connections)\n  4. Deprecated nodes still in use\n  5. Causal-edge health (refuted / flagged / untested / unverified roots)\n  6. Statement verification status breakdown\n  7. Invalid statements with reasons\n\nUse audit regularly to track:\n  • Verification coverage (% proven vs unproven)\n  • Integration status (orphans, pending)\n  • Consistency issues (gaps, weight violations)\n  • Deprecation problems (deprecated referenced)\n\nSemantic checks (--semantic, requires --features embeddings):\n  • Near-duplicates: unconnected node pairs with label cosine >= 0.80\n  • Label echoes: statements restating a dependency's label (cosine >= 0.75)\n    — single-concept elaborations that add no relationship\nBoth reuse the .braim/embeddings.json sidecar index and are ADVISORY.")]
    Audit {
        #[arg(long, help = "Embedding-based checks: near-duplicate pairs and label echoes (default build; absent only with --no-default-features)")]
        semantic: bool,
    },
    #[command(about = "List all nodes (optionally filtered by domain or type)", long_about = "List: Display all nodes in the graph.\n\nExamples:\n  braim list                        # All nodes\n  braim list --domain payment       # Only 'payment' domain\n  braim list --type statement       # Only statements\n  braim list --domain acme --type atomic  # Combine filters\n\nOutput: ID, label, type, domains, source count, verification status.")]
    List {
        #[arg(long, help = "Filter by domain")]
        domain: Option<String>,
        #[arg(long, help = "Filter by type (atomic, compound, statement)")]
        r#type: Option<String>,
        #[arg(long, help = "Filter by metadata key=value (e.g. scope=cognitivex_flow)")]
        meta: Option<String>,
    },
    #[command(about = "Start HTTP server for HTML graph viewer", long_about = "Serve: Launch web interface for browsing the graph.\n\nUsage:\n  braim serve          # Start on port 8000 (default)\n  braim serve --port 9000\n\nThen open: http://localhost:8000\n\nFeatures:\n  • Visual graph navigation\n  • Search by name or ID\n  • Verification status colors\n  • Node size by verification strength (word-cloud layout)\n  • Filter by domain/source\n  • Click nodes to inspect details")]
    Serve {
        #[arg(long, default_value = "8000", help = "Port to listen on")]
        port: u16,
    },
    #[command(about = "Import concepts/statements from external source", long_about = "Import: Load graph data from JSON/CSV or other braim exports.\n\nUsage:\n  braim import data.json\n  braim import graph.csv --filter-domain payment\n  braim import backup.json --only-proven\n  braim import data.json --domain-map \"old:new,legacy:current\"\n  braim import /other/.braim --full   # full-fidelity (trusted) import\n\nDefault mode treats the source as UNTRUSTED: verification resets to unproven,\nsource entities are dropped, and because_of/contradicts edges are not carried.\n\n--full is the TRUSTED self-import for consolidating your own graphs:\n  • verification status and verified_by preserved\n  • source entities imported, statement source_ids remapped\n  • because_of and contradicts edges carried (endpoints remapped)\n  • dedup hits UNION the duplicate's sources into the target and recompute\n    its status — corroboration stacks, promotion still needs distinct\n    PRIMARY types (braim ID:185/190)\n\n--only-proven admits proven AND proven_strong nodes.\n\nAfter import, run: braim version save \"imported from X\"")]
    Import {
        source: String,
        #[arg(long, help = "Only import nodes from specified domain")]
        filter_domain: Option<String>,
        #[arg(long, help = "Only import proven/proven_strong statements")]
        only_proven: bool,
        #[arg(long, help = "Remap domain names during import (format: old:new,old2:new2)")]
        domain_map: Vec<String>,
        #[arg(long, help = "Full-fidelity trusted import: preserve verification, carry source entities and because_of/contradicts edges, union duplicate sources")]
        full: bool,
    },
    #[command(about = "Migrate legacy statement node_types to claim/fact/invalid_statement", long_about = "Migrate Node Types: Rewrite all `statement` node_type values to claim/fact/invalid_statement based on verification_status.\n\nPer BRAIM_NODE_TYPE_CLAIM_FACT_SPEC §6 — required after upgrading from versions that stored all statement-family nodes as `statement`.\n\nMapping:\n  verification_status == invalid          → invalid_statement\n  verification_status == unproven         → claim\n  verification_status in {partial, proven, proven_strong} → fact\n\nIdempotent. Safe to run multiple times.")]
    MigrateNodeTypes,

    #[command(name = "migrate-refutation", about = "Undo the pre-3.3 refutation cascade (dry by default)", long_about = "Migrate Refutation: repair nodes the OLD cascade marked invalid as collateral.\n\nUsage:\n  braim migrate-refutation           # report what would change, touch nothing\n  braim migrate-refutation --apply   # write the repair\n\nBefore BRAIM_DEPENDENCY_INHERITANCE_SPEC §3.3, invalidating a statement cascaded\n`Invalid` to every transitive dependent. Those nodes were never refuted on their\nown evidence — they were collateral, and they carry `invalid_reason:\ndepends_on_invalidated:<id>` to say so. §3.3 replaced that rule: a refuted\ndependency now leaves the cap set, and the dependent settles at whatever its own\nsources support. This command applies the new rule to nodes refuted under the\nold one.\n\nThe reason string is fully re-derivable, so nothing is lost by recomputing it\naway. Each repaired node is stamped `support_withdrawn_by` so the affected set\nstays a reviewable worklist rather than a silent verdict.\n\nDRY BY DEFAULT. Repairing hundreds of nodes without saying so would be\nindistinguishable from corrupting them (§4.5). Read the report, then --apply.\n\nIdempotent: a second run selects nothing. Checkpoint first — `braim version\nsave` — as with any bulk change.")]
    MigrateRefutation {
        #[arg(long, help = "Write the repair (default: report only)")]
        apply: bool,
        #[arg(long, help = "Emit JSON")]
        json: bool,
    },
    #[command(about = "Publish a domain (plus its dependency closure) into another braim", long_about = "Export: Publish one domain from this working graph into a central braim.\n\nUsage:\n  braim export billing --to ~/.braim_central\n  braim export billing --to ~/.braim_central --include-unproven\n  braim export billing --to ~/.braim_central --domain-map \"billing:sonar_billing\"\n\nThis is the contribute flow (braim ID:232/240): issue-isolated working graphs stay\nper-task, and verified knowledge is published domain-by-domain into central.\n\nWhat crosses:\n  • the domain's nodes PLUS their full dependency closure — concepts, statements,\n    and attached source entities from other domains that the exported statements\n    stand on (self-contained vendored pack, ID:220; fixes the lossy slice ID:180)\n  • because_of and contradicts edges among the exported set\n  • full fidelity: verification status preserved, duplicate sources unioned into\n    existing central nodes so corroboration accumulates (ID:185/190)\n\nDefaults:\n  • floor at PARTIAL: a statement needs at least one PRIMARY source to publish,\n    so evidence-free claims stay home while single-source findings can reach\n    central and corroborate there (braim ID:253). --include-unproven removes\n    the floor entirely.\n\nAfter export, checkpoint central: braim --data-dir <central> version save \"...\"")]
    Export {
        domain: String,
        #[arg(long, help = "Target braim data dir (default: the central recorded by `braim init --team --central`)")]
        to: Option<String>,
        #[arg(long, help = "Also export unproven statements (default floor: partial, i.e. at least one PRIMARY source)")]
        include_unproven: bool,
        #[arg(long, help = "Remap domain names during export (format: old:new,old2:new2)")]
        domain_map: Vec<String>,
    },
    #[command(about = "Set up this project for braim: local graph + agent policy hooks", long_about = "Init: bootstrap a working braim setup in one command.\n\nUsage:\n  braim init --team\n  braim init --team --central ~/.braim_central\n  braim init --team --settings .claude/settings.local.json\n\nWhat it does:\n  • Creates the local graph if absent\n  • Installs the agent policy hooks into .claude/settings.json:\n      UserPromptSubmit -> braim policy perturn      (per-turn marker logging)\n      PreCompact       -> braim policy compaction   (what to keep when compacting)\n  • Records where central lives, so `braim export <domain>` needs no --to\n\nThe hooks invoke `braim policy`, not a shell tool reading an absolute path, so\nthe same settings file works on Linux, macOS, and Windows and the policy stays\nversion-locked to the braim binary enforcing it.\n\nIdempotent: re-running reports what is already present and changes nothing.\nExisting settings and any hooks braim did not add are preserved.\n\nWhy solo-first: a teammate starting out has no graphs, so day-one value is the\nsetup that already works alone — a local graph plus the discipline hooks.\nSharing layers on once several graphs exist (braim ID:223).")]
    Init {
        #[arg(long, help = "Install the team agent setup (currently the only mode)")]
        team: bool,
        #[arg(long, help = "Path or URL of the central braim, recorded for later exports")]
        central: Option<String>,
        #[arg(long, help = "Settings file to write (default: .claude/settings.json)")]
        settings: Option<String>,
    },
    #[command(about = "Print an agent policy payload (used by the hooks braim init installs)", long_about = "Policy: emit an agent-integration policy on stdout.\n\nUsage:\n  braim policy perturn        # UserPromptSubmit payload: per-turn marker logging\n  braim policy compaction     # PreCompact payload: keep IDs and edges, not prose\n  braim policy traits         # evidence-capture discipline, for agent memory\n\nThese are the contracts in policies/, embedded in the binary. Hooks call this\ncommand instead of reading a file, which keeps the wiring free of absolute paths\nand shell tools — portable across platforms and version-locked to this binary.")]
    Policy {
        name: String,
    },
    #[command(subcommand, about = "Dream: surface node pairs an LLM should examine for missing relations")]
    Dream(DreamCommands),
    #[command(about = "Fold a duplicate node into the one that survives", long_about = "MergeNodes: union two duplicate nodes into one, keeping all the evidence.\n\nUsage:\n  braim merge-nodes 42 99      # 42 survives, 99 is folded into it\n\nWhat it does:\n  • Unions the loser's sources, source entities, and verified_by into the winner\n  • Moves every reference: a node that depended on the loser now depends on the\n    winner, with weights SUMMED so a referent that cited both keeps its 1.0 total\n  • Moves because_of, contradicts, and gap-register entries, dropping self-edges\n  • Records merged_from on the winner as an audit trace, then removes the loser\n  • Recomputes the winner's verification — new PRIMARY types may promote it\n\nWhat it deliberately does NOT do:\n  • Merge the loser's dependencies into the winner. That would silently rewrite\n    what the surviving statement asserts, so any difference is REPORTED instead.\n\nRefused when: the nodes are the same, either is invalid (merging would launder\nrefuted evidence into a live node), they are of different kinds (concept vs\nstatement), or either depends on the other (related, not duplicate).\n\nThis is the union-merge the corroboration model assumes (braim ID:190/248):\nbefore it, deduplicating meant update-deps plus delete, which discarded the\nloser's sources entirely.")]
    MergeNodes {
        winner: u32,
        loser: u32,
    },
    #[command(about = "Rename a domain across the graph", long_about = "RenameDomain: Replace a domain name on every node that carries it.\n\nUsage:\n  braim rename-domain Billing braim_demo\n\nEffect:\n  • Every node listing the old domain gets the new name (duplicates collapsed)\n  • In sharded layout, affected nodes re-home into the new domain's shard file;\n    the old current shard is pruned\n  • Versioned snapshots (*.vNNNN.json) are immutable history and keep the old name\n\nRename vs merge: renaming onto an EXISTING domain name merges the two domains —\nverify with evidence first that they mean the same thing (braim ID:244: same-name\ndomains proved to be demo vocabulary vs real billing knowledge).")]
    RenameDomain {
        old: String,
        new: String,
    },
    #[command(about = "Convert this data dir to the sharded per-domain layout", long_about = "Shard: Convert single-file storage (current.json) to the sharded per-domain layout.\n\nLayout after conversion:\n  domains/<domain>-<hash>.json   one file per home domain (a node's home = first domains entry)\n  graph.json                     cross-domain state: dictionary, gaps, edges, counters\n  current.json.pre-shard         archived single-file snapshot (escape hatch)\n\nSemantics (braim ID:217/236):\n  • The in-memory graph stays ONE merged view — queries and traversal are unchanged.\n  • Every mutation rewrites the affected shard files; version save still writes whole-graph vNNNN.json snapshots.\n  • Domain filenames carry a deterministic hash suffix so distinct domains like 'Billing' and 'billing' never collide, including on case-insensitive filesystems (macOS/Windows).\n\nDetection is automatic: any braim command on a dir containing domains/ loads the sharded layout.")]
    Shard,
    #[command(about = "Semantic similarity search over node labels (default build; absent only with --no-default-features)", long_about = "Similar: Embedding-backed nearest-neighbour search over node labels.\n\nComplements `query` (concept-graph traversal): finds nodes by MEANING even with\nzero shared words, where lexical query returns nothing. Strongest as a write-time\nDEDUP check — surface a near-duplicate before adding a new node.\n\nExamples:\n  braim similar \"errors in early stages cascade into later ones\"\n  braim similar \"measuring how similar two texts are\" --top 10 --min-score 0.4\n  braim similar \"Cosine Similarity: vector angle measure\" --dedup   # dedup intent\n\nBuilds/refreshes a sidecar index at .braim/embeddings.json on first run; only\nnodes whose label changed are re-embedded thereafter. ADVISORY: it augments,\nnever overrides, the verification lifecycle. Quality is gated on clean\n'Concept: definition' labels (braim ID:6629).")]
    Similar {
        text: String,
        #[arg(long, default_value = "8", help = "Number of results to return")]
        top: usize,
        #[arg(long, default_value = "0.0", help = "Minimum cosine score to include")]
        min_score: f32,
        #[arg(long, help = "Force a full re-embed of every node")]
        rebuild: bool,
        #[arg(long, help = "Dedup intent: raise the default score floor to 0.8 and flag likely duplicates")]
        dedup: bool,
    },
    #[command(about = "Get/set/increment a node's first-class metadata (braim 6336)", long_about = "Meta: structured, queryable node fields — scope, recurrence, status, affected_feature — NOT label/domain encoded.\n\n  braim meta 6318                          # print all metadata for node 6318\n  braim meta 6318 --set scope=deliverable  # set a key\n  braim meta 6318 --inc recurrence         # increment a numeric key, prints new value\n  braim meta 6318 --unset scope             # remove a key entirely\n\nQuery by metadata:  braim list --meta scope=cognitivex_flow")]
    Meta {
        id: u32,
        #[arg(long, help = "Set key=value (e.g. scope=cognitivex_flow)", conflicts_with_all = ["inc", "unset"])]
        set: Option<String>,
        #[arg(long, help = "Increment a numeric key (e.g. recurrence)", conflicts_with = "unset")]
        inc: Option<String>,
        #[arg(long, help = "Remove a key entirely (e.g. terminal_cause)")]
        unset: Option<String>,
    },

    #[command(name = "why-add", about = "Add a because_of causal edge (Five Whys)", long_about = "Why Add: record that one statement occurs because_of another (consequent → cause).\n\nUsage:\n  braim why-add 42 --because 17\n  braim why-add 42 --because 17 --source \"narrative:investigation_2026-06-19\"\n\nRules:\n  • Both endpoints must be STATEMENTS (not concepts).\n  • One outgoing because_of per statement (single cardinality). If a second cause\n    is suspected, model the competition with 'braim statement contradict'.\n  • Unweighted: each link asserts the principal cause.\n  • Cycles are rejected. Chain depth >= 7 warns; > 10 is rejected.\n  • --source must carry a typed prefix (code:|doc:|...|narrative:).\n\nThis edge is isolated from depends_on: perspective/proximity/query are unaffected.\nWalk the chain with 'braim why <id>'; validate a link with 'braim why-test <id>'.")]
    WhyAdd {
        #[arg(help = "Consequent statement ID (the effect)")]
        consequent: u32,
        #[arg(long = "because", help = "Cause statement ID (why the consequent occurs)")]
        because: u32,
        #[arg(long, help = "Optional typed source for the causal hypothesis (e.g. narrative:...)")]
        source: Option<String>,
    },

    #[command(about = "Walk a because_of chain to its root cause (Five Whys)", long_about = "Why: walk the because_of chain from a statement down to its root cause.\n\nUsage:\n  braim why 42\n\nOutput: the ordered chain (consequent → ... → root cause). Each link shows the\ninherited causal-claim status. The terminal statement is marked root_cause; if it\nis unproven it is flagged a candidate root cause needing verification. Contested\nlinks (an unresolved contradicts edge on a chain member) are flagged but do not\nstop the walk. Follows because_of only — never depends_on.")]
    Why {
        #[arg(help = "Statement ID to walk from")]
        id: u32,
    },

    #[command(name = "why-test", about = "Record an inverse-test result on a because_of edge", long_about = "Why Test: record the classical Five-Whys inverse test on a statement's causal edge.\n\nThe inverse test asks: does the consequent stop occurring when the cause is absent?\n\nUsage:\n  braim why-test 42                                    # pass (cause confirmed)\n  braim why-test 42 --source \"test:ablation_run.txt\"   # pass, with explicit test source\n  braim why-test 42 --fail                             # fail (consequent persists without cause)\n\nPass: logs a test: source on the edge; a both-endpoints-proven link is promoted\n      from partial to proven.\nFail: refutes the causal LINK (marked invalid) without invalidating either\n      statement; the consequent is suggested for re-investigation.")]
    WhyTest {
        #[arg(help = "Consequent statement ID whose causal edge is tested")]
        id: u32,
        #[arg(long, help = "Record a FAILING inverse test (consequent persists without the cause)")]
        fail: bool,
        #[arg(long, help = "Optional typed test source (default: test:inverse_test_passed)")]
        source: Option<String>,
    },

    #[command(name = "why-remove", about = "Remove a statement's because_of edge to reassign its cause", long_about = "Why Remove: detach a statement's outgoing because_of edge so it can be re-pointed at a different cause.\n\nThe single-cardinality rule means a statement keeps exactly one cause; why-add\nrejects a second. To REASSIGN a cause, remove the current edge first, then add\nthe new one:\n  braim why-remove 42            # drop 42's current cause edge\n  braim why-add 42 --because 73  # point 42 at a new cause\n\nIt removes the active outgoing edge; if there is none but a refuted (failed\ninverse-test) edge remains, it clears that instead. Errors when the statement\nhas no outgoing causal edge. Only the link is removed — both statements and the\nrest of the chain stay intact.")]
    WhyRemove {
        #[arg(help = "Consequent statement ID whose cause edge is removed")]
        id: u32,
    },
}

#[derive(Subcommand)]
enum ConceptCommands {
    #[command(about = "Add a new atomic or compound concept", long_about = "Concept Add: Create an atomic or compound concept.\n\nAtomic (base unit) — label MUST follow 'Concept: description' format:\n  braim concept add \"Payment: value transfer between two parties\" --domains payment --sources \"code:payment.rs\"\n  braim concept add \"Library: public institution that lends books\" --domains knowledge --sources \"doc:spec.md\"\n\nCompound (depends on 2+ atomics, with weights summing to 1.0; no format constraint on label):\n  braim concept add \"Credit Card Payment\" --domains payment --sources \"code:card.rs\" --depends \"1:0.6,2:0.4\"\n\nArguments:\n  term: For atomics: 'Concept: description' (e.g. 'Library: public lending institution').\n        For compounds: space-separated name, no snake_case.\n  --domains: Comma-separated domain tags (e.g., payment,finance)\n  --sources: Comma-separated SOURCE_TYPE:location pairs (REQUIRED prefix)\n  --depends: Optional dependencies for compounds (format: \"ID:weight,ID:weight\")\n  --strict-sources: Reject if sources contain duplicates or mix PRIMARY+TERTIARY types (default: warn)\n\nSource Types (verification calculated from PRIMARY sources):\n  PRIMARY: code:, doc:, schema:, config:, transcript:, test:\n  SECONDARY: phase_N:, agent:, narrative:\n  TERTIARY: logic:, inference:\n\nValidation Rules:\n  • Atomic labels must use 'Concept: description' format. Auto-normalized: missing or extra spaces\n    around the colon are fixed silently (e.g. 'Payment:transfer' → 'Payment: transfer').\n    Rejected only when no colon is present or either side (name or description) is empty.\n    Enforces self-documenting nodes: the meaning is encoded in the label, not in a separate statement.\n  • Duplicate sources (same string repeated) are allowed by default but warned.\n    Use --strict-sources to reject. Distinct citations (e.g., line numbers) preferred.\n  • Mixing PRIMARY evidence with TERTIARY derivations on same concept discouraged.\n    Use --strict-sources to reject; prefer keeping evidence separate from reasoning.\n  • Multi-word atomic names decomposable into existing atomics trigger hints.\n    Example: Adding \"Library Card: card granting borrowing rights\" when \"Library\" and \"Card\" exist → suggests compound form.\n\nExamples:\n  braim concept add \"Invoice: document requesting payment for goods or services\" --domains payment --sources \"doc:spec.md\"\n  braim concept add \"Fee: fixed charge applied for a service\" --domains payment --sources \"schema:tables.sql\"\n  braim concept add \"Credit Card: payment card linked to a revolving credit line\" --domains payment --sources \"code:card.rs,doc:card.md\"\n\nVerification Status (auto-calculated):\n  0 PRIMARY sources → unproven (not trusted)\n  1 PRIMARY source → partial (use with caution)\n  2 PRIMARY sources → proven (verified)\n  3+ PRIMARY sources → proven_strong (strongly verified)\n\nWeight constraint: All weights must sum to exactly 1.0. Omit --depends for atomics.")]
    Add {
        term: String,
        #[arg(long, help = "Comma-separated domains (e.g., payment,finance)")]
        domains: String,
        #[arg(long, help = "Comma-separated source identifiers")]
        sources: String,
        #[arg(long, help = "Compound dependencies: \"ID:0.5,ID:0.5\" (weights sum to 1.0)")]
        depends: Option<String>,
        #[arg(long, help = "Reject concepts with duplicate sources or PRIMARY+TERTIARY mix")]
        strict_sources: bool,
        #[arg(long, help = "Advisory: warn if an existing node is semantically near-duplicate (default build; absent only with --no-default-features)")]
        check_dupes: bool,
    },
    #[command(about = "Delete a concept (requires --force unless unused)", long_about = "Concept Delete: Remove a concept from the graph.\n\nUsage:\n  braim concept delete 42         # Fails if concept is referenced\n  braim concept delete 42 --force # Force delete (dangerous, breaks statements)\n\nSafety: Deleting a concept breaks any statements/compounds that depend on it.\nUse --force only if you're certain no statements reference this ID.")]
    Delete {
        id: u32,
        #[arg(long, help = "Force delete even if referenced by statements")]
        force: bool,
    },
    #[command(about = "Update dependency weights for a compound concept", long_about = "Concept UpdateWeights: Adjust how a compound combines its atomic components.\n\nUsage:\n  braim concept update-weights 42 --weights \"1:0.3,2:0.7\"\n\nRules:\n  • Only for compounds (concepts with dependencies)\n  • All weights must sum to exactly 1.0\n  • Format: \"ID:weight,ID:weight\"\n  • IDs must already be dependencies of concept 42\n\nExample: CreditCardPayment (ID 42) depends on Payment (1) and CreditCard (2):\n  Current: 1:0.5, 2:0.5  (equal importance)\n  Update:  1:0.8, 2:0.2  (payment is 80% of identity)")]
    UpdateWeights {
        id: u32,
        #[arg(long, help = "New weights: \"ID:weight,ID:weight\" (must sum to 1.0)")]
        weights: String,
    },
    #[command(about = "Add, remove, or replace dependency edges on a compound concept", long_about = "Concept UpdateDeps: Modify the dependency edges of an existing compound without deleting it.\n\nPreserves node ID and all statements that reference it.\n\nUsage:\n  braim concept update-deps 42 --add \"5:0.3\" --remove \"3\"\n  braim concept update-deps 42 --set \"1:0.6,5:0.4\"\n\nFlags:\n  --add \"ID:weight[,ID:weight]\": Insert new dependency edges\n  --remove \"ID[,ID]\":           Remove existing dependency edges by ID\n  --set \"ID:weight[,ID:weight]\": Replace the full dependency list\n\nRules:\n  • --set is exclusive: --add and --remove are ignored when --set is provided\n  • Weights must sum to 1.0 after the operation\n  • --add errors if the ID is already a dependency (use --remove first)\n  • --remove errors if the ID is not a current dependency\n\nExample: Compound 42 depends on IDs 1 and 3. Replace ID 3 with ID 5 at 0.3 weight:\n  braim concept update-deps 42 --remove \"3\" --add \"5:0.3\"\n  (then adjust remaining weight: ID 1 must be updated via update-weights if needed)")]
    UpdateDeps {
        id: u32,
        #[arg(long, help = "Add dependency edges: \"ID:weight[,ID:weight]\"")]
        add: Option<String>,
        #[arg(long, help = "Remove dependency edges by ID: \"ID[,ID]\"")]
        remove: Option<String>,
        #[arg(long, help = "Replace all dependencies: \"ID:weight[,ID:weight]\" (exclusive with --add/--remove)")]
        set: Option<String>,
    },
}

#[derive(Subcommand)]
enum StatementCommands {
    #[command(about = "Add a statement linking concepts with evidence", long_about = "Statement Add: Create a claim linking concepts with verification sources.\n\nBasic statement with typed sources:\n  braim statement add \"Payment requires Invoice\" \\\n    --domains \"payment,payment\" --sources \"code:rules.rs,doc:spec.md\" \\\n    --depends \"1:0.5,2:0.5\"\n  → Status: PROVEN (2 PRIMARY sources: code + doc)\n\nStatement with SECONDARY (contextual) source:\n  braim statement add \"Security assumption\" \\\n    --domains payment --sources \"narrative:assumption\" \\\n    --depends \"1:1.0\"\n  → Status: UNPROVEN (0 PRIMARY sources, only narrative)\n\nInferred statement (derived, not independently verifiable):\n  braim statement add \"Card Payment implies Security\" \\\n    --depends \"1:1.0\" --inferred\n\nArguments:\n  text: The statement claim\n  --domains: Comma-separated domain tags (free count, no parity with sources or depends)\n  --sources: Comma-separated SOURCE_TYPE:location pairs (required, typed prefixes)\n  --depends: Concept IDs with weights (\"ID:weight,...\" must sum to 1.0)\n  --inferred: Mark as derived, not independently verifiable (uses 'inferred' source)\n  --assume: Skip validation checks\n  --strict-sources: Reject if sources contain duplicates or mix PRIMARY+TERTIARY (default: warn)\n  --strict-domains: Reject if domains contain duplicates (default: warn)\n\nSource Types (PRIMARY sources determine verification):\n  PRIMARY (independent evidence): code:, doc:, schema:, config:, transcript:, test:\n  SECONDARY (contextual): phase_N:, agent:, narrative:\n  TERTIARY (derived): logic:, inference:\n\nValidation Rules (enabled by default with warnings; use --strict-* to reject):\n  • Duplicate sources: Same source string appearing multiple times (e.g., doc:a.md,doc:a.md,doc:a.md).\n    Use distinct citations (line numbers, sections) for meaningful source diversity.\n    --strict-sources rejects; default warns and writes the statement.\n  \n  • PRIMARY+TERTIARY mix: Combining evidence (code:, doc:) with derivations (inference:, logic:) on\n    same statement. Muddies verification semantics. Prefer evidence-only sources here; record reasoning\n    separately via label or as dependent inference statement.\n    --strict-sources rejects; default warns and writes.\n  \n  • Duplicate domains: Same domain repeated (e.g., payment,payment,payment). Inflates\n    occurrence counts and obscures actual domain membership. Use distinct domains.\n    --strict-domains rejects; default warns and writes.\n  \n  • Gap register auto-clear: When statement depends on concepts A and B, any registered gap between\n    them is automatically removed. This heuristic may not reflect true semantic resolution—verify\n    connections are correct before relying on cleared gaps.\n\nVerification (auto-calculated from PRIMARY source count):\n  0 PRIMARY → ✗ UNPROVEN (claim, not trusted)\n  1 PRIMARY → ✓ PARTIAL (fact, use with caution)\n  2 PRIMARY (different types) → ✓✓ PROVEN (fact, verified)\n  3+ PRIMARY (different types) → ✓✓✓ PROVEN_STRONG (fact, strongly verified)\n\nNote: Verification status is capped by dependencies (inherits minimum of all depends_on).")]
    Add {
        text: String,
        #[arg(long, help = "Comma-separated domains")]
        domains: Option<String>,
        #[arg(long, help = "Comma-separated sources")]
        sources: Option<String>,
        #[arg(long, help = "Required: \"ID:weight,ID:weight\" (weights sum to 1.0)")]
        depends: String,
        #[arg(long, help = "Mark as inferred (derived, not independently verifiable)")]
        inferred: bool,
        #[arg(long, help = "Skip validation checks")]
        assume: bool,
        #[arg(long, help = "Reject statements with duplicate sources or PRIMARY+TERTIARY mix")]
        strict_sources: bool,
        #[arg(long, help = "Reject statements with duplicate domains")]
        strict_domains: bool,
        #[arg(long, help = "Advisory: warn if an existing node is semantically near-duplicate (default build; absent only with --no-default-features)")]
        check_dupes: bool,
    },
    #[command(about = "Add verification evidence for a statement", long_about = "Statement Verify: Record evidence that supports a statement.\n\n⚠ NOTE: Verification status is now AUTO-CALCULATED from typed sources at statement creation.\nThis command is maintained for backward compatibility but is rarely needed.\n\nModern approach (preferred):\n  braim statement add \"...\" --sources \"code:a.rs,doc:b.md\" ...\n  → Status auto-calculated to PROVEN (2 PRIMARY sources)\n\nLegacy approach (still supported):\n  braim statement verify 42 wikipedia --note \"https://en.wikipedia.org/wiki/Payment\"\n  braim statement verify 42 rfc --note \"RFC 3501 section 3.2\"\n\nOld Verification Levels (deprecated, kept for audit trail):\n  • 0-1 verified_by domains: Unproven\n  • 2 verified_by domains: Partial\n  • 3+ verified_by domains: Proven\n\nUse statement add with typed sources instead. Sources determine verification automatically.")]
    Verify {
        statement_id: u32,
        #[arg(help = "Domain/source identifier (e.g., wikipedia, rfc, docs)")]
        domain: String,
        #[arg(long, help = "Optional evidence URL or reference")]
        note: Option<String>,
    },
    #[command(about = "Remove a statement", long_about = "Statement Delete: Remove a statement from the graph.\n\nUsage:\n  braim statement delete 42        # Fails if other statements depend on it\n  braim statement delete 42 --force # Force delete (breaks dependents)\n\nSafety: Deleting a statement can break other statements that reference it.")]
    Delete {
        id: u32,
        #[arg(long, help = "Force delete even if referenced")]
        force: bool,
    },
    #[command(about = "Mark a statement as refuted (Invalid)", long_about = "Statement Invalidate: Mark a statement as false/refuted and cascade to dependents.\n\nUsage:\n  braim statement invalidate 42 --reason \"Contradicted by RFC 5321\"\n  braim statement invalidate 99 --reason \"Empirical data shows opposite\"\n\nEffect:\n  • Statement becomes INVALID (✗✗ Invalid in UI, red color)\n  • Does NOT delete the statement, preserves history for audit\n  • Cascades to dependent statements (warns before applying)\n  • Dependent statements are demoted/invalidated transitively\n\nCascade behavior:\n  If statement S is invalidated, all statements depending on S:\n  • Are demoted (verification status lowered)\n  • May become invalid themselves if S was their only evidence\n  • Are listed before confirmation (shows affected count)\n\nExamples:\n  braim statement invalidate 42 --reason \"RFC 5321 supersedes this\"\n  braim statement invalidate 99 --reason \"Contradicted by empirical data\"\n\nUse this when new evidence contradicts a previously verified statement.\nStatement history and original claim are preserved for audit trail.")]
    Invalidate {
        id: u32,
        #[arg(long, help = "Reason why statement is invalid")]
        reason: String,
    },
    #[command(about = "Revive an invalidated statement (inverse of invalidate)", long_about = "Statement Revalidate: Clear the invalid flags on a single statement and recompute its verification_status from sources + dependency inheritance.\n\nUsage:\n  braim statement revalidate 169\n\nEffect:\n  • invalid flag, reason, and timestamp are cleared\n  • verification_status is recomputed from typed sources and valid-dependency inheritance\n  • node_type is reset accordingly (claim / fact)\n  • Does NOT cascade: revive dependents explicitly, in dependency order outward\n\nInvalid dependencies:\n  A dependency that is itself invalid is SKIPPED in the inheritance cap (not allowed to\n  re-poison this node) and reported as a warning. Re-anchor it with 'statement update-deps'\n  so the revival is durable.\n\nUse this to recover from an over-broad invalidate cascade, or when refuting evidence is withdrawn.")]
    Revalidate {
        id: u32,
    },
    #[command(about = "Suggest verification sources for a statement", long_about = "Statement VerifySuggest: Find candidate verification sources for an unproven statement.\n\nProblem: Verifying statements requires agents to manually search for evidence.\nSolution: suggest recommends candidate sources based on domain context and similarity.\n\nUsage:\n  braim statement verify-suggest 42\n  braim statement verify-suggest 5\n\nOutput recommendations (by priority):\n  1. Similar verified statements in same domain\n     → If domain:payment has verified statements, show them\n     → Suggests which sources proved similar claims\n  \n  2. Code locations mentioned in statement\n     → If statement mentions \"messageService.js:110-149\"\n     → Suggests code:src/services/messageService.js:110-149\n  \n  3. Recommended source types by domain\n     → domain:payment → suggests doc: sources (spec links)\n     → domain:security → suggests config: sources (settings)\n     → domain:database → suggests schema: sources (DDL)\n\nWorkflow:\n  1. Create unproven statement: braim statement add \"...\" --sources \"narrative:assumption\" ...\n  2. Get suggestions: braim statement verify-suggest <ID>\n  3. Re-create with typed sources: braim statement add \"...\" --sources \"code:verified.rs,doc:spec.md\" ...\n  4. Verification status auto-calculates to PROVEN\n\nNote: Helps agents find evidence without manual investigation.")]
    VerifySuggest {
        id: u32,
    },
    #[command(about = "Update dependency weights for a statement", long_about = "Statement UpdateWeights: Adjust how a statement combines its concept contributions.\n\nUsage:\n  braim statement update-weights 42 --weights \"1:0.4,2:0.6\"\n\nRules:\n  • Weights must sum to exactly 1.0\n  • IDs must be concepts the statement depends on\n  • Cannot change which concepts are referenced\n\nExample: \"Invoice before Payment\" (ID 42) originally weights Invoice=0.5, Payment=0.5.\n  Update to Invoice=0.7, Payment=0.3 (Invoice is more important).")]
    UpdateWeights {
        id: u32,
        #[arg(long, help = "New weights: \"ID:weight,ID:weight\" (must sum to 1.0)")]
        weights: String,
    },
    #[command(about = "Add, remove, or replace dependency edges on a statement", long_about = "Statement UpdateDeps: Modify which concepts a statement depends on without deleting it.\n\nPreserves the statement ID, its attached source entities, and everything referencing it —\nthe alternative (delete + recreate) loses all three.\n\nUsage:\n  braim statement update-deps 42 --add \"5:0.3\" --remove \"3\"\n  braim statement update-deps 42 --set \"1:0.6,5:0.4\"\n\nRules:\n  • --set is exclusive: --add and --remove are ignored when --set is provided\n  • Weights must sum to 1.0 after the operation\n  • Invalid statements cannot be rewired; invalid dependencies are rejected\n  • Verification is recomputed afterward (dependency inheritance may cap it)\n  • Gap-register entries covered by the new dependency pairs auto-clear")]
    UpdateDeps {
        id: u32,
        #[arg(long, help = "Add dependency edges: \"ID:weight[,ID:weight]\"")]
        add: Option<String>,
        #[arg(long, help = "Remove dependency edges by ID: \"ID[,ID]\"")]
        remove: Option<String>,
        #[arg(long, help = "Replace all dependencies: \"ID:weight[,ID:weight]\" (exclusive with --add/--remove)")]
        set: Option<String>,
    },
    #[command(about = "Attach a source entity to an existing statement", long_about = "Statement AddSource: Link a first-class source entity to a statement after creation.\n\nUsage:\n  braim statement add-source 42 --source-id 5001\n\nEffect on non-contested statements:\n  • Appends source entity to the statement's source_ids list\n  • Recomputes verification_status from all string sources + source entities\n  • A new PRIMARY-typed source can raise the verification level\n\nEffect on contested statements (Mechanism A auto-resolution):\n  • If the source is PRIMARY-typed AND the other contested statement does NOT\n    have this source, auto-resolution fires:\n      Winner (this statement) → status recomputed (likely partial/proven/proven_strong)\n      Loser  (other statement) → invalid; cascades to its dependents\n      Contradicts edge → marked resolved\n  • If the new source is on both sides (corroborates both), no auto-resolution;\n    use 'statement resolve-contradiction' instead.\n\nWorkflow:\n  braim source add \"Audit log entry\" --type transcript --location \"transcript:audit.txt:88\"\n  # → ID:5001\n  braim statement add-source 42 --source-id 5001\n  # If 42 is contested and the other side lacks source 5001 → auto-resolved")]
    AddSource {
        id: u32,
        #[arg(long, help = "Source entity ID to attach (from 'braim source add')")]
        source_id: u32,
    },
    #[command(about = "Mark two statements as contradicting each other", long_about = "Statement Contradict: Record that two statements make incompatible claims about the same subject.\n\nUsage:\n  braim statement contradict 42 99 \\\n    --reason \"Statement 42 says 24h, statement 99 says 48h per spec_v2\"\n\n  braim statement contradict 42 99 \\\n    --reason \"Contradicted by spec_v2\" --source 5001\n\nEffect:\n  • Both statements move to 'contested' verification_status\n  • Both are hidden from default queries (use --include-contested to surface them)\n  • Neither can be auto-promoted while contested — new sources do not raise them\n  • Dependents of contested statements inherit the contested state\n\nResolution:\n  Explicit:  braim statement resolve-contradiction <winner> <loser> --winner <id> --reason \"...\"\n  → Winner: status restored to pre-contested level (or recomputed from sources)\n  → Loser:  becomes invalid, cascades to its dependents\n\nQuery contested statements:\n  braim query \"term\" --include-contested\n\nSee CONTRADICTION RESOLUTION section in 'braim --help' for full workflow.")]
    Contradict {
        stmt_a: u32,
        stmt_b: u32,
        #[arg(long, help = "Reason for the contradiction")]
        reason: String,
        #[arg(long, help = "Source ID that revealed the conflict (optional)")]
        source: Option<u32>,
    },
    #[command(about = "Resolve a contradiction between two statements", long_about = "Statement ResolveContradiction: Declare a winner and loser for an active contradiction.\n\nUsage:\n  braim statement resolve-contradiction 42 99 \\\n    --winner 42 --reason \"spec_v1 is authoritative; spec_v2 was a draft\"\n\n  braim statement resolve-contradiction 42 99 \\\n    --winner 99 --reason \"Confirmed by code review\" --source 5002\n\nArguments:\n  stmt_a, stmt_b:  The two statement IDs involved in the contradiction\n  --winner:        ID of the statement that is correct\n  --reason:        Explanation for why this side wins\n  --source:        Optional source entity ID that corroborates the winner\n\nEffect:\n  Winner:\n    • verification_status restored to pre-contested level (or recomputed from sources)\n    • node_type updated accordingly (claim / fact)\n  Loser:\n    • verification_status → invalid\n    • node_type → invalid_statement\n    • Cascade-invalidates all transitive dependents of the loser\n  Contradicts edge:\n    • Marked resolved=true with resolution_winner and resolution_source recorded\n\nPre-conditions:\n  • An unresolved 'contradicts' edge must exist between stmt_a and stmt_b\n  • Neither statement can already be invalid\n  • --winner must be one of the two statement IDs provided")]
    ResolveContradiction {
        stmt_a: u32,
        stmt_b: u32,
        #[arg(long, help = "ID of the winning statement")]
        winner: u32,
        #[arg(long, help = "Reason for the resolution")]
        reason: String,
        #[arg(long, help = "Source ID that corroborates the winner (optional)")]
        source: Option<u32>,
    },
}

#[derive(Subcommand)]
enum DreamCommands {
    #[command(about = "List node pairs worth an LLM's judgement", long_about = "Dream Candidates: rank unconnected node pairs that an LLM should examine for a missing relation.\n\nUsage:\n  braim dream candidates --limit 50\n  braim dream candidates --strategy shared-source,two-hop --json\n  braim dream candidates --strategy semantic --min-semantic 0.8\n\nWhy braim picks the pairs: a 3,000-node graph has ~5 million pairs, so unguided\nsampling burns an overnight budget on noise. braim does the cheap deterministic\nhalf (which pairs are worth reading); the LLM does the expensive half (whether a\nrelation is real and whether sources prove it).\n\nStrategies:\n  shared-source  both nodes cite the same PRIMARY source but were never linked\n  two-hop        A-B and B-C exist, A-C does not (transitive candidate)\n  semantic       labels semantically close yet more than two hops apart\n  gap            a registered zero-path pair a real query already wanted\n\nA pair nominated by several strategies scores higher — independent structural\nsignals agreeing is itself evidence.\n\nExcluded by default: directly linked pairs, invalid nodes, source entities,\nagent-scratch markers (--include-scratch overrides), and pairs already recorded\nin the dream ledger (--replay overrides).\n\nThis command is READ-ONLY. Dream output must land as unproven claims in a local\nworking graph and earn promotion through genuinely re-grounded PRIMARY sources,\nlike any other statement — an LLM asked whether two nodes relate will nearly\nalways say yes.\n\nRefused on graphs marked central (.braim.central): a dream is an unreviewed\nhypothesis, and an unattended central has no reviewer.")]
    Candidates {
        #[arg(long, default_value = "25", help = "Maximum pairs to emit")]
        limit: usize,
        #[arg(long, help = "Comma-separated: shared-source, two-hop, semantic, gap (default: all but semantic)")]
        strategy: Option<String>,
        #[arg(long, help = "Minimum cosine for the semantic strategy")]
        min_semantic: Option<f32>,
        #[arg(long, help = "Also consider nodes tagged scope=agent_scratch")]
        include_scratch: bool,
        #[arg(long, help = "Reconsider pairs already recorded in the dream ledger")]
        replay: bool,
        #[arg(long, help = "Emit JSON for an agent loop to consume")]
        json: bool,
    },
    #[command(about = "Rank load-bearing causes by how much rests on them", long_about = "Dream Constraints: rank causes by leverage — how many statements would need re-examining if this one stopped being true.\n\nUsage:\n  braim dream constraints --limit 10\n  braim dream constraints --json\n\nbraim cannot tell a constraint from any other cause — that is a judgement about\nmeaning. What it computes is the blast radius: the transitive set of statements\nreaching a cause through because_of, scaled by how well evidenced that cause is.\nAn unproven cause is discounted (relaxing an opinion is meaningless) but still\nlisted, since a high-impact assumption may be the one worth testing.\n\nThe LLM then decides which of the top entries are actually constraints and\nwhether they can be relaxed. Same split as pair-dreaming: braim picks the\ntarget deterministically, the model supplies judgement (braim ID:323).\n\nreads_as_limitation is an ADVISORY annotation only. Limitation vocabulary\nmatched 61 of 161 statements on a real graph, mostly false positives, so it is\nreported to the reader and contributes nothing to the ranking.\n\nRead-only, and refused on graphs marked .braim.central like the rest of dream.")]
    Constraints {
        #[arg(long, default_value = "15", help = "Maximum causes to emit")]
        limit: usize,
        #[arg(long, help = "Also consider nodes tagged scope=agent_scratch")]
        include_scratch: bool,
        #[arg(long, help = "Include constraints already walked, even with nothing new since")]
        include_walked: bool,
        #[arg(long, help = "Emit JSON for an agent loop to consume")]
        json: bool,
    },
    #[command(about = "Walk one constraint: what rests on it, what it serves, and whether it is stale", long_about = "Dream What-If: relax one constraint and see what moves.\n\nUsage:\n  braim dream whatif 186\n  braim dream whatif 186 --json\n\nPick a target with `braim dream constraints`, then walk it here. The walk goes\nboth ways: DOWN through because_of to everything resting on the constraint (what\ncomes into play if it is lifted), and UP to the root goal the constraint\nultimately serves (what the relaxation is FOR).\n\nStaleness signals come first for a reason. Whether relaxing a constraint would\nimprove anything is unprovable, but whether the constraint STILL HOLDS is an\nordinary question about current sources — and that is the half of what-if\ndreaming that yields real findings (braim ID:324). A signal is a statement\nciting the same PRIMARY source file, written later, evidenced at least as well,\nwith no contradiction linking the two yet. braim reports them and stops:\nraising a contradiction is a deliberate claim needing a reason and a source,\nnot something to infer from a shared file path.\n\nWhatever the LLM writes from the relaxation itself is a hypothesis. Tag it\n`braim meta <id> --set counterfactual=true` — export refuses those by design,\nbecause no source can prove that removing a constraint would improve an outcome\n(braim ID:322).\n\nRead-only, and refused on graphs marked .braim.central like the rest of dream.")]
    Whatif {
        #[arg(help = "Statement id to relax — pick one from `braim dream constraints`")]
        id: u32,
        #[arg(long, default_value = "5", help = "Maximum staleness signals to report")]
        signals: usize,
        #[arg(long, help = "Also consider nodes tagged scope=agent_scratch")]
        include_scratch: bool,
        #[arg(long, help = "Emit JSON for an agent loop to consume")]
        json: bool,
    },
    #[command(about = "Raise something for a human to look at", long_about = "Dream Flag: put an observation in the review queue, where it survives the session.\n\nUsage:\n  braim dream flag \"merge 412 warned about deps only the loser had\" --kind merge --nodes 412,88\n  braim dream flag \"looked like a contradiction but I could not ground it\" --kind unraised\n\nA night's most review-worthy output is often not a node: a merge warning, detail\ndestroyed with a loser's label, a contradiction the adjudicator could not ground.\nThose used to go in the closing report, which lives in the model's context and\ndoes not survive compaction — so the part of the night that most needed eyes was\nthe part that evaporated (braim ID:347).\n\nThe queue lives in reviews.json beside the graph. Read it with `braim dream\nreview`, sign an item off with `braim dream reviewed <id>`.")]
    Flag {
        #[arg(help = "What a human should look at")]
        note: String,
        #[arg(long, default_value = "note", help = "merge | unraised | duplicate | rate | note")]
        kind: String,
        #[arg(long, help = "Node ids this concerns, comma-separated")]
        nodes: Option<String>,
    },
    #[command(about = "List what a night left for a human", long_about = "Dream Review: the queue of items raised by `braim dream flag`, pending first.\n\nUsage:\n  braim dream review\n  braim dream review --all      # include items already signed off\n  braim dream review --json\n\nThis is what to read after an unattended night. It survives context compaction\nbecause it is a file beside the graph, not prose in a report.\n\nSee also: `braim list --meta scope=dream` for the nodes a session created, and\n`braim dream log` for the pair verdicts.")]
    Review {
        #[arg(long, help = "Include items already signed off")]
        all: bool,
        #[arg(long, help = "Emit JSON")]
        json: bool,
    },
    #[command(about = "Sign a review item off", long_about = "Dream Reviewed: mark a queue item as handled.\n\nUsage:\n  braim dream reviewed 3\n  braim dream reviewed 3 --note \"wired the dependency by hand\"\n\nCleared items are kept, not deleted: what a human decided is itself worth\nkeeping, and a queue that forgets its own history cannot be audited. See them\nwith `braim dream review --all`.")]
    Reviewed {
        #[arg(help = "Review item id from `braim dream review`")]
        id: u32,
        #[arg(long, help = "What you did about it")]
        note: Option<String>,
    },
    #[command(about = "Read back the pair verdicts a night recorded", long_about = "Dream Log: the adjudication ledger (dreams.json), newest first.\n\nUsage:\n  braim dream log --limit 20\n  braim dream log --since 2026-08-10\n  braim dream log --verdict verified\n  braim dream log --json\n\nThe ledger records every pair a session judged, with the note the adjudicator\nwrote. It was previously write-only from the CLI — `dream seen` put entries in\nand only the candidate generator read them back, so the reasoning behind 1939\nverdicts was reachable only with jq (braim ID:347).")]
    Log {
        #[arg(long, help = "Only entries recorded on or after this date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "no-relation | proposed | verified | contradiction | duplicate")]
        verdict: Option<String>,
        #[arg(long, default_value = "25", help = "Maximum entries to show")]
        limit: usize,
        #[arg(long, help = "Emit JSON")]
        json: bool,
    },
    #[command(about = "Record a dream verdict so the pair is not re-examined", long_about = "Dream Seen: write a pair's adjudication into the dream ledger (dreams.json).\n\nUsage:\n  braim dream seen 42 99 --verdict no-relation\n  braim dream seen 42 99 --verdict proposed --note \"statement ID:150 added\"\n\nVerdicts:\n  no-relation    examined, nothing there — never offer this pair again\n  proposed       a relation was recorded as an unproven claim for review\n  verified       a relation was recorded WITH re-grounded PRIMARY sources\n  contradiction  the two nodes actually disagree; a contradicts edge was raised\n\nThe ledger is what lets successive nights advance instead of re-treading the\nsame pairs.")]
    Seen {
        a: u32,
        b: u32,
        #[arg(long, help = "no-relation | proposed | verified | contradiction")]
        verdict: String,
        #[arg(long, help = "Optional note (e.g. the statement id that was created)")]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
enum SourceCommands {
    #[command(about = "Add a first-class source entity", long_about = "Source Add: Create a named source entity with a type, location, and ingestion timestamp.\n\nSources created this way have a stable ID that statements can reference.\nThe same source referenced by multiple statements is counted once for PRIMARY-type diversity.\n\nUsage:\n  braim source add \"Refund design doc section 3.2\" \\\n    --type doc --location \"doc:billing_design.md:3.2\"\n\n  braim source add \"Billing code review\" \\\n    --type code --location \"code:src/billing.rs:42-98\" \\\n    --ingested-by \"agent:context_phase\"\n\nArguments:\n  label:          Human-readable identifier for the source\n  --type:         Source type prefix (code, doc, schema, config, transcript, test,\n                  phase_N, agent, narrative, logic, inference)\n  --location:     Optional file path, URL, or document reference\n  --ingested-by:  Optional agent name or user ID who ingested this source\n  --strict-sources: Reject if label contains a line-number suffix (default: warn)\n\nLine-number warning:\n  Labels like 'tests/oracle.txt:104-127' or 'file.rs:42' are warned by default\n  and rejected with --strict-sources. Source nodes are stable file-level identity;\n  line numbers belong in --sources metadata strings, not node labels.\n\nOutput:\n  Returns the source ID (e.g., ID:5001) for use with 'statement add --source-ids'.\n\nSource types and verification tiers:\n  PRIMARY (independent evidence):    code, doc, schema, config, transcript, test\n  SECONDARY (derived or contextual): phase_N, agent, narrative\n  TERTIARY (logical derivation):     logic, inference\n\nVerification impact:\n  PRIMARY-typed source entities raise statement verification when referenced.\n  Distinct PRIMARY types from different source entities determine the level:\n    1 PRIMARY type → partial\n    2 PRIMARY types → proven\n    3+ PRIMARY types → proven_strong")]
    Add {
        label: String,
        #[arg(long, help = "Source type: code, doc, schema, config, transcript, test, phase_N, agent, narrative, logic, inference")]
        r#type: String,
        #[arg(long, help = "Location (file path, URL, doc reference)")]
        location: Option<String>,
        #[arg(long, help = "Agent or user who ingested this source")]
        ingested_by: Option<String>,
        #[arg(long, help = "Reject if label contains a line-number suffix (default: warn)")]
        strict_sources: bool,
    },
}

#[derive(Subcommand)]
enum VersionCommands {
    #[command(about = "Save a checkpoint of the current graph state", long_about = "Version Save: Create a timestamped backup of the graph.\n\nUsage:\n  braim version save\n  braim version save \"Added payment domain concepts\"\n  braim version save \"Merged branch X changes\"\n\nBest Practices:\n  • Save after each batch of concept/statement additions\n  • Save with descriptive message before major changes\n  • Saves are automatic checkpoints (no manual backups needed)\n\nStored in: .braim/versions/v*.json with timestamp")]
    Save {
        #[arg(default_value = "", help = "Optional description of changes")]
        description: String,
    },
    #[command(about = "List all saved versions with timestamps and descriptions")]
    List,
    #[command(about = "Restore graph to a previous version", long_about = "Version Restore: Revert to a checkpoint.\n\nUsage:\n  braim version list    # Find version numbers\n  braim version restore 2  # Restore to version 2\n\nWarning: This overwrites current.json. The current state is NOT auto-saved.\nAlways 'version save' first if you want to keep current work.")]
    Restore {
        n: u32,
    },
}

fn parse_depends(s: &str) -> Result<HashMap<u32, f64>, String> {
    let mut result = HashMap::new();
    for part in s.split(',') {
        let part = part.trim();
        let mut iter = part.splitn(2, ':');
        let id_str = iter
            .next()
            .ok_or_else(|| "Error: Invalid --depends format. Expected \"ID:weight,ID:weight\"".to_string())?;
        let weight_str = iter
            .next()
            .ok_or_else(|| "Error: Invalid --depends format. Expected \"ID:weight,ID:weight\"".to_string())?;

        let id: u32 = id_str
            .parse()
            .map_err(|_| "Error: Invalid --depends format. Expected \"ID:weight,ID:weight\"".to_string())?;
        let weight: f64 = weight_str
            .parse()
            .map_err(|_| "Error: Invalid --depends format. Expected \"ID:weight,ID:weight\"".to_string())?;

        result.insert(id, weight);
    }
    Ok(result)
}

fn parse_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|item| item.trim().to_string())
        .collect()
}

fn get_node_symbol(node_type: &NodeType) -> &'static str {
    match node_type {
        NodeType::Atomic => "●",
        NodeType::Compound => "◉",
        NodeType::Statement | NodeType::Fact => "▶",
        NodeType::Claim => "?",
        NodeType::InvalidStatement => "✗",
        NodeType::ContestedStatement => "⚠",
        NodeType::Source => "◈",
    }
}

/// Decide whether a node passes the statement-family filter per
/// BRAIM_NODE_TYPE_CLAIM_FACT_SPEC §3.5.
///
/// Concept nodes (Atomic, Compound) always pass. Legacy `Statement` is treated
/// as Fact since the in-memory migration in `Braim::new` will have replaced any
/// `Statement` with the derived type — this branch is defensive only.
fn statement_family_visible(
    node_type: &NodeType,
    only_claims: bool,
    include_claims: bool,
    include_invalid: bool,
    include_contested: bool,
) -> bool {
    match node_type {
        NodeType::Atomic | NodeType::Compound => true,
        NodeType::Fact | NodeType::Statement => !only_claims,
        NodeType::Claim => only_claims || include_claims,
        NodeType::InvalidStatement => include_invalid,
        NodeType::ContestedStatement => include_contested,
        NodeType::Source => false,
    }
}

/// True for commands that only read this data dir. Everything else takes the
/// cross-process write lock before loading, so its read-modify-write cycle
/// cannot interleave with another process's (braim ID:250).
///
/// Two entries deserve their reasoning:
///   • `Proximity`/`Perspective` are deliberately ABSENT — they look like
///     queries but register gap records and flush, so they are writers.
///   • `Export` is present because it only READS this dir; the target central
///     graph is opened separately with its own lock.
fn is_read_only(cmd: &Commands) -> bool {
    matches!(
        cmd,
        Commands::Lookup { .. }
            | Commands::Query { .. }
            | Commands::Node { .. }
            | Commands::List { .. }
            | Commands::Domains
            | Commands::Audit { .. }
            | Commands::Serve { .. }
            | Commands::Similar { .. }
            | Commands::Why { .. }
            | Commands::Export { .. }
            | Commands::Dream(DreamCommands::Candidates { .. })
            | Commands::Dream(DreamCommands::Constraints { .. })
            | Commands::Dream(DreamCommands::Whatif { .. })
            | Commands::Dream(DreamCommands::Review { .. })
            | Commands::Dream(DreamCommands::Log { .. })
            | Commands::Policy { .. }
            | Commands::Init { .. }
            | Commands::Statement(StatementCommands::VerifySuggest { .. })
            | Commands::Version(VersionCommands::List)
    )
}

fn main() {
    let cli = Cli::parse();

    let open = if is_read_only(&cli.command) {
        Braim::new(&cli.data_dir)
    } else {
        Braim::open_for_write(&cli.data_dir)
    };
    let braim = match open {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // `run` OWNS the graph, so the cross-process write lock is released by Drop
    // when it returns — on the error paths too. Exiting the process from inside
    // a command handler skips that Drop and strands .braim.lock for the full
    // stale window, which one missing --domains used to do (braim ID:326).
    let result = run(cli, braim);

    if let Err(e) = result {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

/// Dispatch one command. Fails by returning `Err`, never by exiting: an early
/// `std::process::exit` here would skip `Braim`'s Drop and leak the write lock
/// (braim ID:326).
fn run(cli: Cli, mut braim: Braim) -> Result<(), String> {
    match cli.command {
        Commands::Concept(ConceptCommands::Add {
            term,
            domains,
            sources,
            depends,
            strict_sources,
            check_dupes,
        }) => {
            if check_dupes {
                dedup_warn(&braim, &cli.data_dir, &term, cli.quiet);
            }
            let domains_list = parse_list(&domains);
            let sources_list = parse_list(&sources);

            // Validate duplicate sources
            let (has_dup_sources, dup_sources) = Braim::validate_duplicate_sources(&sources_list);
            if has_dup_sources {
                if strict_sources {
                    return Err("Error: duplicate source entries detected".to_string());
                } else {
                    tips::emit_tip_duplicate_sources(&dup_sources, cli.quiet);
                }
            }

            // Validate PRIMARY+TERTIARY mix
            if Braim::validate_primary_tertiary_mix(&sources_list) {
                if strict_sources {
                    return Err("Error: PRIMARY and TERTIARY sources mixed on same statement".to_string());
                } else {
                    tips::emit_tip_primary_tertiary_mix(cli.quiet);
                }
            }

            let depends_map = match depends {
                Some(d) => Some(match parse_depends(&d) {
                    Ok(m) => m,
                    Err(e) => {
                        return Err(format!("{}", e));
                    }
                }),
                None => None,
            };

            match braim.add_concept(&term, domains_list.clone(), sources_list.clone(), depends_map) {
                Ok(id) => {
                    let node = &braim.state.nodes[&id];
                    let node_type_str = match node.node_type {
                        NodeType::Atomic => "atomic",
                        NodeType::Compound => "compound",
                        NodeType::Statement => "statement",
                        NodeType::Claim => "claim",
                        NodeType::Fact => "fact",
                        NodeType::InvalidStatement => "invalid_statement",
                        NodeType::ContestedStatement => "contested_statement",
                        NodeType::Source => "source",
                    };
                    println!("✓ {} concept added", node_type_str);
                    println!("  ID:{}  domains: {:?}  sources: {:?}  {}", id, domains_list, sources_list, node.label);
                    if !node.depends_on.is_empty() {
                        print!("  depends_on: {{");
                        let mut first = true;
                        for (dep_id, weight) in &node.depends_on {
                            if !first {
                                print!(", ");
                            }
                            print!("{}: {}", dep_id, weight);
                            first = false;
                        }
                        println!("}}");
                    }
                    tips::emit_tip_concept_add(node, cli.quiet);

                    // Check for decomposable atomics (Issue 5)
                    if node.node_type == NodeType::Atomic {
                        let decomposable = braim.find_decomposable_atomics(&node.label);
                        if decomposable.len() >= 2 {
                            let dep_spec = decomposable
                                .iter()
                                .map(|(id, _)| {
                                    let weight = 1.0 / decomposable.len() as f64;
                                    format!("{}:{:.1}", id, weight)
                                })
                                .collect::<Vec<_>>()
                                .join(",");
                            tips::emit_tip_decomposable_compound(&node.label, &decomposable, &dep_spec, cli.quiet);
                        }
                    }

                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            }
        }
        Commands::Concept(ConceptCommands::Delete { id, force }) => {
            if !braim.state.nodes.contains_key(&id) {
                return Err(format!("Error: Concept ID {} not found", id));
            }

            // Find dependents
            let mut dependents = Vec::new();
            for (other_id, other_node) in &braim.state.nodes {
                if other_node.depends_on.contains_key(&id) {
                    dependents.push((*other_id, other_node.label.clone()));
                }
            }

            if !dependents.is_empty() && !force {
                eprintln!("⚠ Concept ID {} is referenced by {} node(s):", id, dependents.len());
                for (dep_id, dep_label) in &dependents {
                    eprintln!("  - ID:{}  {}", dep_id, dep_label);
                }
                eprintln!("");
                return Err("Delete anyway? Use --force to confirm.".to_string());
            }

            match braim.delete_node(id) {
                Ok(_) => {
                    println!("✓ Concept ID:{} deleted", id);
                    if !dependents.is_empty() {
                        println!("  ⚠ {} dependent node(s) now have broken references", dependents.len());
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Concept(ConceptCommands::UpdateWeights { id, weights }) => {
            let new_weights = match parse_depends(&weights) {
                Ok(w) => w,
                Err(e) => {
                    return Err(format!("{}", e));
                }
            };

            if !braim.state.nodes.contains_key(&id) {
                return Err(format!("Error: Concept ID {} not found", id));
            }

            match braim.update_weights(id, new_weights.clone()) {
                Ok(_) => {
                    println!("✓ Concept ID:{} weights updated", id);
                    println!("  depends_on: {:?}", new_weights);
                    Ok(())
                }
                Err(e) => {
                    return Err(format!("{}", e));
                }
            }
        }
        Commands::Concept(ConceptCommands::UpdateDeps { id, add, remove, set }) => {
            let add_map = match add.as_deref().map(parse_depends) {
                Some(Ok(m)) => Some(m),
                Some(Err(e)) => { return Err(format!("{}", e)); }
                None => None,
            };
            let remove_ids: Option<Vec<u32>> = match remove.as_deref() {
                Some(s) => {
                    let parsed: Result<Vec<u32>, _> = s.split(',').map(|x| x.trim().parse::<u32>()).collect();
                    match parsed {
                        Ok(v) => Some(v),
                        Err(_) => { return Err("Error: --remove must be comma-separated IDs (e.g. \"3,7\")".to_string()); }
                    }
                }
                None => None,
            };
            let set_map = match set.as_deref().map(parse_depends) {
                Some(Ok(m)) => Some(m),
                Some(Err(e)) => { return Err(format!("{}", e)); }
                None => None,
            };
            if add_map.is_none() && remove_ids.is_none() && set_map.is_none() {
                return Err("Error: provide at least one of --add, --remove, or --set".to_string());
            }
            match braim.update_deps(id, add_map, remove_ids, set_map) {
                Ok(new_deps) => {
                    println!("✓ Concept ID:{} dependencies updated", id);
                    println!("  depends_on: {:?}", new_deps);
                    Ok(())
                }
                Err(e) => {
                    return Err(format!("{}", e));
                }
            }
        }
        Commands::Statement(StatementCommands::Add {
            text,
            domains,
            sources,
            depends,
            inferred,
            assume,
            strict_sources,
            strict_domains,
            check_dupes,
        }) => {
            if check_dupes {
                dedup_warn(&braim, &cli.data_dir, &text, cli.quiet);
            }
            // Validation: inferred flag is mutually exclusive with explicit sources
            if inferred && sources.is_some() {
                return Err("Error: --inferred and --sources are mutually exclusive. Use --inferred for derived statements.".to_string());
            }

            // Validation: reject manual "inferred" as a source value
            if !inferred && sources.is_some() {
                let sources_str = sources.as_ref().unwrap();
                if sources_str.contains("inferred") {
                    return Err("Error: 'inferred' is a reserved source name. Use --inferred flag for derived statements.".to_string());
                }
            }

            let depends_map = match parse_depends(&depends) {
                Ok(m) => m,
                Err(e) => {
                    return Err(format!("{}", e));
                }
            };

            // Set domains and sources based on inferred flag
            let (domains_list, sources_list) = if inferred {
                // For inferred statements, default domains to "computation" repeated for each dependency
                let dep_count = depends_map.len();
                let doms = domains.as_ref()
                    .map(|d| parse_list(d))
                    .unwrap_or_else(|| vec!["computation".to_string(); dep_count]);
                let sources_vec = vec!["inferred".to_string(); dep_count];
                (doms, sources_vec)
            } else {
                if domains.is_none() || sources.is_none() {
                    return Err("Error: --domains and --sources are required for explicit statements. Use --inferred for derived statements.".to_string());
                }
                (parse_list(domains.as_ref().unwrap()), parse_list(sources.as_ref().unwrap()))
            };

            // Validate duplicate sources (Issue 1)
            if !inferred {
                let (has_dup_sources, dup_sources) = Braim::validate_duplicate_sources(&sources_list);
                if has_dup_sources {
                    if strict_sources {
                        return Err("Error: duplicate source entries detected".to_string());
                    } else {
                        tips::emit_tip_duplicate_sources(&dup_sources, cli.quiet);
                    }
                }

                // Validate PRIMARY+TERTIARY mix (Issue 2)
                if Braim::validate_primary_tertiary_mix(&sources_list) {
                    if strict_sources {
                        return Err("Error: PRIMARY and TERTIARY sources mixed on same statement".to_string());
                    } else {
                        tips::emit_tip_primary_tertiary_mix(cli.quiet);
                    }
                }
            }

            // Validate duplicate domains (Issue 3)
            let (has_dup_domains, dup_domain_counts) = Braim::validate_duplicate_domains(&domains_list);
            if has_dup_domains {
                if strict_domains {
                    return Err("Error: duplicate domain entries detected".to_string());
                } else if braim.distinct_domain_count() > 1 {
                    // Suppress in single-domain graphs — uniform repetition is expected
                    tips::emit_tip_duplicate_domains(&dup_domain_counts, cli.quiet);
                }
            }

            match braim.add_statement(&text, domains_list.clone(), sources_list.clone(), depends_map, assume) {
                Ok(id) => {
                    let node = &braim.state.nodes[&id];
                    let stmt_type = if inferred { "inferred statement" } else { "statement" };
                    println!("✓ {} added", stmt_type);
                    println!("  ID:{}  domains: {:?}  sources: {:?}  {}", id, domains_list, sources_list, text);
                    print!("  depends_on: {{");
                    let mut first = true;
                    for (dep_id, weight) in &node.depends_on {
                        if !first {
                            print!(", ");
                        }
                        print!("{}: {}", dep_id, weight);
                        first = false;
                    }
                    println!("}}");
                    tips::emit_tip_statement_add(node, &braim, cli.quiet);
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            }
        }
        Commands::Statement(StatementCommands::Verify { statement_id, domain, note }) => {
            match braim.verify_statement(statement_id, &domain, note) {
                Ok(()) => {
                    let node = &braim.state.nodes[&statement_id];
                    let status_str = match node.verification_status {
                        graph::VerificationStatus::Invalid => "invalid",
                        graph::VerificationStatus::Unproven => "unproven",
                        graph::VerificationStatus::Contested => "contested",
                        graph::VerificationStatus::Partial => "partial",
                        graph::VerificationStatus::Proven => "proven",
                        graph::VerificationStatus::ProvenStrong => "proven_strong",
                    };
                    println!("✓ Statement ID:{} verified by domain '{}'", statement_id, domain);
                    println!("  Verification status: {}  ({} domains)", status_str, node.verified_by.len());
                    for (d, note_opt) in &node.verified_by {
                        let note_str = note_opt.as_ref().map(|n| format!(" — {}", n)).unwrap_or_default();
                        println!("    ✓ {}{}", d, note_str);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::Delete { id, force }) => {
            if !braim.state.nodes.contains_key(&id) {
                return Err(format!("Error: Statement ID {} not found", id));
            }

            // Find dependents
            let mut dependents = Vec::new();
            for (other_id, other_node) in &braim.state.nodes {
                if other_node.depends_on.contains_key(&id) {
                    dependents.push((*other_id, other_node.label.clone()));
                }
            }

            if !dependents.is_empty() && !force {
                eprintln!("⚠ Statement ID {} is referenced by {} node(s):", id, dependents.len());
                for (dep_id, dep_label) in &dependents {
                    eprintln!("  - ID:{}  {}", dep_id, dep_label);
                }
                eprintln!("");
                return Err("Delete anyway? Use --force to confirm.".to_string());
            }

            match braim.delete_node(id) {
                Ok(_) => {
                    println!("✓ Statement ID:{} deleted", id);
                    if !dependents.is_empty() {
                        println!("  ⚠ {} dependent node(s) now have broken references", dependents.len());
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::Invalidate { id, reason }) => {
            let cascade_preview = braim.find_cascade_nodes(id);
            if !cascade_preview.is_empty() {
                eprintln!("⚠ Invalidating statement ID:{} will cascade to {} dependent statement(s):", id, cascade_preview.len());
                for (dep_id, label) in &cascade_preview {
                    eprintln!("  - ID:{}  {}", dep_id, label);
                }
            }

            match braim.invalidate_statement(id, &reason) {
                Ok(cascaded_ids) => {
                    let node = &braim.state.nodes[&id];
                    println!("✗ Statement ID:{} marked INVALID", id);
                    println!("  Reason: {}", reason);
                    println!("  Invalidated at: {}", node.invalidated_at.as_ref().unwrap_or(&"unknown".to_string()));
                    println!("  Original: {}", node.label);
                    if !cascaded_ids.is_empty() {
                        println!("  Cascade: {} dependent statement(s) marked invalid: {:?}",
                            cascaded_ids.len(), cascaded_ids);
                    }
                    tips::emit_tip_invalidate(&cascaded_ids, cli.quiet);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::Revalidate { id }) => {
            match braim.revalidate_statement(id) {
                Ok((status, invalid_deps)) => {
                    let node = &braim.state.nodes[&id];
                    println!("✓ Statement ID:{} revalidated", id);
                    println!("  Status: {} {}", status.badge(), status.label());
                    println!("  Original: {}", node.label);
                    if !invalid_deps.is_empty() {
                        eprintln!("⚠ still depends on invalid node(s): {:?}", invalid_deps);
                        eprintln!("  these were skipped in the inheritance cap — re-anchor with 'braim statement update-deps {} --set ...' to make the revival durable", id);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::VerifySuggest { id }) => {
            match braim.verify_suggest(id) {
                Ok(report) => {
                    println!("Verification suggestions for Statement ID:{}", report.statement_id);
                    println!("Label: {}", report.label);
                    println!(
                        "Current status: {} ({} PRIMARY sources, {} distinct types)\n",
                        report.status_label, report.primary_count, report.distinct_primary_types
                    );

                    if let Some(msg) = &report.message {
                        println!("{}\n", msg);
                    } else {
                        println!("Suggested verification sources (ranked by predicted promotion impact):");
                        for (i, c) in report.candidates.iter().enumerate() {
                            println!("  {}. {}", i + 1, c.source);
                            println!("     → {}", c.rationale);
                            println!("     → Promotion impact: {}", c.impact);
                        }
                        println!();
                    }
                    // Always surface the missing-types summary — useful even when no
                    // concrete candidates exist (target still knows what would help).
                    println!("Already-attached source types: {:?}", report.already_attached_types);
                    println!("Missing source types that would promote: {:?}", report.missing_primary_types);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::UpdateWeights { id, weights }) => {
            let new_weights = match parse_depends(&weights) {
                Ok(w) => w,
                Err(e) => {
                    return Err(format!("{}", e));
                }
            };

            if !braim.state.nodes.contains_key(&id) {
                return Err(format!("Error: Statement ID {} not found", id));
            }

            match braim.update_weights(id, new_weights.clone()) {
                Ok(_) => {
                    println!("✓ Statement ID:{} weights updated", id);
                    println!("  depends_on: {:?}", new_weights);
                    Ok(())
                }
                Err(e) => {
                    return Err(format!("{}", e));
                }
            }
        }
        Commands::Statement(StatementCommands::UpdateDeps { id, add, remove, set }) => {
            let add_map = match add.as_deref().map(parse_depends) {
                Some(Ok(m)) => Some(m),
                Some(Err(e)) => { return Err(format!("{}", e)); }
                None => None,
            };
            let remove_ids: Option<Vec<u32>> = match remove.as_deref() {
                Some(s) => {
                    let parsed: Result<Vec<u32>, _> = s.split(',').map(|x| x.trim().parse::<u32>()).collect();
                    match parsed {
                        Ok(v) => Some(v),
                        Err(_) => { return Err("Error: --remove must be comma-separated IDs (e.g. \"3,7\")".to_string()); }
                    }
                }
                None => None,
            };
            let set_map = match set.as_deref().map(parse_depends) {
                Some(Ok(m)) => Some(m),
                Some(Err(e)) => { return Err(format!("{}", e)); }
                None => None,
            };
            if add_map.is_none() && remove_ids.is_none() && set_map.is_none() {
                return Err("Error: provide at least one of --add, --remove, or --set".to_string());
            }
            match braim.update_statement_deps(id, add_map, remove_ids, set_map) {
                Ok(new_deps) => {
                    let node = &braim.state.nodes[&id];
                    println!("✓ Statement ID:{} dependencies updated", id);
                    println!("  depends_on: {:?}", new_deps);
                    println!("  Verification: {} {}", node.verification_status.badge(), node.verification_status.label());
                    Ok(())
                }
                Err(e) => {
                    return Err(format!("{}", e));
                }
            }
        }
        Commands::Lookup { term, include_claims, only_claims, include_invalid, exact, no_related } => {
            let result = if exact {
                braim.lookup_exact(&term)
            } else {
                braim.lookup(&term)
            };

            match result {
                Ok((results, is_fuzzy)) => {
                    if is_fuzzy {
                        eprintln!("⚠ Fuzzy match for '{}'", term);
                    }

                    let filtered: Vec<_> = results.into_iter().filter(|(node_id, _)| {
                        let node = &braim.state.nodes[node_id];
                        statement_family_visible(&node.node_type, only_claims, include_claims, include_invalid, false)
                    }).collect();

                    println!("Lookup: '{}'  ({} results)\n", term, filtered.len());
                    for (node_id, score) in filtered {
                        let node = &braim.state.nodes[&node_id];
                        let symbol = get_node_symbol(&node.node_type);
                        let badge = node.verification_status.badge();
                        println!(
                            "  {} {} ID:{}  domains: {:?}  {}           score={:.4}",
                            badge, symbol, node_id, node.domains, node.label, score
                        );

                        if !no_related && !node.depends_on.is_empty() {
                            println!("    Depends on:");
                            for (dep_id, weight) in &node.depends_on {
                                if let Some(dep_node) = braim.get_node(*dep_id) {
                                    println!("      ID:{}  {}  (weight: {:.4})", dep_id, dep_node.label, weight);
                                }
                            }
                        }

                        if !no_related {
                            let (_, depended_by_nodes) = braim.get_related_nodes_bounded(node_id);
                            if !depended_by_nodes.is_empty() {
                                println!("    Referenced by:");
                                for (ref_id, ref_node) in depended_by_nodes.iter().take(10) {
                                    println!("      ID:{}  {}  ({})", ref_id, ref_node.label, match ref_node.node_type {
                                        graph::NodeType::Atomic => "atomic",
                                        graph::NodeType::Compound => "compound",
                                        graph::NodeType::Statement => "statement",
                                        graph::NodeType::Claim => "claim",
                                        graph::NodeType::Fact => "fact",
                                        graph::NodeType::InvalidStatement => "invalid_statement",
                                        graph::NodeType::ContestedStatement => "contested_statement",
                                        graph::NodeType::Source => "source",
                                    });
                                }
                                if depended_by_nodes.len() > 10 {
                                    println!("      ... +{} more", depended_by_nodes.len() - 10);
                                }
                            }
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Query { terms, include_claims, only_claims, min_trust, primary_only, include_invalid, include_contested, semantic } => {
            let term_list: Vec<&str> = terms.split(',').map(|s| s.trim()).collect();
            match braim.query(&term_list) {
                Ok(results) => {
                    let mut filtered: Vec<_> = results.into_iter().filter(|(node_id, _)| {
                        let node = &braim.state.nodes[node_id];

                        // Filter by node_type (claim/fact/invalid) per spec §3.5
                        if !statement_family_visible(&node.node_type, only_claims, include_claims, include_invalid, include_contested) {
                            return false;
                        }

                        // Filter by trust level
                        if let Some(ref trust) = min_trust {
                            let status = &node.verification_status;
                            let passes = match trust.as_str() {
                                "partial" => matches!(status, graph::VerificationStatus::Partial | graph::VerificationStatus::Proven | graph::VerificationStatus::ProvenStrong),
                                "proven" => matches!(status, graph::VerificationStatus::Proven | graph::VerificationStatus::ProvenStrong),
                                _ => true,
                            };
                            if !passes {
                                return false;
                            }
                        }

                        // Filter by primary sources
                        if primary_only {
                            let has_primary = node.sources.iter().any(|s| {
                                let (source_type, _) = Braim::parse_source(s);
                                source_type.tier() == "PRIMARY"
                            });
                            if !has_primary {
                                return false;
                            }
                        }

                        true
                    }).collect();

                    filtered.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

                    let was_empty = filtered.is_empty();
                    if was_empty {
                        tips::emit_tip_query_no_results(include_claims, cli.quiet);
                    }

                    println!("Query: {:?}\n", term_list);
                    for (node_id, score) in filtered {
                        let node = &braim.state.nodes[&node_id];
                        let symbol = get_node_symbol(&node.node_type);
                        let badge = node.verification_status.badge();
                        let exact_marker = if (score - 1.0).abs() <= 0.001 {
                            "  ★ exact match"
                        } else {
                            ""
                        };
                        println!(
                            "  {} {} ID:{}  domains: {:?}  {}  score={:.4}{}",
                            badge, symbol, node_id, node.domains, node.label, score, exact_marker
                        );
                    }
                    if was_empty && semantic {
                        query_semantic_fallback(&braim, &cli.data_dir, &terms, cli.quiet);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Proximity { term_a, term_b } => {
            match braim.proximity(&term_a, &term_b) {
                Ok(paths) => {
                    if paths.is_empty() {
                        println!(
                            "No paths found between '{}' and '{}'.",
                            term_a, term_b
                        );
                        println!("  ⚠ Registered to gap register for investigation.");
                    } else {
                        println!("Proximity: '{}' → '{}'  ({} path{})\n", term_a, term_b, paths.len(), if paths.len() == 1 { "" } else { "s" });
                        for (i, path_info) in paths.iter().enumerate() {
                            println!("  Path {}  weight={:.4}", i + 1, path_info.weight);
                            for (j, &node_id) in path_info.path.iter().enumerate() {
                                let node = &braim.state.nodes[&node_id];
                                if j > 0 {
                                    print!("    → ");
                                } else {
                                    print!("    ");
                                }
                                println!("ID:{}  domains: {:?}  {}", node_id, node.domains, node.label);
                            }
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Perspective { term_a, term_b } => {
            match braim.perspective(&term_a, &term_b) {
                Ok(domain_weights) => {
                    if domain_weights.is_empty() {
                        println!(
                            "No paths found between '{}' and '{}'.",
                            term_a, term_b
                        );
                        println!("  ⚠ Registered to gap register for investigation.");
                    } else {
                        println!("Perspective: '{}' → '{}'  (grouped by domain)\n", term_a, term_b);
                        let mut domains: Vec<_> = domain_weights.iter().collect();
                        domains.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                        for (domain, weight) in domains {
                            println!("  [{}]  weight={:.4}  \"relationship exists in {} domain\"", domain, weight, domain.to_lowercase());
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Version(VersionCommands::Save { description }) => {
            let desc = if description.is_empty() {
                ""
            } else {
                &description
            };
            match braim.version_save(desc) {
                Ok(version_num) => {
                    println!("✓ Version {} saved — \"{}\"", version_num, desc);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Version(VersionCommands::List) => {
            match braim.version_list() {
                Ok(versions) => {
                    println!("Saved versions ({}):\n", versions.len());
                    for info in versions {
                        println!("  v{:04}  {} nodes {}  \"{}\"", info.version, info.node_count, info.saved_at, info.description);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Version(VersionCommands::Restore { n }) => {
            match braim.version_restore(n) {
                Ok(()) => {
                    let node_count = braim.state.nodes.len();
                    println!("✓ Restored to version {}  ({} nodes)", n, node_count);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Node { id, related } => {
            match braim.get_node(id) {
                Some(node) => {
                    let node_type_str = match node.node_type {
                        NodeType::Atomic => "atomic",
                        NodeType::Compound => "compound",
                        NodeType::Statement => "statement",
                        NodeType::Claim => "claim",
                        NodeType::Fact => "fact",
                        NodeType::InvalidStatement => "invalid_statement",
                        NodeType::ContestedStatement => "contested_statement",
                        NodeType::Source => "source",
                    };
                    let status_str = match node.status {
                        graph::NodeStatus::Active => "active",
                        graph::NodeStatus::Pending => "pending",
                        graph::NodeStatus::Deprecated => "deprecated",
                    };

                    println!("Node ID:{}", id);
                    println!("  Type: {}", node_type_str);
                    println!("  Label: {}", node.label);
                    println!("  Domains: {:?}", node.domains);
                    println!("  Sources: {:?}", node.sources);
                    if !node.source_ids.is_empty() {
                        println!("  Source entities:");
                        for &src_id in &node.source_ids {
                            if let Some(src) = braim.get_node(src_id) {
                                let type_str = src.source_type.as_deref().unwrap_or("?");
                                let loc_str = src.location.as_deref().unwrap_or("");
                                println!("    ID:{}  {}  ({})  {}", src_id, src.label, type_str, loc_str);
                            } else {
                                println!("    ID:{}  (not found)", src_id);
                            }
                        }
                    }
                    println!("  Status: {}", status_str);
                    println!("  Created: {}", node.created_at);

                    if node.node_type.is_statement_family() {
                        let verify_str = match node.verification_status {
                            graph::VerificationStatus::Invalid => "invalid",
                            graph::VerificationStatus::Unproven => "unproven",
                            graph::VerificationStatus::Contested => "contested",
                            graph::VerificationStatus::Partial => "partial",
                            graph::VerificationStatus::Proven => "proven",
                            graph::VerificationStatus::ProvenStrong => "proven_strong",
                        };
                        println!("  Verification: {} ({} domains)", verify_str, node.verified_by.len());
                    }

                    if !node.depends_on.is_empty() {
                        println!("  Depends on:");
                        for (dep_id, weight) in &node.depends_on {
                            if let Some(dep_node) = braim.get_node(*dep_id) {
                                println!("    ID:{}  {}  (weight: {:.4})", dep_id, dep_node.label, weight);
                            }
                        }
                    }

                    if related {
                        let (_, depended_by_nodes) = braim.get_related_nodes(id);

                        if !depended_by_nodes.is_empty() {
                            println!("\n  Referenced by:");
                            for (ref_id, ref_node) in depended_by_nodes {
                                println!("    ID:{}  {}  ({})", ref_id, ref_node.label, match ref_node.node_type {
                                    NodeType::Atomic => "atomic",
                                    NodeType::Compound => "compound",
                                    NodeType::Statement => "statement",
                                    NodeType::Claim => "claim",
                                    NodeType::Fact => "fact",
                                    NodeType::InvalidStatement => "invalid_statement",
                                    NodeType::ContestedStatement => "contested_statement",
                                    NodeType::Source => "source",
                                });
                            }
                        }
                    }

                    Ok(())
                }
                None => Err(format!("Error: Node ID {} not found", id)),
            }
        }
        Commands::Audit { semantic } => {
            let report = braim.audit();

            println!("── Orphan nodes (active, unreferenced, no dependencies) ──");
            if report.orphans.is_empty() {
                println!("  none");
            } else {
                for node in &report.orphans {
                    println!("  ⏳ ID:{}  domains: {:?}  {}", node.id, node.domains, node.label);
                }
            }

            println!("\n── Pending nodes (declared but unintegrated) ──");
            if report.pending.is_empty() {
                println!("  none");
            } else {
                for node in &report.pending {
                    println!("  ⏳ ID:{}  domains: {:?}  {}", node.id, node.domains, node.label);
                }
            }

            println!("\n── Gap register (zero-path pairs — pending investigation) ──");
            if report.gaps.is_empty() {
                println!("  none");
            } else {
                for gap in &report.gaps {
                    println!(
                        "  ✗ ID:{} '{}'  ←→  ID:{} '{}'",
                        gap.concept_a, gap.label_a, gap.concept_b, gap.label_b
                    );
                    println!("    {}", gap.note);
                }
            }

            println!("\n── Deprecated nodes still referenced ──");
            if report.deprecated_referenced.is_empty() {
                println!("  none");
            } else {
                for node in &report.deprecated_referenced {
                    println!("  ⚠ ID:{}  domains: {:?}  {}", node.id, node.domains, node.label);
                }
            }

            println!("\n── Refuted causal links (because_of failed inverse test) ──");
            if report.refuted_causal_links.is_empty() {
                println!("  none");
            } else {
                for e in &report.refuted_causal_links {
                    println!("  ✗ ID:{} '{}'  ─because→  ID:{} '{}'", e.from, e.from_label, e.to, e.to_label);
                    if let Some(r) = &e.reason {
                        println!("    {} — re-investigate the cause of ID:{}", r, e.from);
                    }
                }
            }

            println!("\n── Statements flagged for re-investigation (cause invalidated below) ──");
            if report.reinvestigate_flagged.is_empty() {
                println!("  none");
            } else {
                for node in &report.reinvestigate_flagged {
                    let note = node.metadata.get("because_of_reinvestigate").map(|s| s.as_str()).unwrap_or("");
                    println!("  ⚠ ID:{}  {}  ({})", node.id, node.label, note);
                }
            }

            println!("\n── Untested causal links (because_of without inverse test) ──");
            if report.untested_causal_links.is_empty() {
                println!("  none");
            } else {
                for e in &report.untested_causal_links {
                    println!("  ○ ID:{} '{}'  ─because→  ID:{} '{}'  — run: braim why-test {}",
                        e.from, e.from_label, e.to, e.to_label, e.from);
                }
            }

            println!("\n── Unverified root causes (chain bottoms out below proven) ──");
            if report.unverified_roots.is_empty() {
                println!("  none");
            } else {
                for node in &report.unverified_roots {
                    println!("  ○ ID:{}  [{}]  {}", node.id, node.verification_status.label(), node.label);
                }
            }

            let statements: Vec<_> = braim.state.nodes.values()
                .filter(|n| n.node_type.is_statement_family())
                .collect();
            println!("\n── Statement verification status (Rule of 3) ──");
            if statements.is_empty() {
                println!("  none");
            } else {
                let mut by_status: std::collections::HashMap<_, Vec<_>> = std::collections::HashMap::new();
                for stmt in &statements {
                    by_status.entry(stmt.verification_status.clone())
                        .or_insert_with(Vec::new)
                        .push(stmt);
                }

                let status_order = [
                    graph::VerificationStatus::ProvenStrong,
                    graph::VerificationStatus::Proven,
                    graph::VerificationStatus::Partial,
                    graph::VerificationStatus::Contested,
                    graph::VerificationStatus::Unproven,
                    graph::VerificationStatus::Invalid,
                ];

                for status in &status_order {
                    if let Some(stmts) = by_status.get(status) {
                        let symbol = match status {
                            graph::VerificationStatus::ProvenStrong => "✓✓✓",
                            graph::VerificationStatus::Proven => "✓✓",
                            graph::VerificationStatus::Partial => "✓",
                            graph::VerificationStatus::Contested => "⚠",
                            graph::VerificationStatus::Unproven => "○",
                            graph::VerificationStatus::Invalid => "✗",
                        };
                        for stmt in stmts {
                            println!("  {} ID:{}  {}  ({} verifications)  {}",
                                symbol, stmt.id, format!("{:?}", status).to_lowercase(),
                                stmt.verified_by.len(), stmt.label);
                        }
                    }
                }
            }

            // Show invalidated statements
            let invalid_stmts: Vec<_> = braim.state.nodes.values()
                .filter(|n| n.node_type.is_statement_family() && n.invalid)
                .collect();

            if !invalid_stmts.is_empty() {
                println!("\n── Invalidated statements (refuted claims) ──");
                for stmt in invalid_stmts {
                    println!("  ✗ ID:{}  {}", stmt.id, stmt.label);
                    if let Some(reason) = &stmt.invalid_reason {
                        println!("    Reason: {}", reason);
                    }
                    if let Some(invalidated_at) = &stmt.invalidated_at {
                        println!("    Invalidated: {}", invalidated_at);
                    }
                }
            }

            if semantic {
                run_semantic_audit(&braim, &cli.data_dir)
            } else {
                Ok(())
            }
        }
        Commands::Domains => {
            let domain_counts = braim.get_domain_stats();

            if domain_counts.is_empty() {
                println!("No domains defined yet.");
            } else {
                println!("Domain                    Count");
                println!("──────────────────────────────────");
                let mut total = 0;
                for (domain, count) in domain_counts {
                    println!("{:<25} {}", domain, count);
                    total += count;
                }
                println!("──────────────────────────────────");
                println!("{:<25} {}", "TOTAL", total);
            }

            Ok(())
        }
        Commands::List { domain, r#type, meta } => {
            let mut nodes: Vec<_> = braim.state.nodes.values().collect();
            nodes.sort_by_key(|n| n.id);

            if let Some(d) = &domain {
                nodes.retain(|n| n.domains.contains(d));
            }

            if let Some(kv) = &meta {
                match kv.split_once('=') {
                    Some((k, v)) => {
                        let ids = braim.nodes_by_meta(k, v);
                        nodes.retain(|n| ids.contains(&n.id));
                    }
                    None => {
                        return Err("--meta must be key=value (e.g. scope=cognitivex_flow)".to_string());
                    }
                }
            }

            if let Some(t) = &r#type {
                nodes.retain(|n| {
                    match t.as_str() {
                        "atomic" => n.node_type == NodeType::Atomic,
                        "compound" => n.node_type == NodeType::Compound,
                        // "statement" matches the whole statement family (legacy + new variants)
                        "statement" => n.node_type.is_statement_family(),
                        "claim" => n.node_type == NodeType::Claim,
                        "fact" => n.node_type == NodeType::Fact,
                        "invalid_statement" => n.node_type == NodeType::InvalidStatement,
                        _ => true,
                    }
                });
            }

            println!("ID     Type             Verification     Domains          Label");
            println!("────────────────────────────────────────────────────────────────────");
            for node in nodes {
                let type_str = match node.node_type {
                    NodeType::Atomic => "● atomic",
                    NodeType::Compound => "◉ compound",
                    NodeType::Statement => "▶ statement",
                    NodeType::Claim => "? claim",
                    NodeType::Fact => "▶ fact",
                    NodeType::InvalidStatement => "✗ invalid",
                    NodeType::ContestedStatement => "⚠ contested",
                    NodeType::Source => "◈ source",
                };
                let verify_str = if node.node_type.is_statement_family() {
                    format!("{} {}", node.verification_status.badge(), node.verification_status.label())
                } else {
                    "-".to_string()
                };
                let domains_str = node.domains.join(",");
                println!(
                    "{:5}  {:12} {:16} {:16} {}",
                    node.id, type_str, verify_str, domains_str, node.label
                );
            }

            Ok(())
        }
        Commands::Meta { id, set, inc, unset } => {
            if let Some(k) = unset {
                // Loud on a key that was not there: a silent success makes a
                // typo indistinguishable from a removal.
                match braim.unset_meta(id, &k)? {
                    true => println!("unset {}.metadata[{}]", id, k),
                    false => {
                        let node = braim.state.nodes.get(&id);
                        let keys: Vec<&str> = node
                            .map(|n| {
                                let mut ks: Vec<&str> = n.metadata.keys().map(|s| s.as_str()).collect();
                                ks.sort();
                                ks
                            })
                            .unwrap_or_default();
                        return Err(format!(
                            "Error: node {} has no metadata key '{}'{}",
                            id,
                            k,
                            if keys.is_empty() {
                                " (it has none at all)".to_string()
                            } else {
                                format!(" — it has: {}", keys.join(", "))
                            }
                        ));
                    }
                }
            } else if let Some(kv) = set {
                match kv.split_once('=') {
                    Some((k, v)) => match braim.set_meta(id, k, v) {
                        Ok(_) => println!("set {}.metadata[{}] = {}", id, k, v),
                        Err(e) => { return Err(format!("{}", e)); }
                    },
                    None => { return Err("--set must be key=value".to_string()); }
                }
            } else if let Some(k) = inc {
                match braim.inc_meta(id, &k) {
                    Ok(n) => println!("{}.metadata[{}] = {}", id, k, n),
                    Err(e) => { return Err(format!("{}", e)); }
                }
            } else {
                match braim.state.nodes.get(&id) {
                    Some(node) if node.metadata.is_empty() => println!("node {} has no metadata", id),
                    Some(node) => {
                        let mut keys: Vec<_> = node.metadata.keys().collect();
                        keys.sort();
                        for k in keys { println!("  {} = {}", k, node.metadata[k]); }
                    }
                    None => { return Err(format!("Error: Node ID {} does not exist", id)); }
                }
            }
            Ok(())
        }
        Commands::Serve { port } => {
            serve_viewer(braim.data_dir.to_str().unwrap_or(".braim"), port)
        }
        Commands::Import { source, filter_domain, only_proven, domain_map, full } => {
            let actual_source = if source.ends_with(".json") {
                source.clone()
            } else if source.ends_with(".braim") {
                format!("{}/current.json", source)
            } else {
                format!("{}/.braim/current.json", source)
            };

            // Parse domain mappings. Each --domain-map value may carry several
            // comma-separated pairs — the documented "old:new,old2:new2" form.
            let mut domain_mappings = HashMap::new();
            for mapping in domain_map {
                for pair in mapping.split(',').filter(|p| !p.trim().is_empty()) {
                    let parts: Vec<&str> = pair.split(':').collect();
                    if parts.len() != 2 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
                        return Err(format!("Error: Invalid domain mapping '{}'. Use --domain-map \"source:target[,source2:target2]\"", pair));
                    }
                    domain_mappings.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
                }
            }

            match braim.import_graph(
                &actual_source,
                filter_domain.as_deref(),
                if only_proven { Some(VerificationStatus::Proven) } else { None },
                domain_mappings,
                full,
            ) {
                Ok(manifest) => {
                    println!("✓ Import complete{}", if full { " (full-fidelity)" } else { "" });
                    println!("  Imported: {} nodes", manifest.imported_count);
                    println!("  Deduplicated: {} (skipped, target version kept)", manifest.deduplicated_count);
                    println!("  Filtered out: {} (by domain/status)", manifest.skipped_count);
                    if manifest.counterfactuals_refused > 0 {
                        println!("  Quarantined: {} counterfactual node(s) refused (braim ID:322)", manifest.counterfactuals_refused);
                    }
                    if full {
                        println!("  Source entities: {} imported", manifest.sources_imported);
                        println!("  Edges carried: {} because_of, {} contradicts", manifest.because_of_imported, manifest.contradicts_imported);
                        println!("  Dedup targets with unioned sources: {}", manifest.sources_unioned);
                    }

                    if !manifest.duplicates.is_empty() {
                        println!("\n── Duplicates found ──");
                        for dup in &manifest.duplicates {
                            println!("  ID:{} → ID:{}  {}  ({})", dup.source_id, dup.target_id, dup.reason, dup.target_label);
                        }
                    }

                    if manifest.imported_count > 0 {
                        println!("\n── ID Mappings (source → target) ──");
                        let mut mappings: Vec<_> = manifest.id_mappings.iter().collect();
                        mappings.sort_by_key(|&(src, _)| src);
                        for (src, tgt) in mappings {
                            if manifest.duplicates.iter().all(|d| d.source_id != *src) {
                                println!("  {} → {}", src, tgt);
                            }
                        }
                    }

                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Source(SourceCommands::Add { label, r#type, location, ingested_by, strict_sources }) => {
            if Braim::label_has_line_number_suffix(&label) {
                let msg = format!(
                    "Source label '{}' contains a line-number suffix. Line numbers are volatile — use the file path only and record the range in --sources metadata (e.g. test:{}:104-127).",
                    label, label.rfind(':').map(|i| &label[..i]).unwrap_or(&label)
                );
                if strict_sources {
                    return Err(format!("Error: {}", msg));
                } else {
                    eprintln!("⚠ {}", msg);
                }
            }
            match braim.add_source(&label, &r#type, location, ingested_by) {
                Ok(id) => {
                    println!("✓ source added");
                    println!("  ID:{}  type: {}  label: {}", id, r#type, label);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::AddSource { id, source_id }) => {
            match braim.add_source_to_statement(id, source_id) {
                Ok(AddSourceResult { auto_resolved: true, winner_id, loser_id, winner_status, .. }) => {
                    println!("✓ Source ID:{} attached to statement ID:{}", source_id, id);
                    println!("  ⚡ Auto-resolved contradiction (Mechanism A):");
                    println!("    Winner ID:{} → {}", winner_id.unwrap(), winner_status.map(|s| s.label()).unwrap_or("?"));
                    println!("    Loser  ID:{} → invalid", loser_id.unwrap());
                    Ok(())
                }
                Ok(AddSourceResult { auto_resolved: false, .. }) => {
                    let node = &braim.state.nodes[&id];
                    println!("✓ Source ID:{} attached to statement ID:{}", source_id, id);
                    println!("  Verification: {} {}", node.verification_status.badge(), node.verification_status.label());
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::Contradict { stmt_a, stmt_b, reason, source }) => {
            match braim.contradict_statements(stmt_a, stmt_b, &reason, source) {
                Ok(()) => {
                    println!("⚠ Statements ID:{} and ID:{} marked CONTESTED", stmt_a, stmt_b);
                    println!("  Reason: {}", reason);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Statement(StatementCommands::ResolveContradiction { stmt_a, stmt_b, winner, reason, source }) => {
            let loser = if winner == stmt_a { stmt_b } else { stmt_a };
            match braim.resolve_contradiction(winner, loser, &reason, source) {
                Ok(()) => {
                    println!("✓ Contradiction resolved");
                    println!("  Winner: ID:{}", winner);
                    println!("  Loser: ID:{} → invalid", loser);
                    println!("  Reason: {}", reason);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::MigrateRefutation { apply, json } => {
            let repairs = braim.migrate_refutation_cascade(apply)?;
            if json {
                let payload: Vec<_> = repairs.iter().map(|r| serde_json::json!({
                    "id": r.id,
                    "refuted_by": r.refuted_by,
                    "label": r.label,
                    "restored": r.restored.label(),
                    "cause_recovered": r.cause_recovered,
                })).collect();
                let text = serde_json::to_string_pretty(&serde_json::json!({
                    "applied": apply,
                    "count": repairs.len(),
                    "repairs": payload,
                })).map_err(|e| format!("Failed to serialize the migration report: {}", e))?;
                println!("{}", text);
                return Ok(());
            }
            if repairs.is_empty() {
                println!("Nothing to migrate — no node carries a depends_on_invalidated reason.");
                return Ok(());
            }
            println!("{} node(s) were refuted as collateral by the pre-3.3 cascade{}:\n",
                repairs.len(), if apply { ", now repaired" } else { " (dry run)" });
            let mut by_status: std::collections::BTreeMap<&str, usize> = Default::default();
            for r in &repairs {
                *by_status.entry(r.restored.label()).or_insert(0) += 1;
                println!("  ID:{}  invalid → {}{}", r.id, r.restored.label(),
                    if r.cause_recovered { "   (cause ID:".to_string() + &r.refuted_by.to_string() + " is no longer refuted)" } else { String::new() });
                println!("        {}", r.label.chars().take(110).collect::<String>());
            }
            println!("\nRestored to: {}", by_status.iter()
                .map(|(k, v)| format!("{} {}", v, k)).collect::<Vec<_>>().join(", "));
            println!("Each carries support_withdrawn_by — review with: braim list --meta support_withdrawn_by=<cause>");
            if apply {
                println!("\nWritten. Checkpoint: braim version save \"refutation cascade migrated\"");
            } else {
                println!("\nNothing was written. Checkpoint first, then: braim migrate-refutation --apply");
            }
            Ok(())
        }
        Commands::MigrateNodeTypes => {
            match braim.migrate_node_types() {
                Ok(changed) => {
                    if changed == 0 {
                        println!("✓ No migration needed — all node_types already derived from verification_status");
                    } else {
                        println!("✓ Migrated {} node(s) to claim/fact/invalid_statement", changed);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Export { domain, to, include_unproven, domain_map } => {
            // Fall back to the central recorded at bootstrap, so routine
            // publishing is `braim export <domain>` with nothing to remember.
            let to = match to.or_else(|| bootstrap::read_central_pointer(&braim.data_dir)) {
                Some(t) => t,
                None => {
                    return Err(format!("Error: no target. Pass --to <dir>, or record one once with \
                               `braim init --team --central <dir>`."));
                }
            };
            let mut domain_mappings = HashMap::new();
            for mapping in domain_map {
                for pair in mapping.split(',').filter(|p| !p.trim().is_empty()) {
                    let parts: Vec<&str> = pair.split(':').collect();
                    if parts.len() != 2 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
                        return Err(format!("Error: Invalid domain mapping '{}'. Use --domain-map \"source:target[,source2:target2]\"", pair));
                    }
                    domain_mappings.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
                }
            }
            // Mappings apply before filtering, so filter on the post-map name.
            let effective_domain = domain_mappings.get(&domain).cloned().unwrap_or_else(|| domain.clone());

            // The target is mutated by this export, so it takes its own write
            // lock; the source graph above stays read-only and unlocked.
            match Braim::open_for_write(&to) {
                Ok(mut target) => {
                    match target.import_state(
                        braim.state.clone(),
                        Some(&effective_domain),
                        // Floor at Partial, not Proven: one PRIMARY source is real
                        // evidence and must be publishable, or two teammates each
                        // holding one type can never corroborate (braim ID:253).
                        if include_unproven { None } else { Some(VerificationStatus::Partial) },
                        domain_mappings,
                        true,
                    ) {
                        Ok(manifest) => {
                            println!("✓ Exported domain '{}' → {}", effective_domain, to);
                            println!("  Published: {} nodes ({} deduplicated into existing central nodes)",
                                manifest.imported_count, manifest.deduplicated_count);
                            println!("  Source entities: {}  Edges: {} because_of, {} contradicts",
                                manifest.sources_imported, manifest.because_of_imported, manifest.contradicts_imported);
                            if manifest.sources_unioned > 0 {
                                println!("  Corroboration: {} central node(s) gained sources from this export", manifest.sources_unioned);
                            }
                            if manifest.counterfactuals_refused > 0 {
                                println!("  Quarantined: {} counterfactual node(s) held back — a what-if is a hypothesis,",
                                    manifest.counterfactuals_refused);
                                println!("               and no source can prove one (braim ID:322).");
                            }
                            println!("\nCheckpoint central: braim --data-dir {} version save \"export {} from $(pwd)\"", to, effective_domain);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            }
        }
        Commands::Dream(DreamCommands::Candidates {
            limit,
            strategy,
            min_semantic,
            include_scratch,
            replay,
            json,
        }) => {
            if let Err(e) = dream::refuse_if_central(&braim.data_dir) {
                return Err(format!("{}", e));
            }
            // Default set omits `semantic`: it needs the embedding index, which
            // costs a model load and a full pass. Ask for it explicitly.
            let strategies = match strategy.as_deref() {
                None => vec![Strategy::SharedSource, Strategy::TwoHop, Strategy::RegisteredGap],
                Some(list) => {
                    let mut out = Vec::new();
                    for part in list.split(',').filter(|p| !p.trim().is_empty()) {
                        match Strategy::parse(part) {
                            Ok(s) => out.push(s),
                            Err(e) => {
                                return Err(format!("{}", e));
                            }
                        }
                    }
                    out
                }
            };
            let opts = DreamOptions {
                min_semantic: min_semantic.unwrap_or(dream::DEFAULT_MIN_SEMANTIC),
                limit,
                include_scratch,
                replay,
                strategies: strategies.clone(),
            };
            let semantic_pairs = if strategies.contains(&Strategy::Semantic) {
                semantic_pair_scores(&braim, &cli.data_dir, opts.min_semantic, cli.quiet)
            } else {
                Vec::new()
            };
            let found = dream::candidates(&braim, &opts, &semantic_pairs);
            print_candidates(&found, json, limit);
            Ok(())
        }
        Commands::Dream(DreamCommands::Constraints { limit, include_scratch, include_walked, json }) => {
            if let Err(e) = dream::refuse_if_central(&braim.data_dir) {
                return Err(format!("{}", e));
            }
            let found = dream::constraints(&braim, limit, include_scratch, include_walked);
            if json {
                // Fail loudly: a consumer piping --json into a tool must not
                // read empty stdout plus exit 0 as "no constraints".
                let text = serde_json::to_string_pretty(&found)
                    .map_err(|e| format!("Failed to serialize constraints: {}", e))?;
                println!("{}", text);
            } else if found.shown.is_empty() {
                if found.walked_hidden > 0 {
                    println!("Nothing new: all {} load-bearing cause(s) have been walked and no \
                              statement has arrived since. --include-walked to walk one again.",
                        found.walked_hidden);
                } else {
                    println!("No load-bearing causes — the graph has no because_of chains to rank.");
                }
            } else {
                println!("Load-bearing causes ({} of {} ranked):\n", found.shown.len(), found.ranked);
                for c in &found.shown {
                    println!("  {:.2}  ID:{}  [{}]{}{}", c.score, c.id, c.verification,
                        if c.reads_as_limitation { "  reads-as-limitation" } else { "" },
                        if c.reopened.is_some() { "  REOPENED" } else { "" });
                    println!("        {}", c.label);
                    println!("        why: {}", c.rationale);
                    if let Some(r) = &c.reopened {
                        println!("        reopened: {}", r);
                    }
                    println!();
                }
                if found.dropped() > 0 {
                    println!("{} more ranked below the cut — raise --limit to see them.\n", found.dropped());
                }
                if found.walked_hidden > 0 {
                    println!("{} already walked with nothing new since — --include-walked to see them.\n",
                        found.walked_hidden);
                }
                println!("Leverage only — whether these are constraints, and whether they can be");
                println!("relaxed, is the judgement call. Read each one before acting on it.");
            }
            Ok(())
        }
        Commands::Dream(DreamCommands::Whatif { id, signals, include_scratch, json }) => {
            dream::refuse_if_central(&braim.data_dir)?;
            let cf = dream::counterfactual(&braim, id, include_scratch, signals)?;
            if json {
                let text = serde_json::to_string_pretty(&cf)
                    .map_err(|e| format!("Failed to serialize counterfactual: {}", e))?;
                println!("{}", text);
                return Ok(());
            }
            println!("What-if ID:{}  [{}]\n  {}\n", cf.id, cf.verification, cf.label);

            // Staleness first: if the constraint is already obsolete there is no
            // counterfactual to write, only a contradiction to raise.
            if cf.stale_signals.is_empty() {
                println!("Staleness: no statement cites the same source file more recently.\n");
            } else {
                println!("Staleness signals ({}) — read these before imagining anything:", cf.stale_signals.len());
                for sg in &cf.stale_signals {
                    println!("  ID:{}  [{}]  {}", sg.id, sg.verification, sg.why);
                    println!("        {}", sg.label);
                }
                println!("  If one of these supersedes the constraint, that is a contradiction, not a dream:");
                println!("  braim statement contradict {} <that_id> --reason \"...\" --source <id>\n", cf.id);
            }

            if cf.rests_on.is_empty() {
                println!("Nothing rests on this yet — relaxing it moves nothing measurable.\n");
            } else {
                println!("Rests on it ({}):", cf.rests_on.len());
                for l in &cf.rests_on {
                    println!("  {}{} ID:{}  [{}]", "  ".repeat(l.depth - 1), "└─", l.id, l.verification);
                    println!("  {}   {}", "  ".repeat(l.depth - 1), l.label);
                }
                println!();
            }

            match (&cf.root, cf.serves.len()) {
                (Some(r), n) => {
                    println!("Serves ({} link(s) up to the root goal):", n);
                    for l in &cf.serves {
                        println!("  {}↑ ID:{}  [{}]  {}", "  ".repeat(l.depth - 1), l.id, l.verification, l.label);
                    }
                    println!("  root: ID:{}\n", r.id);
                }
                (None, _) => println!("Serves: nothing records what this constraint ultimately answers to.\n"),
            }

            println!("Frame:\n{}", cf.frame);
            Ok(())
        }
        Commands::Dream(DreamCommands::Flag { note, kind, nodes }) => {
            let ids = match nodes.as_deref() {
                Some(list) => {
                    let parsed: Result<Vec<u32>, _> =
                        list.split(',').map(|x| x.trim().parse::<u32>()).collect();
                    match parsed {
                        Ok(v) => v,
                        Err(_) => return Err("Error: --nodes must be comma-separated ids (e.g. \"412,88\")".to_string()),
                    }
                }
                None => Vec::new(),
            };
            for id in &ids {
                if !braim.state.nodes.contains_key(id) {
                    return Err(format!("Error: node ID:{} does not exist — flag what is there", id));
                }
            }
            let item = dream::flag(&braim.data_dir, &kind, &note, ids)?;
            println!("✓ review item {} raised [{}]", item.id, item.kind);
            println!("  {}", item.note);
            if !item.nodes.is_empty() {
                println!("  nodes: {}", item.nodes.iter().map(|i| format!("ID:{}", i))
                    .collect::<Vec<_>>().join(", "));
            }
            println!("\nRead the queue: braim dream review");
            Ok(())
        }
        Commands::Dream(DreamCommands::Review { all, json }) => {
            let items = dream::pending(&braim.data_dir, all);
            if json {
                let text = serde_json::to_string_pretty(&items)
                    .map_err(|e| format!("Failed to serialize the review queue: {}", e))?;
                println!("{}", text);
                return Ok(());
            }
            let open = items.iter().filter(|i| i.cleared_at.is_none()).count();
            if items.is_empty() {
                println!("Review queue empty — nothing a night flagged is waiting.");
            } else {
                println!("Review queue ({} pending{}):\n", open,
                    if all { format!(", {} cleared", items.len() - open) } else { String::new() });
                for i in &items {
                    let mark = match &i.cleared_at {
                        Some(at) => format!("  ✓ cleared {}", at),
                        None => String::new(),
                    };
                    println!("  [{}] {}  raised {}{}", i.id, i.kind, i.raised_at, mark);
                    println!("      {}", i.note);
                    if !i.nodes.is_empty() {
                        println!("      nodes: {}", i.nodes.iter().map(|n| format!("ID:{}", n))
                            .collect::<Vec<_>>().join(", "));
                    }
                    if let Some(n) = &i.cleared_note {
                        println!("      resolution: {}", n);
                    }
                    println!();
                }
                if open > 0 {
                    println!("Sign one off: braim dream reviewed <id> --note \"<what you did>\"");
                }
            }
            // The queue holds what has no other home; nodes have one, so point at
            // it rather than duplicating them here.
            let created = braim.state.nodes.values()
                .filter(|n| n.metadata.get("scope").map(|s| s == "dream").unwrap_or(false))
                .count();
            if created > 0 {
                println!("{} node(s) carry scope=dream — braim list --meta scope=dream", created);
            }
            Ok(())
        }
        Commands::Dream(DreamCommands::Reviewed { id, note }) => {
            let item = dream::clear(&braim.data_dir, id, note)?;
            println!("✓ review item {} signed off", item.id);
            println!("  {}", item.note);
            Ok(())
        }
        Commands::Dream(DreamCommands::Log { since, verdict, limit, json }) => {
            if let Some(v) = &verdict {
                const VERDICTS: [&str; 5] =
                    ["no-relation", "proposed", "verified", "contradiction", "duplicate"];
                if !VERDICTS.contains(&v.as_str()) {
                    return Err(format!("Error: --verdict must be one of {} (got '{}')",
                        VERDICTS.join(", "), v));
                }
            }
            let entries = dream::log(&braim.data_dir, since.as_deref(), verdict.as_deref(), limit);
            if json {
                let text = serde_json::to_string_pretty(&entries)
                    .map_err(|e| format!("Failed to serialize the ledger: {}", e))?;
                println!("{}", text);
                return Ok(());
            }
            let total = dream::load_ledger(&braim.data_dir).len();
            if entries.is_empty() {
                println!("No ledger entries match ({} recorded in total).", total);
                return Ok(());
            }
            println!("Dream ledger ({} shown of {} recorded):\n", entries.len(), total);
            for e in &entries {
                println!("  {}  ID:{} ↔ ID:{}  [{}]", e.recorded_at, e.a, e.b, e.verdict);
                if let Some(n) = &e.note {
                    println!("      {}", n);
                }
            }
            Ok(())
        }
        Commands::Dream(DreamCommands::Seen { a, b, verdict, note }) => {
            if let Err(e) = dream::refuse_if_central(&braim.data_dir) {
                return Err(format!("{}", e));
            }
            match dream::record_ledger(&braim.data_dir, a, b, &verdict, note) {
                Ok(()) => {
                    println!("✓ Dream ledger updated: ID:{} ↔ ID:{} = {}", a, b, verdict);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Policy { name } => {
            match bootstrap::policy_body(&name) {
                Ok(body) => {
                    println!("{}", body);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Init { team, central, settings } => {
            if !team {
                return Err("Error: pass --team (the only mode today). See `braim init --help`.".to_string());
            }
            let settings_path = std::path::PathBuf::from(
                settings.unwrap_or_else(|| ".claude/settings.json".to_string()),
            );
            let graph_dir = braim.data_dir.clone();
            // Constructing `braim` already created the dir; report whether it
            // had a graph before this run rather than claiming a fresh one.
            let graph_created = braim.state.nodes.is_empty();

            match bootstrap::install_hooks(&settings_path) {
                Ok(changes) => {
                    println!("✓ braim is set up for this project");
                    println!("  Graph: {} ({})", graph_dir.display(),
                        if graph_created { "new, empty" } else { "existing" });
                    println!("  Settings: {}", settings_path.display());
                    for c in &changes {
                        match c {
                            bootstrap::Change::Added(e) =>
                                println!("    + {} hook installed", e),
                            bootstrap::Change::AlreadyPresent(e) =>
                                println!("    = {} hook already present, left alone", e),
                        }
                    }
                    if let Some(c) = central {
                        match bootstrap::write_central_pointer(&graph_dir, &c) {
                            Ok(()) => println!("  Central: {} (recorded)", c),
                            Err(e) => eprintln!("⚠ could not record central pointer: {}", e),
                        }
                    }
                    println!("\nStart a new session so the hooks load, then work as usual.");
                    if changes.iter().any(|c| matches!(c, bootstrap::Change::Added(_))) {
                        println!("Verify a hook any time with: braim policy perturn");
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::MergeNodes { winner, loser } => {
            match braim.merge_nodes(winner, loser) {
                Ok(o) => {
                    println!("✓ Merged ID:{} into ID:{}", o.loser, o.winner);
                    println!("  Evidence gained: {} source(s)", o.sources_added);
                    println!("  Rewired: {} referent(s), {} edge(s)", o.referents_rewired, o.edges_rewired);
                    println!("  Winner status: {} {}", o.new_status.badge(), o.new_status.label());
                    if !o.dep_differences.is_empty() {
                        eprintln!(
                            "⚠ the merged node depended on {:?}, which ID:{} does not — NOT merged, \n  because that would change what the surviving statement asserts. Wire them \n  deliberately with 'braim statement update-deps {}' if they belong.",
                            o.dep_differences, o.winner, o.winner
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::RenameDomain { old, new } => {
            match braim.rename_domain(&old, &new) {
                Ok(touched) => {
                    println!("✓ Domain '{}' renamed to '{}'", old, new);
                    println!("  {} node(s) updated", touched);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Shard => {
            match braim.shard_layout() {
                Ok(domain_count) => {
                    println!("✓ Converted to sharded layout");
                    println!("  {} domain shard(s) under domains/", domain_count);
                    println!("  Cross-domain state in graph.json");
                    println!("  Previous single file archived as current.json.pre-shard");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::WhyAdd { consequent, because, source } => {
            match braim.why_add(consequent, because, source) {
                Ok(warning) => {
                    println!("✓ because_of edge recorded: ID:{} → ID:{}", consequent, because);
                    if let Some(w) = warning {
                        eprintln!("⚠ {}", w);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Why { id } => {
            match braim.why_chain(id) {
                Ok(chain) => {
                    println!("Why ID:{} — because_of chain to root cause:\n", id);
                    let last = chain.steps.len().saturating_sub(1);
                    for (i, step) in chain.steps.iter().enumerate() {
                        let label: String = step.label.chars().take(72).collect();
                        let is_root = i == last;
                        let connector = if i == 0 { "  " } else { "  ↳ because " };
                        print!("{}ID:{} [{}] {}", connector, step.id, step.verification_status.label(), label);
                        if is_root {
                            if step.edge_invalid {
                                print!("   ⟵ causal link refuted — re-investigate");
                            } else if chain.root_verified {
                                print!("   ⟵ root_cause");
                            } else {
                                print!("   ⟵ candidate root cause, unverified (add sources or extend the chain)");
                            }
                        }
                        println!();
                        if let Some(cs) = step.causal_status {
                            let tag = if step.edge_invalid {
                                "  refuted by inverse test".to_string()
                            } else if step.edge_tested {
                                "  inverse-tested".to_string()
                            } else {
                                String::new()
                            };
                            println!("       causal claim: {} {}{}", cs.badge(), cs.label(), tag);
                        }
                        if !step.contested_with.is_empty() {
                            let ids = step.contested_with.iter()
                                .map(|i| format!("ID:{}", i))
                                .collect::<Vec<_>>()
                                .join(", ");
                            println!("       ⚠ contested with {} — see contradicts edge", ids);
                        }
                    }
                    let verdict = if chain.root_verified { "verified" } else { "unverified" };
                    println!("\nroot cause: ID:{} ({})", chain.root_id, verdict);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::WhyTest { id, fail, source } => {
            match braim.why_test(id, !fail, source) {
                Ok(outcome) => {
                    if outcome.passed {
                        println!("✓ Inverse test PASSED for ID:{} → ID:{}", outcome.consequent, outcome.cause);
                        println!("  causal claim: {} {}", outcome.causal_status.badge(), outcome.causal_status.label());
                    } else {
                        println!("✗ Inverse test FAILED for ID:{} → ID:{}", outcome.consequent, outcome.cause);
                        println!("  causal link marked invalid; statements unchanged.");
                        println!("  → re-investigate the cause of ID:{}", outcome.consequent);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::WhyRemove { id } => {
            match braim.why_remove(id) {
                Ok(outcome) => {
                    let kind = if outcome.was_invalid { " (refuted)" } else { "" };
                    println!("✓ removed because_of edge ID:{} → ID:{}{}", outcome.consequent, outcome.cause, kind);
                    println!("  ID:{} now has no cause — reassign with: braim why-add {} --because <cause_id>", outcome.consequent, outcome.consequent);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Similar { text, top, min_score, rebuild, dedup } => {
            run_similar(&braim, &cli.data_dir, &text, top, min_score, rebuild, dedup)
        }
    }
}

/// `braim similar` — embedding nearest-neighbour search over node labels.
/// Builds/refreshes the sidecar index, embeds the query, prints cosine top-k.
#[cfg(feature = "embeddings")]
fn run_similar(
    braim: &Braim,
    data_dir: &str,
    text: &str,
    top: usize,
    min_score: f32,
    rebuild: bool,
    dedup: bool,
) -> Result<(), String> {
    use embed::{corpus, refresh_index, top_k, Embedder, EmbedIndex, FastEmbedder, EMBED_SIDECAR};

    let rows = corpus(braim);
    if rows.is_empty() {
        return Err("graph has no labelled nodes to search".to_string());
    }
    let data_path = std::path::Path::new(data_dir);
    let mut index = EmbedIndex::load(data_path);
    let mut embedder = FastEmbedder::new()?;

    let embedded = refresh_index(&mut embedder, &mut index, &rows, rebuild)?;
    if embedded > 0 {
        index.save(data_path)?;
        eprintln!(
            "(refreshed index: embedded {} node(s) -> {}/{})",
            embedded, data_dir, EMBED_SIDECAR
        );
    }

    let floor = if dedup { min_score.max(0.8) } else { min_score };
    let qv = embedder.embed(&[text.to_string()])?;
    let qv = qv.into_iter().next().ok_or("embedder returned no query vector")?;
    let hits = top_k(&qv, &index, &rows, top, floor, None);

    if hits.is_empty() {
        println!("No nodes above score {:.2}.", floor);
        return Ok(());
    }
    println!(
        "{} for: {:?}",
        if dedup { "Possible duplicates" } else { "Semantic matches" },
        text
    );
    for h in hits {
        let dup = if h.score >= 0.8 { "  ⚠ likely duplicate" } else { "" };
        let label: String = h.label.chars().take(96).collect();
        println!("  ID:{:<5} {:.3} [{}] {}{}", h.id, h.score, h.node_type, label, dup);
    }
    Ok(())
}

#[cfg(not(feature = "embeddings"))]
fn run_similar(
    _braim: &Braim,
    _data_dir: &str,
    _text: &str,
    _top: usize,
    _min_score: f32,
    _rebuild: bool,
    _dedup: bool,
) -> Result<(), String> {
    Err("`braim similar` requires the embeddings feature.\n\
         Rebuild with:  cargo build --release --features embeddings"
        .to_string())
}

#[cfg(feature = "embeddings")]
fn run_semantic_audit(braim: &Braim, data_dir: &str) -> Result<(), String> {
    use embed::{
        corpus, label_echoes, refresh_index, semantic_duplicates, EmbedIndex, FastEmbedder,
        DEDUP_WARN_THRESHOLD, ECHO_WARN_THRESHOLD, EMBED_SIDECAR,
    };

    let rows = corpus(braim);
    if rows.is_empty() {
        println!("\n── Semantic checks ──\n  graph has no labelled nodes");
        return Ok(());
    }
    let data_path = std::path::Path::new(data_dir);
    let mut index = EmbedIndex::load(data_path);
    let mut embedder = FastEmbedder::new()?;
    let embedded = refresh_index(&mut embedder, &mut index, &rows, false)?;
    if embedded > 0 {
        index.save(data_path)?;
        eprintln!(
            "(refreshed index: embedded {} node(s) -> {}/{})",
            embedded, data_dir, EMBED_SIDECAR
        );
    }

    let label_of = |id: u32| -> String {
        braim
            .state
            .nodes
            .get(&id)
            .map(|n| n.label.chars().take(60).collect())
            .unwrap_or_default()
    };

    let dups = semantic_duplicates(braim, &index, DEDUP_WARN_THRESHOLD);
    println!(
        "\n── Semantic near-duplicates (cosine >= {:.2}, advisory) ──",
        DEDUP_WARN_THRESHOLD
    );
    if dups.is_empty() {
        println!("  none");
    } else {
        for d in &dups {
            println!("  ⚠ {:.3}  ID:{}  '{}'", d.score, d.id_a, label_of(d.id_a));
            println!("           ID:{}  '{}'", d.id_b, label_of(d.id_b));
        }
    }

    let echoes = label_echoes(braim, &index, ECHO_WARN_THRESHOLD);
    println!(
        "\n── Label echoes (statement restates own dependency, cosine >= {:.2}, advisory) ──",
        ECHO_WARN_THRESHOLD
    );
    if echoes.is_empty() {
        println!("  none");
    } else {
        for e in &echoes {
            println!(
                "  ⚠ {:.3}  ID:{} '{}'  echoes dep ID:{} '{}'",
                e.score,
                e.statement_id,
                label_of(e.statement_id),
                e.dep_id,
                label_of(e.dep_id)
            );
        }
    }
    Ok(())
}

#[cfg(not(feature = "embeddings"))]
fn run_semantic_audit(_braim: &Braim, _data_dir: &str) -> Result<(), String> {
    Err("`braim audit --semantic` requires the embeddings feature.\n\
         Rebuild with:  cargo build --release --features embeddings"
        .to_string())
}

/// Advisory pre-add dedup check (phase 2). Embeds the candidate label and warns
/// — non-blocking — if any existing node is at/above DEDUP_WARN_THRESHOLD. Builds
/// or refreshes the sidecar index on demand. Never blocks: embeddings are
/// probabilistic and near-synonymous-but-distinct concepts can score high.
#[cfg(feature = "embeddings")]
fn dedup_warn(braim: &Braim, data_dir: &str, candidate: &str, quiet: bool) {
    use embed::{
        corpus, refresh_index, top_k, EmbedIndex, Embedder, FastEmbedder, DEDUP_WARN_THRESHOLD,
    };
    let rows = corpus(braim);
    if rows.is_empty() {
        return;
    }
    let data_path = std::path::Path::new(data_dir);
    let mut index = EmbedIndex::load(data_path);
    let mut embedder = match FastEmbedder::new() {
        Ok(e) => e,
        Err(e) => {
            if !quiet {
                eprintln!("(dedup check skipped: {e})");
            }
            return;
        }
    };
    match refresh_index(&mut embedder, &mut index, &rows, false) {
        Ok(n) if n > 0 => {
            let _ = index.save(data_path);
        }
        Ok(_) => {}
        Err(e) => {
            if !quiet {
                eprintln!("(dedup check skipped: {e})");
            }
            return;
        }
    }
    let qv = match embedder.embed(&[candidate.to_string()]) {
        Ok(mut v) => v.drain(..).next(),
        Err(_) => None,
    };
    let qv = match qv {
        Some(v) => v,
        None => return,
    };
    let hits = top_k(&qv, &index, &rows, 3, DEDUP_WARN_THRESHOLD, None);
    if hits.is_empty() {
        return;
    }
    eprintln!("⚠ possible duplicate(s) — lookup-first before adding:");
    for h in hits {
        let label: String = h.label.chars().take(90).collect();
        eprintln!("    ID:{} {:.3} [{}] {}", h.id, h.score, h.node_type, label);
    }
    eprintln!("  (advisory only; the add proceeds. Reuse an existing node if it is the same concept.)");
}

#[cfg(not(feature = "embeddings"))]
fn dedup_warn(_braim: &Braim, _data_dir: &str, _candidate: &str, quiet: bool) {
    if !quiet {
        eprintln!("(--check-dupes requires the embeddings feature; rebuild with --features embeddings)");
    }
}

/// `query --semantic` fallback (phase 3): when concept-graph traversal returns
/// nothing, search by meaning instead. Embeds the raw query terms and prints
/// embedding top-k above a noise floor. Reuses the sidecar index.
#[cfg(feature = "embeddings")]
fn query_semantic_fallback(braim: &Braim, data_dir: &str, terms: &str, quiet: bool) {
    use embed::{corpus, refresh_index, top_k, EmbedIndex, Embedder, FastEmbedder};
    let rows = corpus(braim);
    if rows.is_empty() {
        return;
    }
    let data_path = std::path::Path::new(data_dir);
    let mut index = EmbedIndex::load(data_path);
    let mut embedder = match FastEmbedder::new() {
        Ok(e) => e,
        Err(e) => {
            if !quiet {
                eprintln!("(semantic fallback skipped: {e})");
            }
            return;
        }
    };
    if let Ok(n) = refresh_index(&mut embedder, &mut index, &rows, false) {
        if n > 0 {
            let _ = index.save(data_path);
        }
    }
    let qv = match embedder.embed(&[terms.to_string()]) {
        Ok(mut v) => v.drain(..).next(),
        Err(_) => None,
    };
    let qv = match qv {
        Some(v) => v,
        None => return,
    };
    // 0.30 noise floor: real matches on clean corpora land 0.4-0.7; below ~0.35
    // is drift. Better to show "nothing relevant" than a wall of noise.
    let hits = top_k(&qv, &index, &rows, 8, 0.30, None);
    if hits.is_empty() {
        return;
    }
    println!("\nNo concept-graph match — semantic fallback (by meaning):");
    for h in hits {
        let label: String = h.label.chars().take(96).collect();
        println!("  ID:{:<5} {:.3} [{}] {}", h.id, h.score, h.node_type, label);
    }
}

#[cfg(not(feature = "embeddings"))]
fn query_semantic_fallback(_braim: &Braim, _data_dir: &str, _terms: &str, quiet: bool) {
    if !quiet {
        eprintln!("(--semantic requires the embeddings feature; rebuild with --features embeddings)");
    }
}

fn serve_viewer(data_dir: &str, port: u16) -> Result<(), String> {
    use tiny_http::{Response, Header};

    let addr = format!("127.0.0.1:{}", port);
    let server = tiny_http::Server::http(&addr)
        .map_err(|e| format!("Failed to start server: {}", e))?;

    println!("✓ BRAIM Viewer running at http://localhost:{}", port);
    println!("  Open: http://localhost:{}/", port);
    println!("  Press Ctrl+C to stop");
    println!();

    for request in server.incoming_requests() {
        let response = match request.url() {
            "/" | "/viewer.html" => {
                Response::from_string(include_str!("../viewer.html"))
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap())
            }
            "/current.json" => {
                // Load through Braim so both layouts work: single-file dirs and
                // sharded dirs (domains/ + graph.json) serve the same merged view.
                match Braim::new(data_dir).and_then(|b| b.state_json()) {
                    Ok(content) => Response::from_string(content)
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap()),
                    Err(_) => Response::from_string("{\"error\": \"Data not found\"}")
                        .with_status_code(404),
                }
            }
            _ => Response::from_string("404 Not Found").with_status_code(404),
        };

        let _ = request.respond(response);
    }

    Ok(())
}

/// Render the dream worklist. JSON is the agent-loop surface; the text form is
/// for a human sanity-checking what the night will chew on.
fn print_candidates(found: &[Candidate], json: bool, limit: usize) {
    if json {
        match serde_json::to_string_pretty(found) {
            Ok(s) => println!("{}", s),
            Err(e) => eprintln!("Failed to serialize candidates: {}", e),
        }
        return;
    }
    if found.is_empty() {
        println!("No dream candidates — every eligible pair is already linked or adjudicated.");
        return;
    }
    println!("Dream candidates ({} of max {}):\n", found.len(), limit);
    for c in found {
        println!(
            "  {:.2}  [{}]  ID:{} ↔ ID:{}",
            c.score,
            c.strategies.join("+"),
            c.a,
            c.b
        );
        println!("        A: {}  {:?}", c.a_label, c.a_domains);
        println!("        B: {}  {:?}", c.b_label, c.b_domains);
        println!("        why: {}\n", c.rationale);
    }
    println!("Adjudicate each pair, then record it:");
    println!("  braim dream seen <a> <b> --verdict no-relation|proposed|verified|contradiction");
}

/// All node pairs whose labels sit above `threshold` cosine. Quadratic in nodes,
/// which is fine for a local working graph — and dreaming is refused on central,
/// the only graph large enough for that to matter.
#[cfg(feature = "embeddings")]
fn semantic_pair_scores(
    braim: &Braim,
    data_dir: &str,
    threshold: f32,
    quiet: bool,
) -> Vec<(u32, u32, f32)> {
    use embed::{corpus, cosine, refresh_index, EmbedIndex, FastEmbedder, EMBED_SIDECAR};

    let rows = corpus(braim);
    if rows.is_empty() {
        return Vec::new();
    }
    let data_path = std::path::Path::new(data_dir);
    let mut index = EmbedIndex::load(data_path);
    let mut embedder = match FastEmbedder::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("(semantic strategy unavailable: {})", e);
            return Vec::new();
        }
    };
    match refresh_index(&mut embedder, &mut index, &rows, false) {
        Ok(n) if n > 0 => {
            let _ = index.save(data_path);
            if !quiet {
                eprintln!("(refreshed index: embedded {} node(s) -> {})", n, EMBED_SIDECAR);
            }
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("(semantic strategy unavailable: {})", e);
            return Vec::new();
        }
    }

    let ids: Vec<u32> = {
        let mut v: Vec<u32> = index.vectors.keys().copied().collect();
        v.sort();
        v
    };
    let mut out = Vec::new();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let (a, b) = (ids[i], ids[j]);
            if let (Some(va), Some(vb)) = (index.vectors.get(&a), index.vectors.get(&b)) {
                let c = cosine(&va.vec, &vb.vec);
                if c >= threshold {
                    out.push((a, b, c));
                }
            }
        }
    }
    out
}

#[cfg(not(feature = "embeddings"))]
fn semantic_pair_scores(_: &Braim, _: &str, _: f32, _: bool) -> Vec<(u32, u32, f32)> {
    eprintln!("(the semantic strategy needs the embeddings feature; this binary was built with --no-default-features)");
    Vec::new()
}
