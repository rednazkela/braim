use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use chrono::Utc;

/// Cross-process exclusive write lock over one data dir.
///
/// Every braim mutation is a read-modify-write cycle spanning the whole process:
/// `Braim::new` loads the graph, the command mutates memory, `flush` rewrites the
/// files. Without a lock, concurrent writers clobber each other — measured, not
/// theorised: six simultaneous exports into one central lost two contributions
/// outright, and six simultaneous `version save` runs recorded four of six index
/// entries (braim ID:250). Writers hold this from BEFORE the load until the
/// process exits; readers never take it and rely on atomic renames instead, so
/// queries and the viewer stay non-blocking.
///
/// Built on `create_new` rather than an OS advisory lock to stay dependency-free
/// and behave identically on Linux, macOS, and Windows.
pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    /// A lock file older than this is assumed abandoned by a crashed process.
    const STALE_AFTER: Duration = Duration::from_secs(60);
    /// How long a writer waits for a peer before giving up with a clear error.
    const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn acquire(dir: &Path) -> Result<FileLock, String> {
        let path = dir.join(".braim.lock");
        let start = Instant::now();
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(FileLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map(|t| t.elapsed().map(|age| age > Self::STALE_AFTER).unwrap_or(false))
                        .unwrap_or(false);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > Self::WAIT_TIMEOUT {
                        return Err(format!(
                            "Error: timed out after {}s waiting for the write lock at {}. \
                             Another braim process is writing to this graph; if none is running, \
                             delete that file.",
                            Self::WAIT_TIMEOUT.as_secs(),
                            path.display()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(format!("Failed to acquire write lock: {}", e)),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Monotonic counter bumped after a sharded write completes. Atomic renames make
/// each shard file individually sound, but a sharded update touches MANY files,
/// so a lock-free reader can otherwise merge shard A's new state with shard B's
/// old one — observed in practice as dangling cross-domain references
/// (tests/concurrency.rs::readers_never_observe_an_inconsistent_shard_set).
/// Paired with the writer's lock file this forms a seqlock: see `load_sharded`.
fn seq_path(dir: &Path) -> PathBuf {
    dir.join(".braim.seq")
}

fn read_seq(dir: &Path) -> u64 {
    fs::read_to_string(seq_path(dir))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Bumped only AFTER every file of an update has landed, so a reader seeing an
/// unchanged sequence across its read knows no write completed inside it.
fn bump_seq(dir: &Path) -> Result<(), String> {
    let next = read_seq(dir).wrapping_add(1);
    write_atomic(&seq_path(dir), &next.to_string())
}

fn writer_active(dir: &Path) -> bool {
    dir.join(".braim.lock").exists()
}

/// Write a file atomically: fill a temp sibling, then rename over the target.
/// `fs::write` truncates before writing, so a concurrent reader can observe an
/// empty or half-written graph; rename is atomic on POSIX and replaces on
/// Windows, so readers only ever see a complete prior or new state.
fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, content)
        .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to replace {}: {}", path.display(), e)
    })
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Atomic,
    Compound,
    /// Legacy variant kept for permissive deserialization.
    /// Post-load migration converts these to Claim/Fact/InvalidStatement
    /// based on verification_status (per BRAIM_NODE_TYPE_CLAIM_FACT_SPEC §6).
    Statement,
    Claim,
    Fact,
    InvalidStatement,
    ContestedStatement,
    Source,
}

impl NodeType {
    /// True for statement-family nodes (legacy `Statement`, `Claim`, `Fact`,
    /// `InvalidStatement`, `ContestedStatement`). Concepts (`Atomic`, `Compound`) return false.
    pub fn is_statement_family(&self) -> bool {
        matches!(self,
            NodeType::Statement | NodeType::Claim | NodeType::Fact |
            NodeType::InvalidStatement | NodeType::ContestedStatement)
    }

    /// Derive node_type from verification_status per spec §3.2.
    pub fn from_verification_status(status: VerificationStatus) -> NodeType {
        match status {
            VerificationStatus::Invalid => NodeType::InvalidStatement,
            VerificationStatus::Unproven => NodeType::Claim,
            VerificationStatus::Contested => NodeType::ContestedStatement,
            VerificationStatus::Partial
            | VerificationStatus::Proven
            | VerificationStatus::ProvenStrong => NodeType::Fact,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Active,
    Pending,
    Deprecated,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Invalid,
    #[default]
    Unproven,
    Contested,
    Partial,
    Proven,
    ProvenStrong,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceType {
    Code,
    Doc,
    Schema,
    Config,
    Transcript,
    Test,
    PhaseN,
    Agent,
    Narrative,
    Logic,
    Inference,
}

impl SourceType {
    pub fn tier(&self) -> &'static str {
        match self {
            SourceType::Code | SourceType::Doc | SourceType::Schema |
            SourceType::Config | SourceType::Transcript | SourceType::Test => "PRIMARY",
            SourceType::PhaseN | SourceType::Agent | SourceType::Narrative => "SECONDARY",
            SourceType::Logic | SourceType::Inference => "TERTIARY",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "code" => Some(SourceType::Code),
            "doc" => Some(SourceType::Doc),
            "schema" => Some(SourceType::Schema),
            "config" => Some(SourceType::Config),
            "transcript" => Some(SourceType::Transcript),
            "test" => Some(SourceType::Test),
            s if s.starts_with("phase_") => Some(SourceType::PhaseN),
            "agent" => Some(SourceType::Agent),
            "narrative" => Some(SourceType::Narrative),
            "logic" => Some(SourceType::Logic),
            "inference" => Some(SourceType::Inference),
            _ => None,
        }
    }
}

impl VerificationStatus {
    pub fn badge(&self) -> &'static str {
        match self {
            VerificationStatus::ProvenStrong => "✓✓✓",
            VerificationStatus::Proven => "✓✓",
            VerificationStatus::Partial => "✓",
            VerificationStatus::Contested => "⚠",
            VerificationStatus::Unproven => "✗",
            VerificationStatus::Invalid => "✗✗",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            VerificationStatus::ProvenStrong => "proven_strong",
            VerificationStatus::Proven => "proven",
            VerificationStatus::Partial => "partial",
            VerificationStatus::Contested => "contested",
            VerificationStatus::Unproven => "unproven",
            VerificationStatus::Invalid => "invalid",
        }
    }

    /// Canonical rank for inheritance capping per
    /// BRAIM_DEPENDENCY_INHERITANCE_SPEC §3.1.
    pub fn rank(&self) -> u8 {
        match self {
            VerificationStatus::Invalid => 0,
            VerificationStatus::Unproven => 1,
            VerificationStatus::Contested => 2,
            VerificationStatus::Partial => 3,
            VerificationStatus::Proven => 4,
            VerificationStatus::ProvenStrong => 5,
        }
    }

    /// Inverse of `rank`: map a canonical rank back to a status.
    /// Single source of truth for rank -> status so the two never drift.
    pub fn from_rank(rank: u8) -> VerificationStatus {
        match rank {
            0 => VerificationStatus::Invalid,
            1 => VerificationStatus::Unproven,
            2 => VerificationStatus::Contested,
            3 => VerificationStatus::Partial,
            4 => VerificationStatus::Proven,
            _ => VerificationStatus::ProvenStrong,
        }
    }
}

/// Typed errors for node construction so callers can match on the failure
/// kind instead of parsing strings. `Display` reproduces the original
/// messages verbatim, so callers that surface the text are unaffected.
/// (Domain/source arity is intentionally not represented — it is no longer
/// validated since domains were decoupled from arity.)
#[derive(Debug, Clone, PartialEq)]
pub enum GraphError {
    EmptyDomainsSources,
    StatementNoDependency,
    DependencyNotFound(u32),
    WeightsNotOne(f64),
    CompoundNoDependencies,
    ConceptExists { term: String, domains: Vec<String>, id: u32 },
    /// Errors raised outside the typed validation layer (source-prefix
    /// validation, persistence, concept-graph checks). Carries the original
    /// message so nothing is lost during the migration off `String` errors.
    Other(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::EmptyDomainsSources => {
                write!(f, "Error: domains and sources must not be empty")
            }
            GraphError::StatementNoDependency => {
                write!(f, "Error: Statement must have at least 1 dependency")
            }
            GraphError::DependencyNotFound(id) => {
                write!(f, "Error: Dependency ID {} does not exist", id)
            }
            GraphError::WeightsNotOne(sum) => {
                write!(f, "Error: Weights must sum to 1.0 — got {:.4}", sum)
            }
            GraphError::CompoundNoDependencies => {
                write!(f, "Error: Compound concept must have dependencies")
            }
            GraphError::ConceptExists { term, domains, id } => write!(
                f,
                "Error: Concept '{}' already exists in domain {:?} (ID {})",
                term, domains, id
            ),
            GraphError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for GraphError {}

impl From<String> for GraphError {
    fn from(s: String) -> Self {
        GraphError::Other(s)
    }
}

impl From<&str> for GraphError {
    fn from(s: &str) -> Self {
        GraphError::Other(s.to_string())
    }
}

impl From<GraphError> for String {
    fn from(e: GraphError) -> Self {
        e.to_string()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Node {
    pub id: u32,
    /// Defaulted: pre-domains graphs (May 2026 era) lack the field entirely.
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    pub node_type: NodeType,
    pub label: String,
    pub depends_on: HashMap<u32, f64>,
    pub status: NodeStatus,
    pub created_at: String,
    #[serde(default)]
    pub verified_by: HashMap<String, Option<String>>,
    #[serde(default)]
    pub verification_status: VerificationStatus,
    #[serde(default)]
    pub invalid: bool,
    #[serde(default)]
    pub invalid_reason: Option<String>,
    #[serde(default)]
    pub invalidated_at: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub ingested_by: Option<String>,
    #[serde(default)]
    pub source_ids: Vec<u32>,
    #[serde(default)]
    pub pre_contested_status: Option<VerificationStatus>,
    /// First-class structured metadata (queryable, incrementable). Used by the
    /// open-SIC register so scope, recurrence_count, affected_feature, status,
    /// action_deadline are real fields — not label-string or domain encoded
    /// (braim 6336). `#[serde(default)]` keeps existing current.json loadable.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GapRecord {
    pub concept_a: u32,
    pub concept_b: u32,
    pub label_a: String,
    pub label_b: String,
    pub status: String,
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContradictEdge {
    pub from: u32,
    pub to: u32,
    pub reason: String,
    pub source_id: Option<u32>,
    pub created_at: String,
    pub resolved: bool,
    pub resolution_source: Option<u32>,
    pub resolution_winner: Option<u32>,
}

/// Directional causal edge: `from occurs because_of to` (consequent → cause).
/// Supports the Five Whys methodology (braim-because-of-edge.md). Unlike
/// `depends_on` (compositional, weighted) it is unweighted — each link asserts
/// a single principal cause. Unlike `contradicts` it is directional. Kept as a
/// separate GraphState collection so it never pollutes `depends_on` traversal
/// (perspective / proximity stay compositional-only).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BecauseOfEdge {
    /// Consequent statement: the effect / symptom.
    pub from: u32,
    /// Cause statement: why `from` occurs.
    pub to: u32,
    /// Optional typed source string justifying the causal hypothesis.
    pub source: Option<String>,
    pub created_at: String,
    /// `test:`-typed source recorded by a passing inverse test (`why-test`).
    /// Presence upgrades a both-endpoints-proven edge from partial → proven.
    #[serde(default)]
    pub test_source: Option<String>,
    /// Set when an inverse test failed: the causal link is refuted while the
    /// endpoint statements themselves stay valid.
    #[serde(default)]
    pub invalid: bool,
    #[serde(default)]
    pub invalid_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphState {
    pub nodes: HashMap<u32, Node>,
    pub dictionary: HashMap<String, Vec<u32>>,
    pub id_to_domain: HashMap<u32, String>,
    pub gaps: Vec<GapRecord>,
    pub next_id: u32,
    pub version: u32,
    #[serde(default)]
    pub contradicts: Vec<ContradictEdge>,
    #[serde(default)]
    pub because_of: Vec<BecauseOfEdge>,
}

/// One checkpoint in the sharded layout's versions.json index. Instead of a
/// whole-graph clone, it records WHICH per-domain snapshot each domain was at —
/// the domain snapshot file (domains/<name>-<hash>.v<NNNN>.json) is the pin
/// artifact the mount manifest's pinned_version references (braim ID:214/242).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShardedVersionEntry {
    pub version: u32,
    pub description: String,
    pub saved_at: String,
    pub node_count: usize,
    /// domain → that domain's snapshot version at this checkpoint
    pub domain_versions: HashMap<String, u32>,
    /// version of the cross-domain header snapshot (graph.v<NNNN>.json)
    pub header_version: u32,
}

/// Layout-agnostic version summary for listings.
pub struct VersionInfo {
    pub version: u32,
    pub description: String,
    pub saved_at: String,
    pub node_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionMeta {
    pub description: String,
    pub saved_at: String,
    pub data: GraphState,
}

#[derive(Clone, Debug)]
pub struct PathInfo {
    pub path: Vec<u32>,
    pub weight: f64,
    #[allow(dead_code)]
    pub domains: Vec<String>,
}

/// A `because_of` edge surfaced by audit, with both endpoint labels resolved
/// for display. Used for refuted-link and untested-link findings.
#[derive(Clone, Debug)]
pub struct CausalEdgeInfo {
    pub from: u32,
    pub from_label: String,
    pub to: u32,
    pub to_label: String,
    /// invalid_reason for a refuted link; None for an untested link.
    pub reason: Option<String>,
}

pub struct AuditReport {
    pub orphans: Vec<Node>,
    pub pending: Vec<Node>,
    pub gaps: Vec<GapRecord>,
    pub deprecated_referenced: Vec<Node>,
    /// because_of edges refuted by a failing inverse test (`why-test --fail`):
    /// unfinished investigations left in the graph.
    pub refuted_causal_links: Vec<CausalEdgeInfo>,
    /// Statements carrying the `because_of_reinvestigate` metadata flag because
    /// a cause below them was invalidated — they need a fresh look.
    pub reinvestigate_flagged: Vec<Node>,
    /// Active because_of edges with no inverse-test source: unvalidated causal
    /// hypotheses (Five Whys discipline says links should be inverse-tested).
    pub untested_causal_links: Vec<CausalEdgeInfo>,
    /// Terminal root-cause statements of a because_of chain whose verification
    /// is below `proven`: chains that bottom out without solid evidence.
    pub unverified_roots: Vec<Node>,
}

/// A single candidate source returned by `verify_suggest`.
/// Per BRAIM_VERIFY_SUGGEST_SPEC §3.2 each candidate carries the concrete
/// `type:location` source string, a one-line rationale explaining why it
/// was selected, the promotion-impact prediction (status label the target
/// would reach if this source is added), and a numeric rank used for sorting.
#[derive(Clone, Debug)]
pub struct SuggestedSource {
    pub source: String,
    pub rationale: String,
    pub impact: String,
    pub rank: u8,
}

/// Full result of `verify_suggest`. `message` short-circuits everything else
/// (used for proven_strong / invalid / no-candidates outcomes).
#[derive(Clone, Debug)]
pub struct VerifySuggestion {
    pub statement_id: u32,
    pub label: String,
    pub status_label: String,
    pub primary_count: usize,
    pub distinct_primary_types: usize,
    pub message: Option<String>,
    pub candidates: Vec<SuggestedSource>,
    pub already_attached_types: Vec<String>,
    pub missing_primary_types: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DuplicateRecord {
    pub source_id: u32,
    pub target_id: u32,
    pub target_label: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct AddSourceResult {
    pub auto_resolved: bool,
    pub winner_id: Option<u32>,
    pub loser_id: Option<u32>,
    pub winner_status: Option<VerificationStatus>,
}

/// What a merge_nodes call did, so the caller can report it honestly.
pub struct MergeOutcome {
    pub winner: u32,
    pub loser: u32,
    /// Source strings + source entities the winner gained from the loser.
    pub sources_added: usize,
    pub referents_rewired: usize,
    pub edges_rewired: usize,
    pub new_status: VerificationStatus,
    /// Dependencies the loser had and the winner does not. NOT merged — that
    /// would rewrite what the surviving statement asserts — so they surface here
    /// for a human to decide about.
    pub dep_differences: Vec<u32>,
}

pub struct ImportManifest {
    /// Full-fidelity mode only: source entities imported.
    pub sources_imported: usize,
    /// Full-fidelity mode only: because_of edges carried over.
    pub because_of_imported: usize,
    /// Full-fidelity mode only: contradicts edges carried over.
    pub contradicts_imported: usize,
    /// Full-fidelity mode only: dedup hits whose sources were unioned into the target.
    pub sources_unioned: usize,
    pub imported_count: usize,
    pub deduplicated_count: usize,
    pub skipped_count: usize,
    pub id_mappings: HashMap<u32, u32>,
    pub duplicates: Vec<DuplicateRecord>,
}

pub struct Braim {
    pub data_dir: PathBuf,
    pub state: GraphState,
    /// Number of nodes that had a legacy `statement` node_type rewritten
    /// in-memory during the most recent load. Drives the migration command
    /// summary so users know whether the on-disk file was already canonical.
    pub legacy_node_types_migrated: usize,
    /// Reverse-adjacency index built at load: dependents[X] = list of
    /// (node_id, edge_weight) for every ACTIVE node whose depends_on contains X.
    /// Replaces the O(n) full-scan-per-step in propagate() with O(1) lookup,
    /// which (with the visited-set) bounds traversal to O(V+E) and fixes the
    /// high-fan-out query hang.
    pub dependents: HashMap<u32, Vec<(u32, f64)>>,
    /// Held for the process lifetime by instances opened via `open_for_write`,
    /// serialising this graph's read-modify-write cycle against other processes.
    /// `None` on read-only instances, which never block and never write.
    write_lock: Option<FileLock>,
}

/// Canonical list of PRIMARY-tier source type prefix names.
/// Used by verify-suggest to enumerate "missing types that would promote"
/// and to keep candidate ranking consistent with `SourceType::tier()`.
const ALL_PRIMARY_TYPES: &[&str] = &[
    "code", "doc", "schema", "config", "transcript", "test",
];

fn primary_type_name(t: &SourceType) -> &'static str {
    match t {
        SourceType::Code => "code",
        SourceType::Doc => "doc",
        SourceType::Schema => "schema",
        SourceType::Config => "config",
        SourceType::Transcript => "transcript",
        SourceType::Test => "test",
        _ => "",
    }
}

fn predicted_status_label(distinct_primary_count: usize) -> &'static str {
    match distinct_primary_count {
        0 => "unproven",
        1 => "partial",
        2 => "proven",
        _ => "proven_strong",
    }
}

/// Tokenize a label into lowercase alphanumeric terms of length ≥3.
/// Used by verify-suggest to measure label similarity across statements.
fn extract_label_terms(label: &str) -> HashSet<String> {
    label.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '/')
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| t.len() >= 3)
        .collect()
}

fn shared_term_count(a: &HashSet<String>, b: &HashSet<String>) -> usize {
    a.intersection(b).count()
}

/// Extract file-path-like substrings from a label and classify them by
/// extension. Returns (full_token_path, primary_type_prefix).
/// `full_token_path` preserves any trailing `:line[-range]` location suffix
/// so the resulting source string can be attached verbatim.
fn extract_label_paths(label: &str) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for raw in label.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '(' || c == ')') {
        let token = raw.trim_matches(|c: char|
            !c.is_alphanumeric() && c != '/' && c != '.' && c != ':' && c != '-' && c != '_');
        if token.is_empty() {
            continue;
        }
        // Strip any trailing :line[-range] suffix to inspect the extension on the path itself.
        let path_only = token.split(':').next().unwrap_or(token);
        let dot_idx = match path_only.rfind('.') {
            Some(idx) => idx,
            None => continue,
        };
        let ext = path_only[dot_idx + 1..].to_lowercase();
        let type_prefix = match ext.as_str() {
            "js" | "ts" | "tsx" | "jsx" | "py" | "rs" | "go" | "java"
            | "kt" | "rb" | "php" | "c" | "cpp" | "h" | "hpp" | "cs"
            | "swift" | "scala" | "ex" | "exs" | "el" | "lua" | "sh" => Some("code"),
            "sql" => Some("schema"),
            "md" | "mdx" | "rst" | "txt" | "adoc" => Some("doc"),
            "yml" | "yaml" | "json" | "toml" | "ini" | "conf" | "env" => Some("config"),
            _ => None,
        };
        if let Some(t) = type_prefix {
            out.push((token.to_string(), t));
        }
    }
    out
}

/// One node in a `why` causal walk, plus the status of the edge leaving it
/// toward the next node (None for the root cause).
#[derive(Clone, Debug)]
pub struct WhyStep {
    pub id: u32,
    pub label: String,
    pub verification_status: VerificationStatus,
    /// Inherited verification status of the outgoing `because_of` edge (the
    /// causal claim). None for the terminal root cause (no outgoing edge).
    pub causal_status: Option<VerificationStatus>,
    /// True when the outgoing edge carries a passing inverse-test source.
    pub edge_tested: bool,
    /// True when the outgoing edge was refuted by a failing inverse test.
    pub edge_invalid: bool,
    /// IDs this node is currently contested with (unresolved contradicts edge).
    pub contested_with: Vec<u32>,
}

/// Result of walking a `because_of` chain from a consequent to its root cause.
#[derive(Clone, Debug)]
pub struct WhyChain {
    pub steps: Vec<WhyStep>,
    pub root_id: u32,
    /// True when the terminal root cause has a source-derived proven status.
    pub root_verified: bool,
}

/// Outcome of `why_test`.
#[derive(Clone, Debug)]
pub struct WhyTestOutcome {
    pub consequent: u32,
    pub cause: u32,
    pub passed: bool,
    /// Resulting causal-claim status after the test (for a pass).
    pub causal_status: VerificationStatus,
}

/// Outcome of `why_remove`.
#[derive(Clone, Debug)]
pub struct WhyRemoveOutcome {
    pub consequent: u32,
    /// Cause the removed edge pointed at.
    pub cause: u32,
    /// True when the removed edge had been refuted by a failing inverse test.
    pub was_invalid: bool,
}

/// Soft-warn threshold (inclusive): resulting chain depth ≥ this emits a
/// stderr warning. Hard-reject threshold (exclusive upper bound): resulting
/// depth > MAX rejects. Per braim-because-of-edge.md Open Questions.
const WHY_DEPTH_WARN: usize = 7;
const WHY_DEPTH_MAX: usize = 10;

impl Braim {
    pub fn parse_source(source: &str) -> (SourceType, String) {
        if let Some(colon_idx) = source.find(':') {
            let prefix = &source[..colon_idx];
            let location = &source[colon_idx + 1..];
            if let Some(source_type) = SourceType::from_prefix(prefix) {
                return (source_type, location.to_string());
            }
        }
        (SourceType::Narrative, source.to_string())
    }

    pub fn validate_source_prefix(source: &str) -> Result<(), String> {
        if !source.contains(':') {
            return Err(format!(
                "Error: source '{}' missing required type prefix (code:|doc:|schema:|config:|transcript:|test:|phase_N:|agent:|narrative:|logic:|inference:)",
                source
            ));
        }
        Ok(())
    }

    pub fn calculate_verification_status_from_sources(sources: &[String]) -> VerificationStatus {
        let mut primary_types = std::collections::HashSet::new();

        for source in sources {
            let (source_type, _location) = Self::parse_source(source);
            if source_type.tier() == "PRIMARY" {
                primary_types.insert(source_type);
            }
        }

        let primary_count = primary_types.len();
        match primary_count {
            0 => VerificationStatus::Unproven,
            1 => VerificationStatus::Partial,
            2 => VerificationStatus::Proven,
            _ => VerificationStatus::ProvenStrong,
        }
    }

    pub fn validate_duplicate_sources(sources: &[String]) -> (bool, Vec<String>) {
        let mut seen = std::collections::HashMap::new();
        let mut duplicates = Vec::new();

        for source in sources {
            *seen.entry(source.clone()).or_insert(0) += 1;
        }

        for (source, count) in seen {
            if count > 1 {
                duplicates.push(source);
            }
        }

        (duplicates.len() > 0, duplicates)
    }

    pub fn validate_primary_tertiary_mix(sources: &[String]) -> bool {
        let mut has_primary = false;
        let mut has_tertiary = false;

        for source in sources {
            let (source_type, _location) = Self::parse_source(source);
            match source_type.tier() {
                "PRIMARY" => has_primary = true,
                "TERTIARY" => has_tertiary = true,
                _ => {}
            }
        }

        has_primary && has_tertiary
    }

    pub fn validate_duplicate_domains(domains: &[String]) -> (bool, std::collections::HashMap<String, usize>) {
        let mut counts = std::collections::HashMap::new();

        for domain in domains {
            *counts.entry(domain.clone()).or_insert(0) += 1;
        }

        let has_dups = counts.values().any(|&count| count > 1);
        (has_dups, counts)
    }

    /// Returns the number of distinct domain values across all non-source active nodes.
    pub fn distinct_domain_count(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for node in self.state.nodes.values() {
            if node.node_type != NodeType::Source {
                for d in &node.domains {
                    seen.insert(d.clone());
                }
            }
        }
        seen.len()
    }

    /// Returns true if the label ends with a line-number suffix (e.g. `:104` or `:104-127`).
    pub fn label_has_line_number_suffix(label: &str) -> bool {
        if let Some(colon_pos) = label.rfind(':') {
            let after = &label[colon_pos + 1..];
            if after.is_empty() {
                return false;
            }
            if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
            if let Some(dash_pos) = after.find('-') {
                let before = &after[..dash_pos];
                let tail = &after[dash_pos + 1..];
                if !before.is_empty()
                    && before.chars().all(|c| c.is_ascii_digit())
                    && !tail.is_empty()
                    && tail.chars().all(|c| c.is_ascii_digit())
                {
                    return true;
                }
            }
        }
        false
    }

    /// Add, remove, or replace dependency edges on an existing compound, preserving node ID.
    pub fn update_deps(
        &mut self,
        node_id: u32,
        add: Option<HashMap<u32, f64>>,
        remove: Option<Vec<u32>>,
        set: Option<HashMap<u32, f64>>,
    ) -> Result<HashMap<u32, f64>, String> {
        {
            let node = self.state.nodes.get(&node_id)
                .ok_or(format!("Error: Node ID {} does not exist", node_id))?;
            if node.node_type != NodeType::Compound {
                return Err(format!(
                    "Error: Node ID {} is not a compound (node_type: {:?})",
                    node_id, node.node_type
                ));
            }
        }
        self.update_deps_core(node_id, add, remove, set)
    }

    /// Statement counterpart of update_deps: preserves the statement ID and its
    /// attached sources instead of forcing delete+recreate. Recomputes inherited
    /// verification and clears gaps covered by the new dependency pairs.
    pub fn update_statement_deps(
        &mut self,
        node_id: u32,
        add: Option<HashMap<u32, f64>>,
        remove: Option<Vec<u32>>,
        set: Option<HashMap<u32, f64>>,
    ) -> Result<HashMap<u32, f64>, String> {
        {
            let node = self.state.nodes.get(&node_id)
                .ok_or(format!("Error: Node ID {} does not exist", node_id))?;
            if !node.node_type.is_statement_family() {
                return Err(format!(
                    "Error: Node ID {} is not a statement (node_type: {:?})",
                    node_id, node.node_type
                ));
            }
            if node.invalid || node.verification_status == VerificationStatus::Invalid {
                return Err(format!("Error: Cannot update deps of invalid statement ID {}", node_id));
            }
        }
        for ids in [add.as_ref().map(|m| m.keys().copied().collect::<Vec<_>>()),
                    set.as_ref().map(|m| m.keys().copied().collect::<Vec<_>>())]
            .into_iter()
            .flatten()
        {
            for dep_id in ids {
                if let Some(dep) = self.state.nodes.get(&dep_id) {
                    if dep.invalid || dep.verification_status == VerificationStatus::Invalid {
                        return Err(format!(
                            "Error: Dependency ID {} is invalid — cannot wire a statement to a refuted node",
                            dep_id
                        ));
                    }
                }
            }
        }
        let new_deps = self.update_deps_core(node_id, add, remove, set)?;

        // Dependency inheritance changed — recompute status unless contested
        // (contested resolves only through the contradiction lifecycle).
        if self.state.nodes[&node_id].verification_status != VerificationStatus::Contested {
            let new_status = {
                let stmt = self.state.nodes.get(&node_id).unwrap();
                let entity_types = self.fetch_source_entity_types(&stmt.source_ids);
                let source_derived =
                    Self::calculate_verification_status_from_all_sources(&stmt.sources, &entity_types);
                let mut cap: Option<u8> = None;
                for dep_id in new_deps.keys() {
                    if let Some(dep) = self.state.nodes.get(dep_id) {
                        if !dep.node_type.is_statement_family() {
                            continue;
                        }
                        let r = dep.verification_status.rank();
                        cap = Some(cap.map_or(r, |p: u8| p.min(r)));
                    }
                }
                match cap {
                    Some(c) if source_derived.rank() > c => VerificationStatus::from_rank(c),
                    _ => source_derived,
                }
            };
            let stmt = self.state.nodes.get_mut(&node_id).unwrap();
            stmt.verification_status = new_status;
            stmt.node_type = NodeType::from_verification_status(new_status);
        }

        let dep_ids: Vec<u32> = new_deps.keys().copied().collect();
        self.clear_gaps_for_deps(&dep_ids);
        self.flush()?;
        Ok(new_deps)
    }

    fn update_deps_core(
        &mut self,
        node_id: u32,
        add: Option<HashMap<u32, f64>>,
        remove: Option<Vec<u32>>,
        set: Option<HashMap<u32, f64>>,
    ) -> Result<HashMap<u32, f64>, String> {

        let new_deps: HashMap<u32, f64> = if let Some(set_deps) = set {
            for &dep_id in set_deps.keys() {
                if !self.state.nodes.contains_key(&dep_id) {
                    return Err(format!("Error: Dependency ID {} does not exist", dep_id));
                }
            }
            set_deps
        } else {
            let mut deps = self.state.nodes.get(&node_id).unwrap().depends_on.clone();
            if let Some(ids_to_remove) = remove {
                for id in ids_to_remove {
                    if !deps.contains_key(&id) {
                        return Err(format!(
                            "Error: ID {} is not a current dependency of node {}",
                            id, node_id
                        ));
                    }
                    deps.remove(&id);
                }
            }
            if let Some(new_entries) = add {
                for (id, weight) in new_entries {
                    if deps.contains_key(&id) {
                        return Err(format!(
                            "Error: ID {} is already a dependency of node {}. Use --remove first or --set to replace all.",
                            id, node_id
                        ));
                    }
                    if !self.state.nodes.contains_key(&id) {
                        return Err(format!("Error: Dependency ID {} does not exist", id));
                    }
                    deps.insert(id, weight);
                }
            }
            deps
        };

        if new_deps.is_empty() {
            return Err("Error: Node must retain at least 1 dependency".to_string());
        }
        let sum: f64 = new_deps.values().sum();
        if (sum - 1.0).abs() > 0.001 {
            return Err(format!("Error: Weights must sum to 1.0 — got {:.4}", sum));
        }

        self.state.nodes.get_mut(&node_id).unwrap().depends_on = new_deps.clone();
        self.flush()?;
        Ok(new_deps)
    }

    pub fn new(data_dir: &str) -> Result<Self, String> {
        Self::load_from(data_dir, false)
    }

    fn load_from(data_dir: &str, holds_lock: bool) -> Result<Self, String> {
        let path = PathBuf::from(data_dir);
        fs::create_dir_all(&path).map_err(|e| format!("Failed to create data dir: {}", e))?;

        let current_path = path.join("current.json");
        let mut state: GraphState = if path.join("domains").is_dir() {
            Self::load_sharded(&path, holds_lock)?
        } else if current_path.exists() {
            let content = fs::read_to_string(&current_path)
                .map_err(|e| format!("Failed to read current.json: {}", e))?;
            serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse current.json: {}", e))?
        } else {
            GraphState {
                nodes: HashMap::new(),
                dictionary: HashMap::new(),
                id_to_domain: HashMap::new(),
                gaps: Vec::new(),
                next_id: 1,
                version: 0,
                contradicts: Vec::new(),
                because_of: Vec::new(),
            }
        };

        // Permissive read-time migration per BRAIM_NODE_TYPE_CLAIM_FACT_SPEC §6:
        // rewrite legacy `statement` node_type in-memory to claim/fact/invalid_statement
        // based on verification_status. Persistence happens on next flush() or
        // explicitly via `migrate_node_types()`.
        let mut legacy_count = 0usize;
        for node in state.nodes.values_mut() {
            if matches!(node.node_type, NodeType::Statement) {
                node.node_type = NodeType::from_verification_status(node.verification_status);
                legacy_count += 1;
            }
        }

        // Ensure atomics are indexed by short name (part before ': ') for query-by-name support.
        // Idempotent: skips IDs already present under the short key.
        let atomic_shorts: Vec<(String, u32)> = state.nodes.iter()
            .filter(|(_, n)| n.node_type == NodeType::Atomic)
            .filter_map(|(&id, n)| {
                let short = Self::atomic_short_name_key(&n.label)?;
                if short != n.label.to_lowercase() { Some((short, id)) } else { None }
            })
            .collect();
        for (short, id) in atomic_shorts {
            let entry = state.dictionary.entry(short).or_insert_with(Vec::new);
            if !entry.contains(&id) {
                entry.push(id);
            }
        }

        // Build the reverse-adjacency index once at load (see field doc).
        let dependents = Self::build_dependents(&state);

        Ok(Braim {
            data_dir: path,
            state,
            legacy_node_types_migrated: legacy_count,
            dependents,
            write_lock: None,
        })
    }

    /// Open for mutation: take the cross-process write lock BEFORE loading, and
    /// hold it until this instance is dropped. Acquiring first is the whole
    /// point — a lock taken after the load would leave the read half of the
    /// read-modify-write cycle unprotected and still lose updates (braim ID:250).
    pub fn open_for_write(data_dir: &str) -> Result<Self, String> {
        let path = PathBuf::from(data_dir);
        fs::create_dir_all(&path).map_err(|e| format!("Failed to create data dir: {}", e))?;
        let lock = FileLock::acquire(&path)?;
        let mut braim = Self::load_from(data_dir, true)?;
        braim.write_lock = Some(lock);
        Ok(braim)
    }

    /// dependents[X] = (node_id, weight) for every ACTIVE node whose
    /// depends_on contains X. Mirrors the active-node filter the old
    /// propagate() applied to the expanding node.
    fn build_dependents(state: &GraphState) -> HashMap<u32, Vec<(u32, f64)>> {
        let mut dependents: HashMap<u32, Vec<(u32, f64)>> = HashMap::new();
        for node in state.nodes.values() {
            if node.status != NodeStatus::Active {
                continue;
            }
            for (&dep_id, &w) in &node.depends_on {
                dependents.entry(dep_id).or_default().push((node.id, w));
            }
        }
        dependents
    }

    /// Force-rewrite all node_type fields from verification_status and flush
    /// the result to disk. Returns the total number of nodes that were
    /// migrated, including any that were already converted in-memory by the
    /// post-load step in `Braim::new`. Idempotent.
    pub fn migrate_node_types(&mut self) -> Result<usize, String> {
        let mut changed = self.legacy_node_types_migrated;
        for node in self.state.nodes.values_mut() {
            if !node.node_type.is_statement_family() {
                continue;
            }
            let expected = NodeType::from_verification_status(node.verification_status);
            if node.node_type != expected {
                node.node_type = expected;
                changed += 1;
            }
        }
        // Always flush — the post-load step may already have updated the
        // in-memory state but not the on-disk file.
        self.flush()?;
        self.legacy_node_types_migrated = 0;
        Ok(changed)
    }

    pub fn add_source(
        &mut self,
        label: &str,
        source_type_str: &str,
        location: Option<String>,
        ingested_by: Option<String>,
    ) -> Result<u32, String> {
        // Validate source_type_str is a known prefix
        if SourceType::from_prefix(source_type_str).is_none() {
            return Err(format!(
                "Error: unknown source type '{}'. Use: code, doc, schema, config, transcript, test, phase_N, agent, narrative, logic, inference",
                source_type_str
            ));
        }
        let id = self.state.next_id;
        self.state.next_id += 1;
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let node = Node {
            id,
            domains: vec![],
            sources: vec![],
            node_type: NodeType::Source,
            label: label.to_string(),
            depends_on: HashMap::new(),
            status: NodeStatus::Active,
            created_at: now.clone(),
            verified_by: HashMap::new(),
            verification_status: VerificationStatus::Unproven,
            invalid: false,
            invalid_reason: None,
            invalidated_at: None,
            source_type: Some(source_type_str.to_string()),
            location,
            ingested_by,
            source_ids: vec![],
            pre_contested_status: None,
            metadata: HashMap::new(),
        };
        self.state.nodes.insert(id, node);
        let lower = label.to_lowercase();
        self.state.dictionary.entry(lower).or_insert_with(Vec::new).push(id);
        self.flush()?;
        Ok(id)
    }

    /// Compute verification status from both string sources and source entity IDs.
    /// Source entity IDs are looked up to get their source_type for PRIMARY classification.
    /// Pass pre-fetched entity types as &[String] to avoid borrow conflicts.
    pub fn calculate_verification_status_from_all_sources(
        sources: &[String],
        source_entity_types: &[String],
    ) -> VerificationStatus {
        let mut primary_types = std::collections::HashSet::new();
        for source in sources {
            let (source_type, _) = Self::parse_source(source);
            if source_type.tier() == "PRIMARY" {
                primary_types.insert(source_type);
            }
        }
        for st in source_entity_types {
            if let Some(source_type) = SourceType::from_prefix(st) {
                if source_type.tier() == "PRIMARY" {
                    primary_types.insert(source_type);
                }
            }
        }
        match primary_types.len() {
            0 => VerificationStatus::Unproven,
            1 => VerificationStatus::Partial,
            2 => VerificationStatus::Proven,
            _ => VerificationStatus::ProvenStrong,
        }
    }

    /// Collect the source_type strings for all source entity IDs on a node.
    fn fetch_source_entity_types(&self, source_ids: &[u32]) -> Vec<String> {
        source_ids.iter()
            .filter_map(|&sid| {
                self.state.nodes.get(&sid)
                    .filter(|n| n.node_type == NodeType::Source)
                    .and_then(|n| n.source_type.clone())
            })
            .collect()
    }

    pub fn add_source_to_statement(
        &mut self,
        statement_id: u32,
        source_id: u32,
    ) -> Result<AddSourceResult, String> {
        // Validate statement
        {
            let stmt = self.state.nodes.get(&statement_id)
                .ok_or(format!("Error: Statement ID {} not found", statement_id))?;
            if !stmt.node_type.is_statement_family() {
                return Err(format!("Error: Node ID {} is not a statement", statement_id));
            }
            if stmt.verification_status == VerificationStatus::Invalid {
                return Err(format!("Error: Cannot add source to invalid statement ID {}", statement_id));
            }
            if stmt.source_ids.contains(&source_id) {
                return Err(format!("Error: Source ID {} already attached to statement ID {}", source_id, statement_id));
            }
        }
        // Validate source entity
        let source_type_str = {
            let src = self.state.nodes.get(&source_id)
                .ok_or(format!("Error: Source ID {} not found", source_id))?;
            if src.node_type != NodeType::Source {
                return Err(format!(
                    "Error: Node ID {} is not a source entity (use 'braim source add' to create one)",
                    source_id
                ));
            }
            src.source_type.clone()
        };
        let source_is_primary = source_type_str
            .as_deref()
            .and_then(SourceType::from_prefix)
            .map(|t| t.tier() == "PRIMARY")
            .unwrap_or(false);

        // Attach the source
        {
            let stmt = self.state.nodes.get_mut(&statement_id).unwrap();
            stmt.source_ids.push(source_id);
        }

        // The statement's dependency pairs are connected regardless of which
        // branch below fires; gaps registered after the statement was created
        // would otherwise survive promotion (only `statement add` cleared them).
        let dep_ids: Vec<u32> = self.state.nodes[&statement_id].depends_on.keys().copied().collect();
        self.clear_gaps_for_deps(&dep_ids);

        let is_contested = self.state.nodes[&statement_id].verification_status
            == VerificationStatus::Contested;

        // Check for Mechanism A auto-resolution
        if is_contested && source_is_primary {
            let edge_idx = self.state.contradicts.iter().position(|e| {
                !e.resolved && (e.from == statement_id || e.to == statement_id)
            });
            if let Some(idx) = edge_idx {
                let other_id = {
                    let e = &self.state.contradicts[idx];
                    if e.from == statement_id { e.to } else { e.from }
                };
                let other_has_source = self.state.nodes.get(&other_id)
                    .map(|n| n.source_ids.contains(&source_id))
                    .unwrap_or(false);

                if !other_has_source {
                    let winner_status = {
                        let stmt = self.state.nodes.get(&statement_id).unwrap();
                        let entity_types = self.fetch_source_entity_types(&stmt.source_ids);
                        Self::calculate_verification_status_from_all_sources(&stmt.sources, &entity_types)
                    };
                    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    {
                        let winner = self.state.nodes.get_mut(&statement_id).unwrap();
                        winner.verification_status = winner_status;
                        winner.node_type = NodeType::from_verification_status(winner_status);
                        winner.pre_contested_status = None;
                    }
                    {
                        let loser = self.state.nodes.get_mut(&other_id).unwrap();
                        loser.verification_status = VerificationStatus::Invalid;
                        loser.node_type = NodeType::InvalidStatement;
                        loser.invalid = true;
                        loser.invalid_reason = Some(format!(
                            "contested_resolved_against_by_source_{}", source_id
                        ));
                        loser.invalidated_at = Some(now.clone());
                        loser.pre_contested_status = None;
                    }
                    {
                        let edge = &mut self.state.contradicts[idx];
                        edge.resolved = true;
                        edge.resolution_winner = Some(statement_id);
                        edge.resolution_source = Some(source_id);
                    }
                    let cascade_ids: Vec<u32> = self.find_cascade_nodes(other_id)
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect();
                    for dep_id in cascade_ids {
                        if let Some(dep) = self.state.nodes.get_mut(&dep_id) {
                            if dep.invalid || dep.verification_status == VerificationStatus::Invalid {
                                continue;
                            }
                            dep.invalid = true;
                            dep.invalid_reason = Some(format!("depends_on_invalidated:{}", other_id));
                            dep.invalidated_at = Some(now.clone());
                            dep.verification_status = VerificationStatus::Invalid;
                            dep.node_type = NodeType::InvalidStatement;
                        }
                    }
                    self.flush()?;
                    return Ok(AddSourceResult {
                        auto_resolved: true,
                        winner_id: Some(statement_id),
                        loser_id: Some(other_id),
                        winner_status: Some(winner_status),
                    });
                }
            }
        }

        // No auto-resolution: recompute status if not contested
        if !is_contested {
            let new_status = {
                let stmt = self.state.nodes.get(&statement_id).unwrap();
                let entity_types = self.fetch_source_entity_types(&stmt.source_ids);
                Self::calculate_verification_status_from_all_sources(&stmt.sources, &entity_types)
            };
            let stmt = self.state.nodes.get_mut(&statement_id).unwrap();
            stmt.verification_status = new_status;
            stmt.node_type = NodeType::from_verification_status(new_status);
        }

        self.flush()?;
        Ok(AddSourceResult {
            auto_resolved: false,
            winner_id: None,
            loser_id: None,
            winner_status: None,
        })
    }

    pub fn contradict_statements(
        &mut self,
        from: u32,
        to: u32,
        reason: &str,
        source_id: Option<u32>,
    ) -> Result<(), String> {
        // Validate both IDs exist and are statements
        for &id in &[from, to] {
            let node = self.state.nodes.get(&id)
                .ok_or(format!("Error: Statement ID {} not found", id))?;
            if !node.node_type.is_statement_family() {
                return Err(format!("Error: Node ID {} is not a statement", id));
            }
            if node.verification_status == VerificationStatus::Invalid {
                return Err(format!("Error: Statement ID {} is invalid and cannot be contested", id));
            }
        }
        // Check no existing unresolved contradicts edge between them
        for edge in &self.state.contradicts {
            if !edge.resolved
                && ((edge.from == from && edge.to == to) || (edge.from == to && edge.to == from))
            {
                return Err(format!(
                    "Error: unresolved contradiction already exists between {} and {}",
                    from, to
                ));
            }
        }
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.state.contradicts.push(ContradictEdge {
            from,
            to,
            reason: reason.to_string(),
            source_id,
            created_at: now,
            resolved: false,
            resolution_source: None,
            resolution_winner: None,
        });
        // Move both to contested, preserving pre_contested_status
        for &id in &[from, to] {
            let node = self.state.nodes.get_mut(&id).unwrap();
            if node.verification_status != VerificationStatus::Contested {
                node.pre_contested_status = Some(node.verification_status);
                node.verification_status = VerificationStatus::Contested;
                node.node_type = NodeType::ContestedStatement;
            }
        }
        self.flush()?;
        Ok(())
    }

    pub fn resolve_contradiction(
        &mut self,
        winner_id: u32,
        loser_id: u32,
        reason: &str,
        source_id: Option<u32>,
    ) -> Result<(), String> {
        // Validate
        for &id in &[winner_id, loser_id] {
            self.state.nodes.get(&id)
                .ok_or(format!("Error: Statement ID {} not found", id))?;
        }
        // Find the contradicts edge (may have been from either direction)
        let edge_idx = self.state.contradicts.iter().position(|e| {
            !e.resolved
                && ((e.from == winner_id && e.to == loser_id)
                    || (e.from == loser_id && e.to == winner_id))
        }).ok_or("Error: no active contradiction edge between these statements".to_string())?;

        // Restore winner to pre_contested_status (or recompute from sources)
        {
            let winner = self.state.nodes.get_mut(&winner_id).unwrap();
            let restored = winner.pre_contested_status
                .unwrap_or_else(|| Self::calculate_verification_status_from_sources(&winner.sources));
            winner.verification_status = restored;
            winner.node_type = NodeType::from_verification_status(restored);
            winner.pre_contested_status = None;
        }

        // Invalidate loser
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        {
            let loser = self.state.nodes.get_mut(&loser_id).unwrap();
            loser.verification_status = VerificationStatus::Invalid;
            loser.node_type = NodeType::InvalidStatement;
            loser.invalid = true;
            loser.invalid_reason = Some(format!("contested_resolved_against: {}", reason));
            loser.invalidated_at = Some(now.clone());
            loser.pre_contested_status = None;
        }

        // Mark edge resolved
        let edge = &mut self.state.contradicts[edge_idx];
        edge.resolved = true;
        edge.resolution_winner = Some(winner_id);
        edge.resolution_source = source_id;

        // Cascade-invalidate loser dependents
        let cascade_ids: Vec<u32> = self.find_cascade_nodes(loser_id)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        for dep_id in cascade_ids {
            if let Some(dep_node) = self.state.nodes.get_mut(&dep_id) {
                if dep_node.invalid || dep_node.verification_status == VerificationStatus::Invalid {
                    continue;
                }
                dep_node.invalid = true;
                dep_node.invalid_reason = Some(format!("depends_on_invalidated:{}", loser_id));
                dep_node.invalidated_at = Some(now.clone());
                dep_node.verification_status = VerificationStatus::Invalid;
                dep_node.node_type = NodeType::InvalidStatement;
            }
        }

        self.flush()?;
        Ok(())
    }

    // ---- because_of (Five Whys) edges -------------------------------------

    /// The single active (non-invalidated) outgoing causal edge for `id`, if any.
    /// Cardinality is enforced at write time so there is at most one.
    fn because_of_active_outgoing(&self, id: u32) -> Option<&BecauseOfEdge> {
        self.state.because_of.iter().find(|e| e.from == id && !e.invalid)
    }

    /// Any outgoing causal edge for `id` (prefers an active one; falls back to
    /// a refuted edge so `why` can still surface a failed link).
    fn because_of_any_outgoing(&self, id: u32) -> Option<&BecauseOfEdge> {
        self.because_of_active_outgoing(id)
            .or_else(|| self.state.because_of.iter().find(|e| e.from == id))
    }

    /// Statements `id` is currently contested with (unresolved contradicts edge).
    fn contested_partners(&self, id: u32) -> Vec<u32> {
        let mut out: Vec<u32> = self.state.contradicts.iter()
            .filter(|e| !e.resolved && (e.from == id || e.to == id))
            .map(|e| if e.from == id { e.to } else { e.from })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Inherited verification status of a causal edge (the causal claim).
    /// Weakest endpoint caps the status below `proven`; when both endpoints are
    /// proven the claim is `partial` until an inverse test promotes it to `proven`.
    fn edge_causal_status(&self, edge: &BecauseOfEdge) -> VerificationStatus {
        if edge.invalid {
            return VerificationStatus::Invalid;
        }
        let s = |id: u32| self.state.nodes.get(&id)
            .map(|n| n.verification_status)
            .unwrap_or(VerificationStatus::Unproven);
        let from_s = s(edge.from);
        let to_s = s(edge.to);
        let floor = if from_s.rank() <= to_s.rank() { from_s } else { to_s };
        if floor.rank() < VerificationStatus::Proven.rank() {
            floor
        } else if edge.test_source.is_some() {
            VerificationStatus::Proven
        } else {
            VerificationStatus::Partial
        }
    }

    /// Longest active causal chain *below* `id` (edge count). Linear under the
    /// single-cardinality rule; the visited set is a defensive cycle guard.
    fn because_of_downstream_depth(&self, id: u32) -> usize {
        let mut depth = 0;
        let mut cur = id;
        let mut visited = HashSet::new();
        visited.insert(cur);
        while let Some(e) = self.because_of_active_outgoing(cur) {
            if !visited.insert(e.to) {
                break;
            }
            depth += 1;
            cur = e.to;
        }
        depth
    }

    /// Longest active causal chain *above* `id` (edge count). May branch, since
    /// many consequents can share one cause.
    fn because_of_upstream_depth(&self, id: u32, visited: &mut HashSet<u32>) -> usize {
        if !visited.insert(id) {
            return 0;
        }
        let mut best = 0;
        for e in &self.state.because_of {
            if e.invalid || e.to != id {
                continue;
            }
            best = best.max(1 + self.because_of_upstream_depth(e.from, visited));
        }
        visited.remove(&id);
        best
    }

    /// Active causal path from `start` down to `target` (inclusive), if one
    /// exists. Used to render the offending loop for cycle detection.
    fn because_of_path(&self, start: u32, target: u32) -> Option<Vec<u32>> {
        let mut path = vec![start];
        let mut cur = start;
        let mut visited = HashSet::new();
        visited.insert(cur);
        loop {
            if cur == target {
                return Some(path);
            }
            match self.because_of_active_outgoing(cur) {
                Some(e) if visited.insert(e.to) => {
                    path.push(e.to);
                    cur = e.to;
                }
                _ => return None,
            }
        }
    }

    /// Add a `because_of` edge `consequent → cause`. Returns an optional
    /// soft-warn message (non-fatal) when the resulting chain is deep.
    pub fn why_add(
        &mut self,
        consequent: u32,
        cause: u32,
        source: Option<String>,
    ) -> Result<Option<String>, String> {
        if consequent == cause {
            return Err("Error: a statement cannot be its own cause".to_string());
        }
        for &id in &[consequent, cause] {
            let node = self.state.nodes.get(&id)
                .ok_or(format!("Error: Statement ID {} not found", id))?;
            if !node.node_type.is_statement_family() {
                return Err(format!(
                    "Error: because_of accepts only statement endpoints — node ID {} is a concept",
                    id
                ));
            }
        }
        if let Some(src) = &source {
            Self::validate_source_prefix(src)?;
        }
        if let Some(existing) = self.because_of_active_outgoing(consequent) {
            let other = existing.to;
            return Err(format!(
                "Error: statement ID:{} already has a cause (ID:{}). \
                 Use 'braim statement contradict {} <other_cause>' if causes compete, \
                 or 'braim why-remove {}' to reassign it to a different cause.",
                consequent, other, other, consequent
            ));
        }
        // Cycle: adding consequent → cause closes a loop iff `cause` already
        // reaches `consequent` via existing causal edges.
        if let Some(path) = self.because_of_path(cause, consequent) {
            let mut loop_ids = path;
            loop_ids.push(cause);
            let rendered = loop_ids.iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" → ");
            return Err(format!("Error: cycle detected: {}", rendered));
        }
        let mut up_visited = HashSet::new();
        let depth = self.because_of_upstream_depth(consequent, &mut up_visited)
            + 1
            + self.because_of_downstream_depth(cause);
        if depth > WHY_DEPTH_MAX {
            return Err(format!(
                "Error: chain depth limit reached ({} links, max {}). \
                 Review the chain for stalling.",
                depth, WHY_DEPTH_MAX
            ));
        }
        let warning = if depth >= WHY_DEPTH_WARN {
            Some(format!(
                "chain depth >= {}, consider whether this is converging on a root cause",
                WHY_DEPTH_WARN
            ))
        } else {
            None
        };
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.state.because_of.push(BecauseOfEdge {
            from: consequent,
            to: cause,
            source,
            created_at: now,
            test_source: None,
            invalid: false,
            invalid_reason: None,
        });
        self.flush()?;
        Ok(warning)
    }

    /// Walk the `because_of` chain from `start` to its root cause.
    pub fn why_chain(&self, start: u32) -> Result<WhyChain, String> {
        {
            let node = self.state.nodes.get(&start)
                .ok_or(format!("Error: Statement ID {} not found", start))?;
            if !node.node_type.is_statement_family() {
                return Err(format!(
                    "Error: because_of accepts only statement endpoints — node ID {} is a concept",
                    start
                ));
            }
        }
        let mut steps = Vec::new();
        let mut visited = HashSet::new();
        let mut current = start;
        loop {
            if !visited.insert(current) {
                break; // defensive cycle guard
            }
            let node = match self.state.nodes.get(&current) {
                Some(n) => n,
                None => break,
            };
            let outgoing = self.because_of_any_outgoing(current).cloned();
            let (causal_status, edge_tested, edge_invalid, next) = match &outgoing {
                Some(edge) => (
                    Some(self.edge_causal_status(edge)),
                    edge.test_source.is_some(),
                    edge.invalid,
                    if edge.invalid { None } else { Some(edge.to) },
                ),
                None => (None, false, false, None),
            };
            steps.push(WhyStep {
                id: current,
                label: node.label.clone(),
                verification_status: node.verification_status,
                causal_status,
                edge_tested,
                edge_invalid,
                contested_with: self.contested_partners(current),
            });
            match next {
                Some(n) => current = n,
                None => break,
            }
        }
        let root_id = steps.last().map(|s| s.id).unwrap_or(start);
        let root_verified = self.state.nodes.get(&root_id)
            .map(|n| n.verification_status.rank() >= VerificationStatus::Proven.rank())
            .unwrap_or(false);
        Ok(WhyChain { steps, root_id, root_verified })
    }

    /// Record an inverse-test result on the active outgoing causal edge of
    /// `consequent`. A pass logs a `test:` source (promoting a proven/proven
    /// edge from partial → proven); a fail refutes the link without touching
    /// the endpoint statements.
    pub fn why_test(
        &mut self,
        consequent: u32,
        passed: bool,
        source: Option<String>,
    ) -> Result<WhyTestOutcome, String> {
        if let Some(src) = &source {
            Self::validate_source_prefix(src)?;
        }
        let idx = self.state.because_of.iter()
            .position(|e| e.from == consequent && !e.invalid)
            .ok_or(format!(
                "Error: statement ID:{} has no active because_of edge to test",
                consequent
            ))?;
        let cause = self.state.because_of[idx].to;
        if passed {
            let src = source.unwrap_or_else(|| "test:inverse_test_passed".to_string());
            self.state.because_of[idx].test_source = Some(src);
        } else {
            self.state.because_of[idx].invalid = true;
            self.state.because_of[idx].invalid_reason = Some("inverse test failed".to_string());
        }
        let edge = self.state.because_of[idx].clone();
        let causal_status = self.edge_causal_status(&edge);
        self.flush()?;
        Ok(WhyTestOutcome { consequent, cause, passed, causal_status })
    }

    /// Remove the outgoing `because_of` edge from `consequent`, freeing it to be
    /// re-pointed at a different cause via `why_add` (chain reassignment).
    /// Prefers the active edge; falls back to a refuted (invalid) one so a
    /// failed link can be cleared before reassigning. Errors when the statement
    /// has no outgoing causal edge.
    pub fn why_remove(&mut self, consequent: u32) -> Result<WhyRemoveOutcome, String> {
        // Prefer the active outgoing edge (the one cardinality enforces);
        // fall back to a refuted edge so stale failed links can be cleared.
        let idx = self.state.because_of.iter()
            .position(|e| e.from == consequent && !e.invalid)
            .or_else(|| self.state.because_of.iter().position(|e| e.from == consequent))
            .ok_or(format!(
                "Error: statement ID:{} has no because_of edge to remove",
                consequent
            ))?;
        let removed = self.state.because_of.remove(idx);
        self.flush()?;
        Ok(WhyRemoveOutcome {
            consequent,
            cause: removed.to,
            was_invalid: removed.invalid,
        })
    }

    /// Mark every consequent above an invalidated cause as needing
    /// re-investigation (metadata flag only — the causal chain is NOT
    /// auto-invalidated, per braim-because-of-edge.tests.md §13).
    fn flag_because_of_reinvestigation(&mut self, invalidated: u32) {
        let mut visited = HashSet::new();
        let mut stack = vec![invalidated];
        while let Some(cur) = stack.pop() {
            let parents: Vec<u32> = self.state.because_of.iter()
                .filter(|e| !e.invalid && e.to == cur)
                .map(|e| e.from)
                .collect();
            for p in parents {
                if visited.insert(p) {
                    if let Some(n) = self.state.nodes.get_mut(&p) {
                        n.metadata.insert(
                            "because_of_reinvestigate".to_string(),
                            format!("cause {} invalidated", invalidated),
                        );
                    }
                    stack.push(p);
                }
            }
        }
    }

    /// Home domain of a node in sharded layout: first entry of its domains list.
    /// Nodes without domains (e.g. source entities) shard to "_unassigned".
    fn home_domain(node: &Node) -> String {
        node.domains.first().cloned().unwrap_or_else(|| "_unassigned".to_string())
    }

    /// Deterministic, filesystem-safe shard filename for a domain. Lowercases and
    /// replaces non-alphanumerics — but distinct domains may then collide (real
    /// case: "Billing" vs "billing", which also collide RAW on case-insensitive
    /// filesystems, braim ID:225/236). Every name therefore carries a short FNV-1a
    /// hash of the exact domain string, making files unique per distinct domain
    /// on every platform while staying human-readable.
    fn shard_filename(domain: &str) -> String {
        let mut sanitized: String = domain.to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if sanitized.len() > 60 {
            sanitized.truncate(60);
        }
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in domain.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{}-{:08x}.json", sanitized, (hash >> 32) as u32 ^ hash as u32)
    }

    /// Load the sharded layout: graph.json (cross-domain state) + every
    /// domains/*.json (per-domain node maps) merged into one in-memory view
    /// (braim ID:217). A node id appearing in two shard files is corruption.
    /// Seqlock read: retry until the shard set is provably free of a concurrent
    /// update. A write is detected if the writer's lock was present at either
    /// end of our read, or if the completion sequence moved during it — which
    /// together cover a writer that starts before, during, or wholly inside the
    /// read. Readers never take the lock, so queries and the viewer stay
    /// non-blocking.
    /// `holds_lock` is set by writers, which already have exclusive access — for
    /// them a plain read is correct, and running the seqlock would make them spin
    /// against their own lock file until timeout.
    fn load_sharded(path: &PathBuf, holds_lock: bool) -> Result<GraphState, String> {
        if holds_lock {
            return Self::load_sharded_once(path);
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let seq_before = read_seq(path);
            if !writer_active(path) {
                let attempt = Self::load_sharded_once(path)?;
                if !writer_active(path) && read_seq(path) == seq_before {
                    return Ok(attempt);
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        // Starvation fallback: under sustained writes a lock-free reader may never
        // find a quiet window, so take the writer lock briefly to force one.
        // Guarantees progress; costs a short wait only on a saturated graph.
        let _guard = FileLock::acquire(path)?;
        Self::load_sharded_once(path)
    }

    fn load_sharded_once(path: &PathBuf) -> Result<GraphState, String> {
        let header_path = path.join("graph.json");
        let mut state: GraphState = if header_path.exists() {
            let content = fs::read_to_string(&header_path)
                .map_err(|e| format!("Failed to read graph.json: {}", e))?;
            serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse graph.json: {}", e))?
        } else {
            return Err("Error: sharded layout (domains/ exists) but graph.json is missing".to_string());
        };

        let dir = fs::read_dir(path.join("domains"))
            .map_err(|e| format!("Failed to read domains dir: {}", e))?;
        let mut files: Vec<PathBuf> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            // Current shards only — versioned snapshots (*.vNNNN.json) are the
            // immutable pin artifacts, not part of the working view.
            .filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                !name.rsplit_once(".v")
                    .map(|(_, tail)| tail.trim_end_matches(".json").chars().all(|c| c.is_ascii_digit())
                        && tail.ends_with(".json"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();

        for file in files {
            let content = fs::read_to_string(&file)
                .map_err(|e| format!("Failed to read {}: {}", file.display(), e))?;
            let shard: HashMap<u32, Node> = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse {}: {}", file.display(), e))?;
            for (id, node) in shard {
                if state.nodes.insert(id, node).is_some() {
                    return Err(format!(
                        "Error: node ID {} appears in more than one domain shard ({}) — corrupt layout",
                        id, file.display()
                    ));
                }
            }
        }
        Ok(state)
    }

    /// Persist the sharded layout: nodes split by home domain into
    /// domains/<name>-<hash>.json, everything else into graph.json. Shard files
    /// whose domain no longer has nodes are removed (node deleted or re-homed).
    fn flush_sharded(&self) -> Result<(), String> {
        let domains_dir = self.data_dir.join("domains");
        fs::create_dir_all(&domains_dir).map_err(|e| format!("Failed to create domains dir: {}", e))?;

        let mut shards: HashMap<String, HashMap<u32, Node>> = HashMap::new();
        for (id, node) in &self.state.nodes {
            shards.entry(Self::home_domain(node)).or_default().insert(*id, node.clone());
        }

        let mut live_files: HashSet<String> = HashSet::new();
        for (domain, nodes) in &shards {
            let filename = Self::shard_filename(domain);
            let content = Self::canonical_json(nodes)?;
            write_atomic(&domains_dir.join(&filename), &content)
                .map_err(|e| format!("Failed to write shard {}: {}", filename, e))?;
            live_files.insert(filename);
        }

        // Remove CURRENT shard files for domains that no longer own any node.
        // Versioned snapshots (*.vNNNN.json) are immutable pin artifacts
        // (ID:214/242) and are never pruned.
        if let Ok(dir) = fs::read_dir(&domains_dir) {
            for entry in dir.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_snapshot = name.rsplit_once(".v")
                    .map(|(_, tail)| tail.trim_end_matches(".json").chars().all(|c| c.is_ascii_digit())
                        && tail.ends_with(".json"))
                    .unwrap_or(false);
                if name.ends_with(".json") && !is_snapshot && !live_files.contains(&name) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        let header = GraphState {
            nodes: HashMap::new(),
            dictionary: self.state.dictionary.clone(),
            id_to_domain: self.state.id_to_domain.clone(),
            gaps: self.state.gaps.clone(),
            next_id: self.state.next_id,
            version: self.state.version,
            contradicts: self.state.contradicts.clone(),
            because_of: self.state.because_of.clone(),
        };
        let content = Self::canonical_json(&header)?;
        write_atomic(&self.data_dir.join("graph.json"), &content)
            .map_err(|e| format!("Failed to write graph.json: {}", e))?;
        // Last: signals to lock-free readers that this multi-file update is whole.
        bump_seq(&self.data_dir)?;
        Ok(())
    }

    /// Fold `loser` into `winner`: union the evidence, move every reference, drop
    /// the duplicate. This is the union-merge the corroboration model assumes
    /// (braim ID:190/248) — before it existed the only route was update-deps plus
    /// delete, which threw the loser's sources away, the exact anti-pattern the
    /// import-union fix removed.
    ///
    /// Deliberately does NOT merge the loser's own dependencies into the winner:
    /// that would silently rewrite what the surviving statement asserts. Any
    /// difference is reported for a human to act on instead.
    pub fn merge_nodes(&mut self, winner: u32, loser: u32) -> Result<MergeOutcome, String> {
        if winner == loser {
            return Err("Error: winner and loser are the same node".to_string());
        }
        let (w_node, l_node) = {
            let w = self.state.nodes.get(&winner)
                .ok_or(format!("Error: node ID {} not found", winner))?;
            let l = self.state.nodes.get(&loser)
                .ok_or(format!("Error: node ID {} not found", loser))?;
            (w.clone(), l.clone())
        };

        // A refuted node's evidence must never be folded into a live one.
        for (id, n) in [(winner, &w_node), (loser, &l_node)] {
            if n.invalid || n.verification_status == VerificationStatus::Invalid {
                return Err(format!(
                    "Error: node ID {} is invalid — merging would launder refuted evidence into a live node",
                    id
                ));
            }
        }
        // Statements and concepts are different kinds of thing.
        if w_node.node_type.is_statement_family() != l_node.node_type.is_statement_family() {
            return Err(format!(
                "Error: cannot merge across kinds — ID:{} is a {:?} and ID:{} is a {:?}",
                winner, w_node.node_type, loser, l_node.node_type
            ));
        }
        // Either direction of dependency between the two means they are not
        // duplicates, and folding them would create a self-loop.
        if w_node.depends_on.contains_key(&loser) || l_node.depends_on.contains_key(&winner) {
            return Err(format!(
                "Error: ID:{} and ID:{} depend on each other — related nodes, not duplicates",
                winner, loser
            ));
        }

        // 1. Union the evidence. Source-entity ids are already local here, so the
        //    remap is the identity.
        let identity: HashMap<u32, u32> = l_node.source_ids.iter().map(|s| (*s, *s)).collect();
        let before = self.state.nodes[&winner].sources.len()
            + self.state.nodes[&winner].source_ids.len();
        self.union_sources_into(winner, &l_node, &identity);
        let sources_added = self.state.nodes[&winner].sources.len()
            + self.state.nodes[&winner].source_ids.len()
            - before;

        // 2. Move every reference. Weights SUM, which preserves the 1.0 invariant
        //    for a referent that depended on both.
        let mut referents_rewired = 0;
        let referent_ids: Vec<u32> = self.state.nodes.iter()
            .filter(|(id, n)| **id != loser && n.depends_on.contains_key(&loser))
            .map(|(id, _)| *id)
            .collect();
        for id in referent_ids {
            let node = self.state.nodes.get_mut(&id).unwrap();
            if let Some(w) = node.depends_on.remove(&loser) {
                *node.depends_on.entry(winner).or_insert(0.0) += w;
                referents_rewired += 1;
            }
        }

        // 3. Move relationship edges, dropping self-edges and duplicates.
        let mut edges_rewired = 0;
        for e in self.state.because_of.iter_mut() {
            if e.from == loser { e.from = winner; edges_rewired += 1; }
            if e.to == loser { e.to = winner; edges_rewired += 1; }
        }
        self.state.because_of.retain(|e| e.from != e.to);
        let mut seen = HashSet::new();
        self.state.because_of.retain(|e| seen.insert((e.from, e.to)));

        for e in self.state.contradicts.iter_mut() {
            if e.from == loser { e.from = winner; edges_rewired += 1; }
            if e.to == loser { e.to = winner; edges_rewired += 1; }
            if e.source_id == Some(loser) { e.source_id = Some(winner); }
            if e.resolution_winner == Some(loser) { e.resolution_winner = Some(winner); }
            if e.resolution_source == Some(loser) { e.resolution_source = Some(winner); }
        }
        self.state.contradicts.retain(|e| e.from != e.to);
        let mut seen = HashSet::new();
        self.state.contradicts.retain(|e| seen.insert((e.from, e.to)));

        // 4. Gap register entries pointing at the loser now point at the winner.
        for g in self.state.gaps.iter_mut() {
            if g.concept_a == loser { g.concept_a = winner; }
            if g.concept_b == loser { g.concept_b = winner; }
        }
        self.state.gaps.retain(|g| g.concept_a != g.concept_b);

        // 5. Drop the loser from the label index.
        for ids in self.state.dictionary.values_mut() {
            ids.retain(|id| *id != loser);
        }
        self.state.dictionary.retain(|_, ids| !ids.is_empty());

        // 6. Leave a trace: a merge is not a deletion, and the audit trail should
        //    say where the winner's extra evidence came from.
        let dep_differences: Vec<u32> = l_node.depends_on.keys()
            .filter(|d| **d != winner && !w_node.depends_on.contains_key(d))
            .copied()
            .collect();
        {
            let w = self.state.nodes.get_mut(&winner).unwrap();
            let prior = w.metadata.get("merged_from").cloned().unwrap_or_default();
            let trace = if prior.is_empty() {
                loser.to_string()
            } else {
                format!("{},{}", prior, loser)
            };
            w.metadata.insert("merged_from".to_string(), trace);
        }

        self.state.nodes.remove(&loser);

        // 7. New evidence may promote the winner.
        if self.state.nodes[&winner].node_type.is_statement_family() {
            self.recompute_statement_status(winner);
        }
        self.dependents = Self::build_dependents(&self.state);
        self.flush()?;

        let mut dep_differences = dep_differences;
        dep_differences.sort();
        Ok(MergeOutcome {
            winner,
            loser,
            sources_added,
            referents_rewired,
            edges_rewired,
            new_status: self.state.nodes[&winner].verification_status,
            dep_differences,
        })
    }

    /// Rename a domain across the graph: every node carrying `old` in its domains
    /// list gets `new` instead. In sharded layout the next flush re-homes the
    /// affected nodes into the new domain's shard and prunes the old current
    /// shard; existing versioned snapshots are immutable history and keep the
    /// old name. Central-governance operation (braim ID:244): distinguishing a
    /// rename from a merge is the caller's evidence-checked decision.
    pub fn rename_domain(&mut self, old: &str, new: &str) -> Result<usize, String> {
        if old == new {
            return Err("Error: old and new domain names are identical".to_string());
        }
        let mut touched = 0;
        for node in self.state.nodes.values_mut() {
            let mut hit = false;
            for d in node.domains.iter_mut() {
                if d == old {
                    *d = new.to_string();
                    hit = true;
                }
            }
            if hit {
                // A node already carrying the new name would end up with a
                // duplicate entry — collapse it.
                let mut seen = HashSet::new();
                node.domains.retain(|d| seen.insert(d.clone()));
                touched += 1;
            }
        }
        if touched == 0 {
            return Err(format!("Error: no node carries domain '{}'", old));
        }
        for v in self.state.id_to_domain.values_mut() {
            if v == old {
                *v = new.to_string();
            }
        }
        self.flush()?;
        Ok(touched)
    }

    /// Convert this data dir from single-file to sharded layout. current.json is
    /// kept as current.json.pre-shard — a full snapshot escape hatch, since the
    /// conversion itself is one-way. Idempotent error if already sharded.
    pub fn shard_layout(&mut self) -> Result<usize, String> {
        if self.data_dir.join("domains").is_dir() {
            return Err("Error: this data dir already uses the sharded layout".to_string());
        }
        self.flush_sharded()?;
        let current = self.data_dir.join("current.json");
        if current.exists() {
            fs::rename(&current, self.data_dir.join("current.json.pre-shard"))
                .map_err(|e| format!("Failed to archive current.json: {}", e))?;
        }
        let domain_count = self.state.nodes.values()
            .map(Self::home_domain)
            .collect::<HashSet<_>>()
            .len();
        Ok(domain_count)
    }

    /// Serialize with deterministic key order. Persisted structs hold HashMaps,
    /// whose iteration order changes per process — direct to_string_pretty made
    /// identical graphs produce differently-ordered JSON (braim ID:218), which
    /// makes git diffs of shared packs unreadable and breaks byte-level
    /// integrity checks (braim ID:226). Round-tripping through serde_json::Value
    /// sorts every map: with the preserve_order feature off, Value objects are
    /// BTreeMap-backed. Numeric keys sort lexicographically ("10" < "2") — ugly
    /// but stable, and stability is the requirement.
    fn canonical_json<T: Serialize>(value: &T) -> Result<String, String> {
        let v = serde_json::to_value(value)
            .map_err(|e| format!("Failed to serialize state: {}", e))?;
        serde_json::to_string_pretty(&v)
            .map_err(|e| format!("Failed to serialize state: {}", e))
    }

    /// The full merged state as canonical JSON — what current.json holds in the
    /// single-file layout. Lets consumers (e.g. `serve`) stay layout-agnostic.
    pub fn state_json(&self) -> Result<String, String> {
        Self::canonical_json(&self.state)
    }

    fn flush(&mut self) -> Result<(), String> {
        if self.data_dir.join("domains").is_dir() {
            return self.flush_sharded();
        }
        let path = self.data_dir.join("current.json");
        let content = Self::canonical_json(&self.state)?;
        write_atomic(&path, &content)
            .map_err(|e| format!("Failed to write current.json: {}", e))?;
        Ok(())
    }

    pub fn add_concept(
        &mut self,
        term: &str,
        domains: Vec<String>,
        sources: Vec<String>,
        depends_on: Option<HashMap<u32, f64>>,
    ) -> Result<u32, GraphError> {
        Self::validate_non_empty(&domains, &sources)?;
        for source in &sources {
            Self::validate_source_prefix(source)?;
        }

        // For atomics, normalize "Concept:description" → "Concept: description" and validate.
        let normalized;
        let term = if depends_on.is_none() {
            let colon = term.find(':').ok_or_else(|| GraphError::Other(format!(
                "Error: Atomic concept label must use 'Concept: description' format \
                 (e.g. 'Library: public lending institution'). Got: '{}'",
                term
            )))?;
            let name = term[..colon].trim();
            let desc = term[colon + 1..].trim();
            if name.is_empty() || desc.is_empty() {
                return Err(GraphError::Other(format!(
                    "Error: Atomic concept label must use 'Concept: description' format \
                     (e.g. 'Library: public lending institution'). Got: '{}'",
                    term
                )));
            }
            normalized = format!("{}: {}", name, desc);
            normalized.as_str()
        } else {
            term
        };

        let lower_term = term.to_lowercase();
        if let Some(existing_ids) = self.state.dictionary.get(&lower_term) {
            for &existing_id in existing_ids {
                if let Some(node) = self.state.nodes.get(&existing_id) {
                    if node.domains == domains {
                        return Err(GraphError::ConceptExists {
                            term: term.to_string(),
                            domains: domains.clone(),
                            id: existing_id,
                        });
                    }
                }
            }
        }

        let (node_type, final_depends_on) = match depends_on {
            Some(deps) => {
                if deps.is_empty() {
                    return Err(GraphError::CompoundNoDependencies);
                }
                self.validate_deps_exist(&deps)?;
                Self::validate_weights_sum_to_one(&deps)?;
                (NodeType::Compound, deps)
            }
            None => {
                (NodeType::Atomic, HashMap::new())
            }
        };

        let id = self.state.next_id;
        self.state.next_id += 1;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let verification_status = Self::calculate_verification_status_from_sources(&sources);
        let node = Node {
            id,
            domains: domains.clone(),
            sources: sources.clone(),
            node_type,
            label: term.to_string(),
            depends_on: final_depends_on,
            status: NodeStatus::Active,
            created_at: now,
            verified_by: HashMap::new(),
            verification_status,
            invalid: false,
            invalid_reason: None,
            invalidated_at: None,
            source_type: None,
            location: None,
            ingested_by: None,
            source_ids: vec![],
            pre_contested_status: None,
            metadata: HashMap::new(),
        };

        self.state.nodes.insert(id, node);
        self.state.dictionary.entry(lower_term.clone()).or_insert_with(Vec::new).push(id);
        if node_type == NodeType::Atomic {
            if let Some(short) = Self::atomic_short_name_key(term) {
                if short != lower_term {
                    self.state.dictionary.entry(short).or_insert_with(Vec::new).push(id);
                }
            }
        }
        self.state.id_to_domain.insert(id, domains[0].clone());

        self.flush()?;
        Ok(id)
    }

    /// For atomic labels like "Library: public lending institution",
    /// returns "library" so concepts can be found by short name alone.
    fn atomic_short_name_key(label: &str) -> Option<String> {
        let pos = label.find(": ")?;
        let name = label[..pos].trim();
        if name.is_empty() { None } else { Some(name.to_lowercase()) }
    }

    pub fn get_node(&self, id: u32) -> Option<&Node> {
        self.state.nodes.get(&id)
    }

    pub fn get_related_nodes(&self, id: u32) -> (Vec<(u32, &Node)>, Vec<(u32, &Node)>) {
        let mut depends_on = Vec::new();
        let mut depended_by = Vec::new();

        if let Some(node) = self.state.nodes.get(&id) {
            for &dep_id in node.depends_on.keys() {
                if let Some(dep_node) = self.state.nodes.get(&dep_id) {
                    depends_on.push((dep_id, dep_node));
                }
            }
        }

        for (other_id, other_node) in &self.state.nodes {
            if other_node.depends_on.contains_key(&id) {
                depended_by.push((*other_id, other_node));
            }
        }

        (depends_on, depended_by)
    }

    pub fn get_related_nodes_bounded(&self, id: u32) -> (Vec<(u32, &Node)>, Vec<(u32, &Node)>) {
        let (depends_on, depended_by) = self.get_related_nodes(id);
        (
            depends_on.into_iter().take(10).collect(),
            depended_by.into_iter().take(10).collect(),
        )
    }

    pub fn get_domain_stats(&self) -> Vec<(String, usize)> {
        let mut domain_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        for node in self.state.nodes.values() {
            if node.status == NodeStatus::Active {
                for domain in &node.domains {
                    *domain_counts.entry(domain.clone()).or_insert(0) += 1;
                }
            }
        }

        let mut result: Vec<_> = domain_counts.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    fn find_concepts_fuzzy(&self, query: &str) -> Vec<u32> {
        let lower_query = query.to_lowercase();

        if let Some(ids) = self.state.dictionary.get(&lower_query) {
            return ids.clone();
        }

        // Pure-digit strings are ID-shaped. Never substring-match them against label text —
        // "2" would spuriously hit "opened in 2024". Use resolve_term for numeric args instead.
        if lower_query.chars().all(|c| c.is_ascii_digit()) {
            return vec![];
        }

        let mut matches: Vec<(u32, f64)> = Vec::new();

        for (node_id, node) in &self.state.nodes {
            if node.status != NodeStatus::Active {
                continue;
            }

            let lower_label = node.label.to_lowercase();

            let score = if lower_label.starts_with(&lower_query) {
                0.8
            } else if lower_label.contains(&lower_query) {
                0.6
            } else {
                continue;
            };

            matches.push((*node_id, score));
        }

        matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        matches.into_iter().map(|(id, _)| id).collect()
    }

    /// Resolve a term to node IDs for perspective/proximity.
    /// Numeric arg → exact ID lookup; error if ID missing (no silent fuzzy fallback).
    /// Text arg → find_concepts_fuzzy; error if no match.
    fn resolve_term(&self, term: &str) -> Result<Vec<u32>, String> {
        if let Ok(id) = term.parse::<u32>() {
            if self.state.nodes.contains_key(&id) {
                Ok(vec![id])
            } else {
                Err(format!("Error: Node ID {} not found", id))
            }
        } else {
            let ids = self.find_concepts_fuzzy(term);
            if ids.is_empty() {
                Err(format!("Error: Unknown concept '{}'", term))
            } else {
                Ok(ids)
            }
        }
    }

    fn validate_statement_concepts(&self, text: &str, depends_on: &HashMap<u32, f64>) -> Result<(), String> {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let mut concept_positions: Vec<(usize, u32, String)> = Vec::new();

        for (i, token) in tokens.iter().enumerate() {
            let lower = token.to_lowercase();
            if let Some(ids) = self.state.dictionary.get(&lower) {
                for &id in ids {
                    if let Some(node) = self.state.nodes.get(&id) {
                        concept_positions.push((i, id, node.label.clone()));
                    }
                }
            }
        }

        if concept_positions.is_empty() {
            return Ok(());
        }

        let mut i = 0;
        let mut concerns = Vec::new();

        while i < concept_positions.len() {
            let mut j = i;
            let mut sequence = vec![concept_positions[i].1];

            while j + 1 < concept_positions.len()
                && concept_positions[j + 1].0 == concept_positions[j].0 + 1 {
                j += 1;
                sequence.push(concept_positions[j].1);
            }

            if sequence.len() >= 2 {
                let concern = self.check_adjacent_sequence(&sequence, depends_on)?;
                if !concern.is_empty() {
                    concerns.push(concern);
                }
            }

            i = j + 1;
        }

        if !concerns.is_empty() {
            return Err(format!(
                "⚠ Adjacency validation concerns:\n{}\n\nUse --assume to bypass.",
                concerns.join("\n")
            ));
        }

        Ok(())
    }

    fn check_adjacent_sequence(&self, sequence: &[u32], depends_on: &HashMap<u32, f64>) -> Result<String, String> {
        let mut concern = String::new();
        let mut seq = sequence.to_vec();

        while seq.len() >= 2 {
            let right_idx = seq.len() - 2;
            let pair = &seq[right_idx..];

            let labels: Vec<String> = pair.iter()
                .filter_map(|&id| self.state.nodes.get(&id).map(|n| n.label.clone()))
                .collect();

            if labels.len() == 2 {
                // Use name parts (before ': ') so "Library: lends books" + "Card: id token"
                // → "Library Card", matching the compound label rather than the full descriptions.
                let name0 = Self::label_name_part(&labels[0]);
                let name1 = Self::label_name_part(&labels[1]);
                let pair_name = format!("{} {}", name0, name1);
                let pair_found = self.find_compound_by_label(&pair_name);

                match pair_found {
                    Some(compound_id) => {
                        if !depends_on.contains_key(&compound_id) {
                            return Err(format!(
                                "Error: Compound '{}' (ID:{}) exists but statement doesn't use it.\n  \
                                This creates branching paths. Use compound in statement or investigate first.",
                                pair_name, compound_id
                            ));
                        }
                        // Compound exists and is in --depends → silent pass.
                    }
                    None => {
                        concern.push_str(&format!("  • Adjacent concepts '{}' → suggest creating compound\n", pair_name));
                    }
                }
            }

            seq.pop();
        }

        Ok(concern)
    }

    /// Returns the concept name: the part before ": " for colon-format atomics,
    /// or the full label for compounds and legacy bare-noun atomics.
    fn label_name_part(label: &str) -> &str {
        if let Some(pos) = label.find(": ") {
            label[..pos].trim()
        } else {
            label
        }
    }

    fn find_compound_by_label(&self, label: &str) -> Option<u32> {
        for (id, node) in &self.state.nodes {
            if node.label.eq_ignore_ascii_case(label) && node.node_type == NodeType::Compound {
                return Some(*id);
            }
        }
        None
    }

    /// Shared validators (centralized so add_concept/add_statement don't drift).
    /// Domain/source *arity* is intentionally not validated here (decoupled).
    fn validate_non_empty(domains: &[String], sources: &[String]) -> Result<(), GraphError> {
        if domains.is_empty() || sources.is_empty() {
            return Err(GraphError::EmptyDomainsSources);
        }
        Ok(())
    }

    fn validate_weights_sum_to_one(deps: &HashMap<u32, f64>) -> Result<(), GraphError> {
        let sum: f64 = deps.values().sum();
        if (sum - 1.0).abs() > 0.001 {
            return Err(GraphError::WeightsNotOne(sum));
        }
        Ok(())
    }

    fn validate_deps_exist(&self, deps: &HashMap<u32, f64>) -> Result<(), GraphError> {
        for &dep_id in deps.keys() {
            if !self.state.nodes.contains_key(&dep_id) {
                return Err(GraphError::DependencyNotFound(dep_id));
            }
        }
        Ok(())
    }

    pub fn add_statement(
        &mut self,
        text: &str,
        domains: Vec<String>,
        sources: Vec<String>,
        depends_on: HashMap<u32, f64>,
        assume: bool,
    ) -> Result<u32, GraphError> {
        if depends_on.is_empty() {
            return Err(GraphError::StatementNoDependency);
        }
        Self::validate_non_empty(&domains, &sources)?;
        for source in &sources {
            if source != &"inferred".to_string() {
                Self::validate_source_prefix(source)?;
            }
        }

        self.validate_deps_exist(&depends_on)?;
        Self::validate_weights_sum_to_one(&depends_on)?;

        if !assume {
            if let Err(validation_msg) = self.validate_statement_concepts(text, &depends_on) {
                return Err(validation_msg.into());
            }
        }

        let id = self.state.next_id;
        self.state.next_id += 1;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        // Source-derived status per BRAIM_AUTOPROMOTION_SPEC §3.2.
        let source_derived = Self::calculate_verification_status_from_sources(&sources);

        // Dependency inheritance per BRAIM_DEPENDENCY_INHERITANCE_SPEC §3.2:
        // Only statement-typed deps participate; concept deps are skipped.
        // Invalid deps propagate fully (mark the new node invalid).
        // Otherwise cap source_derived to the weakest statement dep.
        let mut invalid_from_dep: Option<u32> = None;
        let mut dep_cap: Option<u8> = None;
        for dep_id in depends_on.keys() {
            if let Some(dep_node) = self.state.nodes.get(dep_id) {
                if !dep_node.node_type.is_statement_family() {
                    continue;
                }
                if dep_node.invalid || dep_node.verification_status == VerificationStatus::Invalid {
                    invalid_from_dep = Some(*dep_id);
                    break;
                }
                let r = dep_node.verification_status.rank();
                dep_cap = Some(match dep_cap {
                    Some(prev) => prev.min(r),
                    None => r,
                });
            }
        }

        let (verification_status, invalid_flag, invalid_reason, invalidated_at) =
            if let Some(dep_id) = invalid_from_dep {
                (
                    VerificationStatus::Invalid,
                    true,
                    Some(format!("depends_on_invalidated:{}", dep_id)),
                    Some(now.clone()),
                )
            } else {
                let final_status = match dep_cap {
                    None => source_derived,
                    Some(cap) => {
                        if source_derived.rank() <= cap {
                            source_derived
                        } else {
                            VerificationStatus::from_rank(cap)
                        }
                    }
                };
                (final_status, false, None, None)
            };

        let node = Node {
            id,
            domains: domains.clone(),
            sources: sources.clone(),
            // Per BRAIM_NODE_TYPE_CLAIM_FACT_SPEC §3.3 — derive node_type from status.
            node_type: NodeType::from_verification_status(verification_status),
            label: text.to_string(),
            depends_on: depends_on.clone(),
            status: NodeStatus::Active,
            created_at: now,
            verified_by: HashMap::new(),
            verification_status,
            invalid: invalid_flag,
            invalid_reason,
            invalidated_at,
            source_type: None,
            location: None,
            ingested_by: None,
            source_ids: vec![],
            pre_contested_status: None,
            metadata: HashMap::new(),
        };

        self.state.nodes.insert(id, node);

        // Auto-clear gap register for newly connected pairs.
        let new_statement_deps: Vec<u32> = depends_on.keys().cloned().collect();
        self.clear_gaps_for_deps(&new_statement_deps);

        self.flush()?;
        Ok(id)
    }

    /// Drop gap-register entries covered by any pair of the given concept ids.
    fn clear_gaps_for_deps(&mut self, dep_ids: &[u32]) {
        self.state.gaps.retain(|gap| {
            for i in 0..dep_ids.len() {
                for j in (i + 1)..dep_ids.len() {
                    let (id_a, id_b) = (dep_ids[i], dep_ids[j]);
                    if (gap.concept_a == id_a && gap.concept_b == id_b)
                        || (gap.concept_a == id_b && gap.concept_b == id_a)
                    {
                        return false;
                    }
                }
            }
            true
        });
    }

    pub fn update_weights(
        &mut self,
        node_id: u32,
        new_weights: HashMap<u32, f64>,
    ) -> Result<(), String> {
        if new_weights.is_empty() {
            return Err("Error: Node must have at least 1 dependency".to_string());
        }

        if !self.state.nodes.contains_key(&node_id) {
            return Err(format!("Error: Node ID {} does not exist", node_id));
        }

        for &dep_id in new_weights.keys() {
            if !self.state.nodes.contains_key(&dep_id) {
                return Err(format!("Error: Dependency ID {} does not exist", dep_id));
            }
        }

        let sum: f64 = new_weights.values().sum();
        if (sum - 1.0).abs() > 0.001 {
            return Err(format!("Error: Weights must sum to 1.0 — got {:.4}", sum));
        }

        let current_count = self.state.nodes.get(&node_id).unwrap().depends_on.len();
        if new_weights.len() != current_count {
            return Err(format!(
                "Error: Cannot change number of dependencies. Current: {}, provided: {}",
                current_count,
                new_weights.len()
            ));
        }

        self.state.nodes.get_mut(&node_id).unwrap().depends_on = new_weights;
        self.flush()?;
        Ok(())
    }

    /// Set a first-class metadata key on a node (braim 6336). Structured, not
    /// label/domain-encoded — so scope/status/affected_feature are queryable.
    pub fn set_meta(&mut self, node_id: u32, key: &str, value: &str) -> Result<(), String> {
        let node = self.state.nodes.get_mut(&node_id)
            .ok_or_else(|| format!("Error: Node ID {} does not exist", node_id))?;
        node.metadata.insert(key.to_string(), value.to_string());
        self.flush()?;
        Ok(())
    }

    /// Increment a numeric metadata key (absent/non-numeric treated as 0).
    /// Returns the new value. The clean recurrence_count increment (braim 6336).
    pub fn inc_meta(&mut self, node_id: u32, key: &str) -> Result<i64, String> {
        let node = self.state.nodes.get_mut(&node_id)
            .ok_or_else(|| format!("Error: Node ID {} does not exist", node_id))?;
        let cur = node.metadata.get(key).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let next = cur + 1;
        node.metadata.insert(key.to_string(), next.to_string());
        self.flush()?;
        Ok(next)
    }

    /// All node ids whose metadata[key] == value — queryable differentiation
    /// (e.g. scope=cognitivex_flow vs scope=deliverable, status=open).
    pub fn nodes_by_meta(&self, key: &str, value: &str) -> Vec<u32> {
        let mut ids: Vec<u32> = self.state.nodes.values()
            .filter(|n| n.metadata.get(key).map(|v| v == value).unwrap_or(false))
            .map(|n| n.id)
            .collect();
        ids.sort();
        ids
    }

    pub fn propagate(&self, term: &str) -> (HashMap<u32, f64>, bool) {
        let source_ids = self.find_concepts_fuzzy(term);

        if source_ids.is_empty() {
            return (HashMap::new(), false);
        }

        let is_fuzzy = !self.state.dictionary.contains_key(&term.to_lowercase());

        let mut scores: HashMap<u32, f64> = HashMap::new();
        let mut visited: HashSet<u32> = HashSet::new();
        let mut queue: VecDeque<(u32, f64)> = VecDeque::new();

        for source_id in source_ids {
            scores.insert(source_id, 1.0);
            if visited.insert(source_id) {
                queue.push_back((source_id, 1.0));
            }
        }

        // Bounded BFS over the reverse-adjacency index: each node is expanded
        // at most once (visited-set), and finding dependents is an O(1) index
        // lookup rather than an O(n) full scan. Total work is O(V+E), replacing
        // the old O(n^2)-capped scan that hung on high-fan-out terms.
        while let Some((current_id, acc)) = queue.pop_front() {
            if let Some(deps) = self.dependents.get(&current_id) {
                for &(node_id, edge_w) in deps {
                    let new_acc = acc * edge_w;
                    *scores.entry(node_id).or_insert(0.0) += new_acc;
                    if visited.insert(node_id) {
                        queue.push_back((node_id, new_acc));
                    }
                }
            }
        }

        (scores, is_fuzzy)
    }

    pub fn lookup(&self, term: &str) -> Result<(Vec<(u32, f64)>, bool), String> {
        let (mut scores, is_fuzzy) = self.propagate(term);

        if scores.is_empty() && !is_fuzzy {
            return Err(format!("Error: Unknown concept '{}'", term));
        }

        let lower_term = term.to_lowercase();
        if let Some(source_ids) = self.state.dictionary.get(&lower_term) {
            for &source_id in source_ids {
                scores.remove(&source_id);
            }
        }

        let mut result: Vec<_> = scores
            .into_iter()
            .map(|(id, score)| (id, (score * 10000.0).round() / 10000.0))
            .collect();
        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        Ok((result, is_fuzzy))
    }

    pub fn lookup_exact(&self, term: &str) -> Result<(Vec<(u32, f64)>, bool), String> {
        let lower_term = term.to_lowercase();

        if let Some(source_ids) = self.state.dictionary.get(&lower_term) {
            let result: Vec<(u32, f64)> = source_ids.iter().map(|&id| (id, 1.0)).collect();
            Ok((result, false))
        } else {
            Err(format!("Error: Unknown concept '{}'", term))
        }
    }

    pub fn query(&self, terms: &[&str]) -> Result<Vec<(u32, f64)>, String> {
        let mut has_any_match = false;
        for term in terms {
            let fuzzy_ids = self.find_concepts_fuzzy(term);
            if !fuzzy_ids.is_empty() {
                has_any_match = true;
                break;
            }
        }
        if !has_any_match {
            return Ok(Vec::new());
        }

        let score_maps: Vec<_> = terms.iter().map(|t| self.propagate(t).0).collect();

        let source_ids: HashSet<u32> = terms
            .iter()
            .flat_map(|t| self.find_concepts_fuzzy(t))
            .collect();

        if score_maps.is_empty() {
            return Ok(Vec::new());
        }

        let mut common: HashSet<u32> = score_maps[0].keys().copied().collect();
        for map in &score_maps[1..] {
            common.retain(|k| map.contains_key(k));
        }

        common.retain(|id| !source_ids.contains(id));

        let mut result: Vec<(u32, f64)> = common
            .into_iter()
            .map(|node_id| {
                let total: f64 = score_maps.iter().map(|m| m.get(&node_id).copied().unwrap_or(0.0)).sum();
                (node_id, (total * 10000.0).round() / 10000.0)
            })
            .collect();

        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        Ok(result)
    }

    fn dfs(
        &self,
        current: u32,
        target: u32,
        visited: &mut HashSet<u32>,
        path: &mut Vec<u32>,
        acc: f64,
        paths: &mut Vec<PathInfo>,
    ) {
        if current == target {
            let domains: Vec<String> = path
                .iter()
                .filter_map(|&id| self.state.nodes.get(&id).map(|n| n.domains[0].clone()))
                .collect();
            let weight = (acc * 10000.0).round() / 10000.0;
            paths.push(PathInfo {
                path: path.clone(),
                weight,
                domains,
            });
            return;
        }

        // Merge two edge sources into one candidate set (dedup by node, keeping
        // the strongest weight): compositional depends_on (reversed: a node that
        // references `current` as a dependency) and causal because_of (a
        // consequent whose cause is `current`, so the walk runs cause →
        // consequent, mirroring the dependency → dependent direction).
        let mut candidate_w: HashMap<u32, f64> = HashMap::new();
        for node in self.state.nodes.values() {
            if node.status == NodeStatus::Active
                && node.depends_on.contains_key(&current)
                && !visited.contains(&node.id)
            {
                let w = node.depends_on[&current];
                candidate_w.entry(node.id).and_modify(|e| if w > *e { *e = w }).or_insert(w);
            }
        }
        // because_of is unweighted: an active causal link propagates at full
        // strength (weight 1.0) so it never dilutes a multiplicative path.
        for edge in &self.state.because_of {
            if edge.invalid || edge.to != current || visited.contains(&edge.from) {
                continue;
            }
            if self.state.nodes.get(&edge.from).map_or(true, |n| n.status != NodeStatus::Active) {
                continue;
            }
            candidate_w.entry(edge.from).and_modify(|e| if 1.0 > *e { *e = 1.0 }).or_insert(1.0);
        }
        let candidates: Vec<(u32, f64)> = candidate_w.into_iter().collect();

        for (nid, edge_w) in candidates {
            visited.insert(nid);
            path.push(nid);
            self.dfs(nid, target, visited, path, acc * edge_w, paths);
            path.pop();
            visited.remove(&nid);
        }
    }

    fn find_paths(&mut self, ids_a: &[u32], ids_b: &[u32]) -> (Vec<PathInfo>, Option<(u32, u32)>) {
        let mut paths = Vec::new();

        for id_a in ids_a {
            for &id_b in ids_b {
                let mut visited = HashSet::new();
                visited.insert(*id_a);
                let mut path = vec![*id_a];
                self.dfs(*id_a, id_b, &mut visited, &mut path, 1.0, &mut paths);
            }
        }

        let gap = if paths.is_empty() {
            ids_a.first().copied().zip(ids_b.first().copied())
        } else {
            None
        };

        paths.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap());
        (paths, gap)
    }

    pub fn proximity(&mut self, term_a: &str, term_b: &str) -> Result<Vec<PathInfo>, String> {
        let ids_a = self.resolve_term(term_a)?;
        let ids_b = self.resolve_term(term_b)?;

        if let Some(&shared) = ids_a.iter().find(|id| ids_b.contains(id)) {
            return Err(format!(
                "Error: '{}' and '{}' both resolve to ID:{} — no path to compute",
                term_a, term_b, shared
            ));
        }

        let (paths, gap) = self.find_paths(&ids_a, &ids_b);

        if let Some((id_a, id_b)) = gap {
            let labels = {
                let node_a = self.state.nodes.get(&id_a).map(|n| n.label.clone());
                let node_b = self.state.nodes.get(&id_b).map(|n| n.label.clone());
                (node_a, node_b)
            };
            if let (Some(label_a), Some(label_b)) = labels {
                self.register_gap(id_a, id_b, &label_a, &label_b);
                self.flush()?;
            }
        }

        Ok(paths)
    }

    pub fn perspective(&mut self, term_a: &str, term_b: &str) -> Result<HashMap<String, f64>, String> {
        let ids_a = self.resolve_term(term_a)?;
        let ids_b = self.resolve_term(term_b)?;

        if let Some(&shared) = ids_a.iter().find(|id| ids_b.contains(id)) {
            return Err(format!(
                "Error: '{}' and '{}' both resolve to ID:{} — no path to compute",
                term_a, term_b, shared
            ));
        }

        let (paths, gap) = self.find_paths(&ids_a, &ids_b);

        if let Some((id_a, id_b)) = gap {
            let labels = {
                let node_a = self.state.nodes.get(&id_a).map(|n| n.label.clone());
                let node_b = self.state.nodes.get(&id_b).map(|n| n.label.clone());
                (node_a, node_b)
            };
            if let (Some(label_a), Some(label_b)) = labels {
                self.register_gap(id_a, id_b, &label_a, &label_b);
                self.flush()?;
            }
        }

        let mut domain_weights: HashMap<String, f64> = HashMap::new();
        for path in paths {
            if let Some(domain) = path.domains.first() {
                *domain_weights.entry(domain.clone()).or_insert(0.0) += path.weight;
            }
        }

        Ok(domain_weights)
    }

    fn register_gap(&mut self, id_a: u32, id_b: u32, label_a: &str, label_b: &str) {
        for gap in &self.state.gaps {
            if (gap.concept_a == id_a && gap.concept_b == id_b)
                || (gap.concept_a == id_b && gap.concept_b == id_a)
            {
                return;
            }
        }

        self.state.gaps.push(GapRecord {
            concept_a: id_a,
            concept_b: id_b,
            label_a: label_a.to_string(),
            label_b: label_b.to_string(),
            status: "pending".to_string(),
            note: format!("No path found between '{}' and '{}'", label_a, label_b),
        });
    }

    pub fn audit(&self) -> AuditReport {
        let mut referenced = HashSet::new();
        for node in self.state.nodes.values() {
            for &dep_id in node.depends_on.keys() {
                referenced.insert(dep_id);
            }
            // Source entities are referenced via attachment, not depends_on.
            for &src_id in &node.source_ids {
                referenced.insert(src_id);
            }
        }

        let mut orphans = Vec::new();
        let mut pending = Vec::new();
        let mut deprecated_referenced = Vec::new();
        let mut refuted_causal_links = Vec::new();
        let mut reinvestigate_flagged = Vec::new();
        let mut untested_causal_links = Vec::new();
        let mut unverified_roots = Vec::new();

        for node in self.state.nodes.values() {
            if node.status == NodeStatus::Active
                && node.depends_on.is_empty()
                && !referenced.contains(&node.id)
            {
                orphans.push(node.clone());
            }
            if node.status == NodeStatus::Pending {
                pending.push(node.clone());
            }
            if node.status == NodeStatus::Deprecated && referenced.contains(&node.id) {
                deprecated_referenced.push(node.clone());
            }
            // Statements flagged for re-investigation after a cause below them
            // was invalidated (flag_because_of_reinvestigation).
            if node.metadata.contains_key("because_of_reinvestigate") {
                reinvestigate_flagged.push(node.clone());
            }
        }

        // because_of-derived findings. label() resolves an endpoint id to a
        // short label for display (falls back to the bare id when missing).
        let label = |id: u32| self.state.nodes.get(&id)
            .map(|n| n.label.clone())
            .unwrap_or_else(|| format!("ID:{}", id));

        // Statements that are the cause end (`to`) of some active edge — i.e.
        // they sit inside a chain — used to find terminal roots below.
        let mut is_cause_in_chain: HashSet<u32> = HashSet::new();
        let mut has_active_outgoing: HashSet<u32> = HashSet::new();
        for edge in &self.state.because_of {
            if edge.invalid {
                refuted_causal_links.push(CausalEdgeInfo {
                    from: edge.from,
                    from_label: label(edge.from),
                    to: edge.to,
                    to_label: label(edge.to),
                    reason: edge.invalid_reason.clone(),
                });
                continue;
            }
            // active edge
            is_cause_in_chain.insert(edge.to);
            has_active_outgoing.insert(edge.from);
            if edge.test_source.is_none() {
                untested_causal_links.push(CausalEdgeInfo {
                    from: edge.from,
                    from_label: label(edge.from),
                    to: edge.to,
                    to_label: label(edge.to),
                    reason: None,
                });
            }
        }

        // Terminal root cause = cause-in-a-chain with no active outgoing edge of
        // its own; flag when its verification is below `proven`.
        for &id in &is_cause_in_chain {
            if has_active_outgoing.contains(&id) {
                continue;
            }
            if let Some(node) = self.state.nodes.get(&id) {
                if node.verification_status.rank() < VerificationStatus::Proven.rank() {
                    unverified_roots.push(node.clone());
                }
            }
        }

        // Deterministic ordering (HashMap/HashSet iteration is unordered).
        refuted_causal_links.sort_by_key(|e| (e.from, e.to));
        untested_causal_links.sort_by_key(|e| (e.from, e.to));
        reinvestigate_flagged.sort_by_key(|n| n.id);
        unverified_roots.sort_by_key(|n| n.id);

        AuditReport {
            orphans,
            pending,
            gaps: self.state.gaps.clone(),
            deprecated_referenced,
            refuted_causal_links,
            reinvestigate_flagged,
            untested_causal_links,
            unverified_roots,
        }
    }

    fn versions_index_path(&self) -> PathBuf {
        self.data_dir.join("versions.json")
    }

    fn read_versions_index(&self) -> Vec<ShardedVersionEntry> {
        fs::read_to_string(self.versions_index_path())
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    /// Snapshot filename for one domain at one version: the `<d><v>.json` pin
    /// artifact (braim ID:214/242), hash-suffixed like the current shard.
    fn shard_version_filename(domain: &str, version: u32) -> String {
        let base = Self::shard_filename(domain);
        format!("{}.v{:04}.json", base.trim_end_matches(".json"), version)
    }

    pub fn version_save(&mut self, description: &str) -> Result<u32, String> {
        self.state.version += 1;
        let version_num = self.state.version;
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        if self.data_dir.join("domains").is_dir() {
            // Sharded: per-domain snapshots for changed domains only, plus a
            // header snapshot and an index entry (ID:242). Unchanged domains
            // keep their existing snapshot version — the pin stays stable.
            let domains_dir = self.data_dir.join("domains");
            let mut index = self.read_versions_index();
            let prev: HashMap<String, u32> = index.last()
                .map(|e| e.domain_versions.clone())
                .unwrap_or_default();
            let prev_header = index.last().map(|e| e.header_version).unwrap_or(0);

            let mut shards: HashMap<String, HashMap<u32, Node>> = HashMap::new();
            for (id, node) in &self.state.nodes {
                shards.entry(Self::home_domain(node)).or_default().insert(*id, node.clone());
            }

            let mut domain_versions: HashMap<String, u32> = HashMap::new();
            for (domain, nodes) in &shards {
                let content = Self::canonical_json(nodes)?;
                let next = match prev.get(domain) {
                    Some(&v) => {
                        let prev_file = domains_dir.join(Self::shard_version_filename(domain, v));
                        match fs::read_to_string(&prev_file) {
                            Ok(existing) if existing == content => {
                                domain_versions.insert(domain.clone(), v);
                                continue;
                            }
                            _ => v + 1,
                        }
                    }
                    None => 1,
                };
                write_atomic(&domains_dir.join(Self::shard_version_filename(domain, next)), &content)
                    .map_err(|e| format!("Failed to write domain snapshot: {}", e))?;
                domain_versions.insert(domain.clone(), next);
            }

            let header = GraphState {
                nodes: HashMap::new(),
                dictionary: self.state.dictionary.clone(),
                id_to_domain: self.state.id_to_domain.clone(),
                gaps: self.state.gaps.clone(),
                next_id: self.state.next_id,
                version: self.state.version,
                contradicts: self.state.contradicts.clone(),
                because_of: self.state.because_of.clone(),
            };
            let header_content = Self::canonical_json(&header)?;
            let header_version = {
                let prev_file = self.data_dir.join(format!("graph.v{:04}.json", prev_header));
                match fs::read_to_string(&prev_file) {
                    Ok(existing) if existing == header_content => prev_header,
                    _ => {
                        let hv = prev_header + 1;
                        write_atomic(&self.data_dir.join(format!("graph.v{:04}.json", hv)), &header_content)
                            .map_err(|e| format!("Failed to write header snapshot: {}", e))?;
                        hv
                    }
                }
            };

            index.push(ShardedVersionEntry {
                version: version_num,
                description: description.to_string(),
                saved_at: now,
                node_count: self.state.nodes.len(),
                domain_versions,
                header_version,
            });
            let index_content = Self::canonical_json(&index)?;
            write_atomic(&self.versions_index_path(), &index_content)
                .map_err(|e| format!("Failed to write versions index: {}", e))?;
        } else {
            let meta = VersionMeta {
                description: description.to_string(),
                saved_at: now,
                data: self.state.clone(),
            };
            let filename = format!("v{:04}.json", version_num);
            let path = self.data_dir.join(&filename);
            let content = Self::canonical_json(&meta)
                .map_err(|e| format!("Failed to serialize version: {}", e))?;
            write_atomic(&path, &content)
                .map_err(|e| format!("Failed to write version file: {}", e))?;
        }

        self.flush()?;
        Ok(version_num)
    }

    pub fn version_restore(&mut self, n: u32) -> Result<(), String> {
        if self.data_dir.join("domains").is_dir() {
            let index = self.read_versions_index();
            let entry = index.iter().find(|e| e.version == n)
                .ok_or(format!("Error: Version {} not found in versions.json", n))?;

            let header_content = fs::read_to_string(
                self.data_dir.join(format!("graph.v{:04}.json", entry.header_version)))
                .map_err(|_| format!("Error: header snapshot graph.v{:04}.json missing", entry.header_version))?;
            let mut state: GraphState = serde_json::from_str(&header_content)
                .map_err(|e| format!("Failed to parse header snapshot: {}", e))?;

            for (domain, v) in &entry.domain_versions {
                let file = self.data_dir.join("domains").join(Self::shard_version_filename(domain, *v));
                let content = fs::read_to_string(&file)
                    .map_err(|_| format!("Error: domain snapshot {} missing", file.display()))?;
                let nodes: HashMap<u32, Node> = serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse domain snapshot: {}", e))?;
                state.nodes.extend(nodes);
            }
            self.state = state;
        } else {
            let filename = format!("v{:04}.json", n);
            let path = self.data_dir.join(&filename);
            let content = fs::read_to_string(&path)
                .map_err(|_| format!("Error: Version {} not found", n))?;
            let meta: VersionMeta = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse version file: {}", e))?;
            self.state = meta.data;
        }
        self.flush()?;
        Ok(())
    }

    pub fn version_list(&self) -> Result<Vec<VersionInfo>, String> {
        if self.data_dir.join("domains").is_dir() {
            return Ok(self.read_versions_index().into_iter().map(|e| VersionInfo {
                version: e.version,
                description: e.description,
                saved_at: e.saved_at,
                node_count: e.node_count,
            }).collect());
        }

        let mut versions = Vec::new();
        for entry in fs::read_dir(&self.data_dir)
            .map_err(|e| format!("Failed to read data dir: {}", e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
            let path = entry.path();
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if filename.starts_with('v') && filename.ends_with(".json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(meta) = serde_json::from_str::<VersionMeta>(&content) {
                        versions.push(VersionInfo {
                            version: meta.data.version,
                            description: meta.description,
                            saved_at: meta.saved_at,
                            node_count: meta.data.nodes.len(),
                        });
                    }
                }
            }
        }

        versions.sort_by(|a, b| a.saved_at.cmp(&b.saved_at));
        Ok(versions)
    }

    pub fn delete_node(&mut self, node_id: u32) -> Result<Vec<u32>, String> {
        // Check if node exists
        let node = self.state.nodes.get(&node_id)
            .ok_or(format!("Error: Node ID {} not found", node_id))?;

        // Find nodes that depend on this node
        let mut dependents = Vec::new();
        for (other_id, other_node) in &self.state.nodes {
            if other_node.depends_on.contains_key(&node_id) {
                dependents.push(*other_id);
            }
        }

        // Remove from dictionary
        let lower_label = node.label.to_lowercase();
        if let Some(ids) = self.state.dictionary.get_mut(&lower_label) {
            ids.retain(|&id| id != node_id);
        }
        if node.node_type == NodeType::Atomic {
            if let Some(short) = Self::atomic_short_name_key(&node.label) {
                if short != lower_label {
                    if let Some(ids) = self.state.dictionary.get_mut(&short) {
                        ids.retain(|&id| id != node_id);
                    }
                }
            }
        }

        // Remove from id_to_domain
        self.state.id_to_domain.remove(&node_id);

        // Remove from nodes
        self.state.nodes.remove(&node_id);

        // Remove from gaps if referenced
        self.state.gaps.retain(|gap| gap.concept_a != node_id && gap.concept_b != node_id);

        self.flush()?;
        Ok(dependents)
    }

    /// Implements BRAIM_VERIFY_SUGGEST_SPEC §3.3 — surface concrete candidate
    /// PRIMARY-typed sources that would promote `statement_id` toward a higher
    /// verification status. See spec §3.4 for the edge-case responses
    /// (concept target, already-proven_strong, invalidated).
    pub fn verify_suggest(&self, statement_id: u32) -> Result<VerifySuggestion, String> {
        let statement = self.state.nodes.get(&statement_id)
            .ok_or(format!("Error: Statement ID {} not found", statement_id))?;

        // Header info — shared across all return paths.
        let primary_types: HashSet<&'static str> = statement.sources.iter()
            .filter_map(|s| {
                let (t, _) = Self::parse_source(s);
                if t.tier() == "PRIMARY" { Some(primary_type_name(&t)) } else { None }
            })
            .collect();
        let primary_count = statement.sources.iter().filter(|s| {
            let (t, _) = Self::parse_source(s);
            t.tier() == "PRIMARY"
        }).count();

        let mut header = VerifySuggestion {
            statement_id,
            label: statement.label.clone(),
            status_label: statement.verification_status.label().to_string(),
            primary_count,
            distinct_primary_types: primary_types.len(),
            message: None,
            candidates: Vec::new(),
            already_attached_types: {
                let mut v: Vec<String> = primary_types.iter().map(|s| s.to_string()).collect();
                v.sort();
                v
            },
            missing_primary_types: {
                let mut v: Vec<String> = ALL_PRIMARY_TYPES.iter()
                    .filter(|t| !primary_types.contains(*t))
                    .map(|s| s.to_string())
                    .collect();
                v.sort();
                v
            },
        };

        // Edge cases per §3.4.
        if !statement.node_type.is_statement_family() {
            return Err(format!("Error: Node ID {} is a concept (atomic/compound); verify-suggest applies to statements only.", statement_id));
        }
        match statement.verification_status {
            VerificationStatus::Invalid => {
                header.message = Some(
                    "Statement is invalidated. Cannot upgrade. Use `statement add` to create a replacement.".to_string()
                );
                return Ok(header);
            }
            VerificationStatus::ProvenStrong => {
                header.message = Some("Statement already at maximum verification.".to_string());
                return Ok(header);
            }
            _ => {}
        }
        if header.missing_primary_types.is_empty() {
            header.message = Some("Statement has all available PRIMARY source types.".to_string());
            return Ok(header);
        }

        // Extract label terms and file-path-like substrings.
        let target_terms = extract_label_terms(&statement.label);
        let target_paths = extract_label_paths(&statement.label);

        // Find related verified facts in the same domain sharing ≥2 terms.
        let mut candidates: Vec<SuggestedSource> = Vec::new();
        let mut related_facts: Vec<(u32, &Node)> = self.state.nodes.iter()
            .filter(|(other_id, other_node)| {
                // §3.3 specifies shared_terms >= 2 but §5 VS2 description requires
                // matching on a single shared term. Use >= 1 to match the test
                // expectations; this is the more useful threshold for short labels.
                **other_id != statement_id
                    && matches!(other_node.node_type, NodeType::Fact)
                    && other_node.domains.iter().any(|d| statement.domains.contains(d))
                    && shared_term_count(&target_terms, &extract_label_terms(&other_node.label)) >= 1
            })
            .map(|(id, n)| (*id, n))
            .collect();
        related_facts.sort_by_key(|(id, _)| *id);

        for (rid, rnode) in &related_facts {
            for src in &rnode.sources {
                let (stype, _) = Self::parse_source(src);
                if stype.tier() != "PRIMARY" {
                    continue;
                }
                candidates.push(SuggestedSource {
                    source: src.clone(),
                    rationale: format!("related fact ID:{} uses {}", rid, src),
                    impact: String::new(),
                    rank: 0,
                });
            }
        }

        // Augment with label-extracted paths.
        for (path, type_prefix) in &target_paths {
            candidates.push(SuggestedSource {
                source: format!("{}:{}", type_prefix, path),
                rationale: format!("label contains path '{}'; suggest {}: source", path, type_prefix),
                impact: String::new(),
                rank: 0,
            });
        }

        // Rank by promotion impact: candidates that introduce a new PRIMARY
        // type rank by the resulting distinct-type count (higher = bigger
        // jump); candidates duplicating an attached type rank 0.
        let existing = &primary_types;
        for c in &mut candidates {
            let c_type = c.source.split(':').next().unwrap_or("");
            if existing.contains(&c_type) {
                c.impact = "no change (type already attached)".to_string();
                c.rank = 0;
            } else {
                let new_count = existing.len() + 1;
                c.impact = predicted_status_label(new_count).to_string();
                c.rank = new_count as u8;
            }
        }

        // Deduplicate by source string, keeping highest-rank rationale.
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut deduped: Vec<SuggestedSource> = Vec::new();
        for c in candidates {
            if let Some(&idx) = seen.get(&c.source) {
                if c.rank > deduped[idx].rank {
                    deduped[idx] = c;
                }
            } else {
                seen.insert(c.source.clone(), deduped.len());
                deduped.push(c);
            }
        }

        deduped.sort_by(|a, b| b.rank.cmp(&a.rank));
        deduped.truncate(10);

        if deduped.is_empty() {
            header.message = Some(
                "No candidates found. Add a `code:`, `doc:`, `schema:`, `config:`, `transcript:`, or `test:` source manually to upgrade verification."
                .to_string()
            );
        } else {
            header.candidates = deduped;
        }

        Ok(header)
    }

    pub fn find_decomposable_atomics(&self, label: &str) -> Vec<(u32, String)> {
        let tokens: Vec<&str> = label.split_whitespace().collect();
        if tokens.len() < 2 {
            return Vec::new();
        }

        let mut matches = Vec::new();
        for token in tokens {
            let lowercase = token.to_lowercase();
            if let Some(ids) = self.state.dictionary.get(&lowercase) {
                for id in ids {
                    if let Some(node) = self.state.nodes.get(id) {
                        if node.node_type == NodeType::Atomic {
                            matches.push((*id, node.label.clone()));
                        }
                    }
                }
            }
        }
        matches
    }

    /// Returns transitive statement-typed dependents of `node_id`
    /// (only statements participate in inheritance cascade — concepts skipped).
    /// Result is ordered by BFS discovery from the root.
    pub fn find_cascade_nodes(&self, node_id: u32) -> Vec<(u32, String)> {
        let mut cascade = Vec::new();
        let mut queue: VecDeque<u32> = VecDeque::from([node_id]);
        let mut visited: HashSet<u32> = HashSet::new();
        visited.insert(node_id);

        while let Some(current_id) = queue.pop_front() {
            let mut direct: Vec<(u32, String)> = self.state.nodes.iter()
                .filter(|(oid, on)| {
                    on.node_type.is_statement_family()
                        && on.depends_on.contains_key(&current_id)
                        && !visited.contains(oid)
                })
                .map(|(oid, on)| (*oid, on.label.clone()))
                .collect();
            direct.sort_by_key(|(id, _)| *id);
            for (dep_id, label) in direct {
                visited.insert(dep_id);
                cascade.push((dep_id, label));
                queue.push_back(dep_id);
            }
        }

        cascade
    }

    /// Invalidate a statement and cascade-invalidate all transitively dependent
    /// statements per BRAIM_DEPENDENCY_INHERITANCE_SPEC §3.3.
    /// Returns the list of cascade-invalidated IDs (excluding the target).
    pub fn invalidate_statement(&mut self, statement_id: u32, reason: &str) -> Result<Vec<u32>, String> {
        {
            let node = self.state.nodes.get(&statement_id)
                .ok_or(format!("Error: Statement ID {} not found", statement_id))?;

            if !node.node_type.is_statement_family() {
                return Err(format!("Error: Node ID {} is not a statement", statement_id));
            }

            if node.sources.contains(&"inferred".to_string()) {
                return Err(format!("Error: Cannot invalidate inferred statement ID {}. Inferred statements are derived relationships.", statement_id));
            }
        }

        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let cascade_ids: Vec<u32> = self.find_cascade_nodes(statement_id)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        {
            let node = self.state.nodes.get_mut(&statement_id).unwrap();
            node.invalid = true;
            node.invalid_reason = Some(reason.to_string());
            node.invalidated_at = Some(now.clone());
            node.verification_status = VerificationStatus::Invalid;
            // Per BRAIM_NODE_TYPE_CLAIM_FACT_SPEC §3.4 — set node_type to InvalidStatement.
            node.node_type = NodeType::InvalidStatement;
        }

        for dep_id in &cascade_ids {
            if let Some(dep_node) = self.state.nodes.get_mut(dep_id) {
                if dep_node.invalid || dep_node.verification_status == VerificationStatus::Invalid {
                    continue;
                }
                dep_node.invalid = true;
                dep_node.invalid_reason = Some(format!("depends_on_invalidated:{}", statement_id));
                dep_node.invalidated_at = Some(now.clone());
                dep_node.verification_status = VerificationStatus::Invalid;
                dep_node.node_type = NodeType::InvalidStatement;
            }
        }

        // because_of consequents above the invalidated cause are flagged for
        // re-investigation but not auto-invalidated (tests.md §13).
        self.flag_because_of_reinvestigation(statement_id);

        self.flush()?;
        Ok(cascade_ids)
    }

    /// Inverse of `invalidate_statement` for a single node: clears the invalid
    /// flags and recomputes verification_status from sources + dependency
    /// inheritance. Dependencies that are themselves invalid are SKIPPED in the
    /// inheritance cap (and returned) rather than re-poisoning the node — this is
    /// what lets a node be revived while a foundational dep stays intentionally
    /// retired; the caller re-anchors those deps with `update-deps`. Does NOT
    /// cascade to dependents: revive explicitly, dependency order outward.
    pub fn revalidate_statement(&mut self, statement_id: u32) -> Result<(VerificationStatus, Vec<u32>), String> {
        {
            let node = self.state.nodes.get(&statement_id)
                .ok_or(format!("Error: Statement ID {} not found", statement_id))?;
            if !node.node_type.is_statement_family() {
                return Err(format!("Error: Node ID {} is not a statement", statement_id));
            }
            let is_invalid = node.invalid || node.verification_status == VerificationStatus::Invalid;
            let is_contested = node.verification_status == VerificationStatus::Contested;
            // A node stuck contested purely by inheritance (a dependency was contested
            // when this node was created, then removed) has no contradiction edge of its
            // own. update_statement_deps refuses to recompute contested nodes, so it can
            // never clear — revalidate recomputes it. A node with a real, unresolved
            // contradiction edge must NOT be touched here; that is the contradiction
            // lifecycle's job (resolve-contradiction).
            let has_active_contradiction = self.state.contradicts.iter().any(|e| {
                !e.resolved && (e.from == statement_id || e.to == statement_id)
            });
            if is_contested && has_active_contradiction {
                return Err(format!(
                    "Error: Statement ID {} is contested by an active contradiction edge — resolve it with 'statement resolve-contradiction', not revalidate",
                    statement_id
                ));
            }
            if !is_invalid && !is_contested {
                return Err(format!("Error: Statement ID {} is neither invalid nor contested — nothing to revalidate", statement_id));
            }
        }

        {
            let node = self.state.nodes.get_mut(&statement_id).unwrap();
            node.invalid = false;
            node.invalid_reason = None;
            node.invalidated_at = None;
        }

        let (new_status, invalid_deps) = {
            let stmt = self.state.nodes.get(&statement_id).unwrap();
            let entity_types = self.fetch_source_entity_types(&stmt.source_ids);
            let source_derived =
                Self::calculate_verification_status_from_all_sources(&stmt.sources, &entity_types);
            let mut cap: Option<u8> = None;
            let mut invalid_deps: Vec<u32> = Vec::new();
            for dep_id in stmt.depends_on.keys() {
                if let Some(dep) = self.state.nodes.get(dep_id) {
                    if !dep.node_type.is_statement_family() {
                        continue;
                    }
                    if dep.invalid || dep.verification_status == VerificationStatus::Invalid {
                        invalid_deps.push(*dep_id);
                        continue;
                    }
                    let r = dep.verification_status.rank();
                    cap = Some(cap.map_or(r, |p: u8| p.min(r)));
                }
            }
            let status = match cap {
                Some(c) if source_derived.rank() > c => VerificationStatus::from_rank(c),
                _ => source_derived,
            };
            (status, invalid_deps)
        };

        {
            let stmt = self.state.nodes.get_mut(&statement_id).unwrap();
            stmt.verification_status = new_status;
            stmt.node_type = NodeType::from_verification_status(new_status);
        }

        self.flush()?;
        Ok((new_status, invalid_deps))
    }

    pub fn verify_statement(&mut self, statement_id: u32, domain: &str, note: Option<String>) -> Result<(), String> {
        let node = self.state.nodes.get_mut(&statement_id)
            .ok_or(format!("Error: Statement ID {} not found", statement_id))?;

        if !node.node_type.is_statement_family() {
            return Err(format!("Error: Node ID {} is not a statement", statement_id));
        }

        // Prevent verification of inferred statements
        if node.sources.contains(&"inferred".to_string()) {
            return Err(format!("Error: Cannot verify inferred statement ID {}. Inferred statements are derived, not verified.", statement_id));
        }

        node.verified_by.insert(domain.to_string(), note);

        let num_verifications = node.verified_by.len();
        node.verification_status = match num_verifications {
            0 | 1 => VerificationStatus::Unproven,
            2 => VerificationStatus::Partial,
            _ => VerificationStatus::Proven,
        };

        self.flush()?;
        Ok(())
    }

    /// `min_status` is the verification floor for admission (`None` = admit all).
    /// `full` = full-fidelity (trusted self-import, braim ID:229/234): preserves
    /// verification state instead of resetting it, imports source entities and
    /// remaps statement source_ids, carries because_of/contradicts edges, and
    /// unions a duplicate's sources into the dedup target so corroboration
    /// accumulates (ID:185/190) instead of being discarded (ID:179).
    pub fn import_graph(
        &mut self,
        source_path: &str,
        filter_domain: Option<&str>,
        min_status: Option<VerificationStatus>,
        domain_mappings: HashMap<String, String>,
        full: bool,
    ) -> Result<ImportManifest, String> {
        let path = PathBuf::from(source_path);
        let source_state: GraphState = if path.is_dir() && path.join("domains").is_dir() {
            // Sharded source dir: merge its shards exactly as load does. Not our
            // lock, so use the reader path.
            Self::load_sharded(&path, false)?
        } else {
            let file = if path.is_dir() { path.join("current.json") } else { path };
            let source_content = fs::read_to_string(&file)
                .map_err(|e| format!("Error reading source file: {}", e))?;
            serde_json::from_str(&source_content)
                .map_err(|e| format!("Error parsing source graph: {}", e))?
        };
        self.import_state(source_state, filter_domain, min_status, domain_mappings, full)
    }

    /// Core of import/export: merge an in-memory source state into this graph.
    /// `braim export` calls this directly with the working graph's state — the
    /// contribute flow and the consume flow are one code path (braim ID:232/240).
    pub fn import_state(
        &mut self,
        mut source_state: GraphState,
        filter_domain: Option<&str>,
        min_status: Option<VerificationStatus>,
        domain_mappings: HashMap<String, String>,
        full: bool,
    ) -> Result<ImportManifest, String> {
        // Apply domain mappings to source nodes
        for node in source_state.nodes.values_mut() {
            let mut remapped_domains = Vec::new();
            for domain in &node.domains {
                let remapped = domain_mappings.get(domain).cloned().unwrap_or_else(|| domain.clone());
                remapped_domains.push(remapped);
            }
            node.domains = remapped_domains;
        }

        let mut id_mappings: HashMap<u32, u32> = HashMap::new();
        let mut duplicates: Vec<DuplicateRecord> = Vec::new();
        let mut imported_count = 0;
        let mut deduplicated_count = 0;
        let mut skipped_count = 0;
        let mut sources_imported = 0;
        let mut because_of_imported = 0;
        let mut contradicts_imported = 0;
        let mut sources_unioned = 0;

        // Domain filtering is closure-aware: the admitted set is the domain's own
        // nodes PLUS everything they transitively depend on (concepts, statements,
        // attached source entities), regardless of those dependencies' domains.
        // Bare same-domain filtering silently dropped every statement with a
        // cross-domain dependency (braim ID:180); a published domain slice must be
        // self-contained — the vendored-closure pack decision (ID:220).
        let domain_admitted: Option<HashSet<u32>> = filter_domain.map(|d| {
            let mut admitted: HashSet<u32> = HashSet::new();
            let mut frontier: Vec<u32> = source_state.nodes.values()
                .filter(|n| n.domains.contains(&d.to_string()))
                .map(|n| n.id)
                .collect();
            while let Some(id) = frontier.pop() {
                if !admitted.insert(id) {
                    continue;
                }
                if let Some(n) = source_state.nodes.get(&id) {
                    frontier.extend(n.depends_on.keys().copied());
                    frontier.extend(n.source_ids.iter().copied());
                }
            }
            admitted
        });
        let in_domain_scope = |node: &Node| -> bool {
            domain_admitted.as_ref().map_or(true, |adm| adm.contains(&node.id))
        };

        // Verification floor for admission. A rank comparison, not equality:
        // `!= Proven` silently dropped proven_strong nodes. `None` admits
        // everything. Export defaults to a Partial floor so a statement with one
        // PRIMARY source can publish and corroborate (braim ID:253); import
        // --only-proven passes Proven.
        let meets_floor =
            |s: VerificationStatus| min_status.map_or(true, |m| s.rank() >= m.rank());

        // Collect nodes by type for ordered processing; sorted by id so import
        // results don't depend on HashMap iteration order.
        let mut source_entities = Vec::new();
        let mut atomics = Vec::new();
        let mut compounds = Vec::new();
        let mut statements = Vec::new();

        for (_, node) in &source_state.nodes {
            match node.node_type {
                NodeType::Atomic => atomics.push(node.clone()),
                NodeType::Compound => compounds.push(node.clone()),
                NodeType::Statement
                | NodeType::Claim
                | NodeType::Fact
                | NodeType::InvalidStatement
                | NodeType::ContestedStatement => statements.push(node.clone()),
                // Source entities are carried only in full-fidelity mode; the
                // legacy path drops them (statement source_ids would dangle).
                NodeType::Source => if full { source_entities.push(node.clone()) },
            }
        }
        for list in [&mut source_entities, &mut atomics, &mut compounds, &mut statements] {
            list.sort_by_key(|n| n.id);
        }

        // --only-proven admits statements at proven rank or above PLUS the concept
        // closure they depend on. Concepts are vocabulary — they rarely reach
        // proven — so gating them by status starves every proven statement of its
        // dependencies and imports nothing. Statement-typed dependencies need no
        // exemption: MIN-inheritance already guarantees a proven statement's
        // statement deps are themselves at proven rank.
        let needed_concepts: HashSet<u32> = if min_status.is_some() {
            let mut needed: HashSet<u32> = HashSet::new();
            let mut frontier: Vec<u32> = statements.iter()
                .filter(|s| meets_floor(s.verification_status))
                .flat_map(|s| s.depends_on.keys().copied())
                .collect();
            while let Some(id) = frontier.pop() {
                if !needed.insert(id) {
                    continue;
                }
                if let Some(n) = source_state.nodes.get(&id) {
                    if !n.node_type.is_statement_family() {
                        frontier.extend(n.depends_on.keys().copied());
                    }
                }
            }
            needed
        } else {
            HashSet::new()
        };
        let concept_admitted = |node: &Node| -> bool {
            min_status.is_none() || meets_floor(node.verification_status) || needed_concepts.contains(&node.id)
        };

        // Process source entities first: statements remap source_ids against them.
        // Dedup key: same label (case-insensitive) and location. Under a domain
        // filter, only entities referenced by admitted statements cross.
        for node in source_entities {
            if !in_domain_scope(&node) {
                skipped_count += 1;
                continue;
            }
            let found = self.state.nodes.iter().find(|(_, t)| {
                t.node_type == NodeType::Source
                    && t.label.to_lowercase() == node.label.to_lowercase()
                    && t.location == node.location
            }).map(|(id, _)| *id);

            if let Some(target_id) = found {
                id_mappings.insert(node.id, target_id);
                deduplicated_count += 1;
            } else {
                let new_id = self.state.next_id;
                id_mappings.insert(node.id, new_id);
                let mut new_node = node.clone();
                new_node.id = new_id;
                self.state.nodes.insert(new_id, new_node);
                self.state.next_id += 1;
                self.state.dictionary.entry(node.label.to_lowercase()).or_insert_with(Vec::new).push(new_id);
                sources_imported += 1;
            }
        }

        // Process atomics first
        for node in atomics {
            if !in_domain_scope(&node) {
                skipped_count += 1;
                continue;
            }

            if !concept_admitted(&node) {
                skipped_count += 1;
                continue;
            }

            let key = (node.label.to_lowercase(), node.domains.clone());

            // Check for existing atomic with same name and domain
            let found = self.state.nodes.iter().find(|(_, t)| {
                t.node_type == NodeType::Atomic
                    && t.label.to_lowercase() == key.0
                    && t.domains == key.1
            }).map(|(id, t)| (*id, t.label.clone()));

            if let Some((target_id, target_label)) = found {
                id_mappings.insert(node.id, target_id);
                duplicates.push(DuplicateRecord {
                    source_id: node.id,
                    target_id,
                    target_label,
                    reason: "same name and domain".to_string(),
                });
                deduplicated_count += 1;
                if full && self.union_sources_into(target_id, &node, &id_mappings) {
                    sources_unioned += 1;
                }
            } else {
                let new_id = self.state.next_id;
                id_mappings.insert(node.id, new_id);

                let mut new_node = node.clone();
                new_node.id = new_id;
                if !full {
                    new_node.verification_status = VerificationStatus::Unproven;
                    new_node.verified_by = HashMap::new();
                }

                self.state.nodes.insert(new_id, new_node.clone());
                self.state.next_id += 1;
                imported_count += 1;

                // Update dictionary
                let lower_label = node.label.to_lowercase();
                self.state.dictionary.entry(lower_label.clone()).or_insert_with(Vec::new).push(new_id);
                if let Some(short) = Self::atomic_short_name_key(&node.label) {
                    if short != lower_label {
                        self.state.dictionary.entry(short).or_insert_with(Vec::new).push(new_id);
                    }
                }
            }
        }

        // Process compounds
        for node in compounds {
            if !in_domain_scope(&node) {
                skipped_count += 1;
                continue;
            }

            if !concept_admitted(&node) {
                skipped_count += 1;
                continue;
            }

            // Check if all dependencies were imported
            let mut all_deps_imported = true;
            for dep_id in node.depends_on.keys() {
                if !id_mappings.contains_key(dep_id) {
                    all_deps_imported = false;
                    skipped_count += 1;
                    break;
                }
            }
            if !all_deps_imported {
                continue;
            }

            let key = (node.label.to_lowercase(), node.domains.clone());

            // Check for existing compound with same name and domain
            let found = self.state.nodes.iter().find(|(_, t)| {
                t.node_type == NodeType::Compound
                    && t.label.to_lowercase() == key.0
                    && t.domains == key.1
            }).map(|(id, t)| (*id, t.label.clone()));

            if let Some((target_id, target_label)) = found {
                id_mappings.insert(node.id, target_id);
                duplicates.push(DuplicateRecord {
                    source_id: node.id,
                    target_id,
                    target_label,
                    reason: "same name and domain".to_string(),
                });
                deduplicated_count += 1;
                if full && self.union_sources_into(target_id, &node, &id_mappings) {
                    sources_unioned += 1;
                }
            } else {
                let new_id = self.state.next_id;

                // Remap dependency IDs
                let mut new_depends_on = HashMap::new();
                for (dep_id, weight) in &node.depends_on {
                    let mapped_id = id_mappings
                        .get(dep_id)
                        .ok_or_else(|| format!("Error: Dependency ID {} not found in mappings", dep_id))?;
                    new_depends_on.insert(*mapped_id, *weight);
                }

                id_mappings.insert(node.id, new_id);

                let mut new_node = node.clone();
                new_node.id = new_id;
                new_node.depends_on = new_depends_on;
                if !full {
                    new_node.verification_status = VerificationStatus::Unproven;
                    new_node.verified_by = HashMap::new();
                }

                self.state.nodes.insert(new_id, new_node.clone());
                self.state.next_id += 1;
                imported_count += 1;

                // Update dictionary
                let lower_label = node.label.to_lowercase();
                self.state.dictionary.entry(lower_label).or_insert_with(Vec::new).push(new_id);
            }
        }

        // Process statements to a fixpoint: a statement may depend on another
        // statement, and a single id-sorted pass would skip any dependent that
        // precedes its dependency. Loop until a pass imports nothing new;
        // whatever remains genuinely has an unimportable dependency.
        let mut pending = statements;
        loop {
            let mut next_pending = Vec::new();
            let mut progressed = false;

            for node in pending {
                if !in_domain_scope(&node) {
                    skipped_count += 1;
                    continue;
                }

                if !meets_floor(node.verification_status) {
                    skipped_count += 1;
                    continue;
                }

                // Defer if any dependency is not (yet) imported.
                if node.depends_on.keys().any(|d| !id_mappings.contains_key(d)) {
                    next_pending.push(node);
                    continue;
                }

                // Remap dependency IDs for comparison
                let mut remapped_deps: Vec<u32> = node.depends_on.keys()
                    .filter_map(|d| id_mappings.get(d).copied())
                    .collect();
                remapped_deps.sort();

                // Check for existing statement with same text and remapped dependencies
                let found = self.state.nodes.iter().find(|(_, t)| {
                    if !t.node_type.is_statement_family() || t.label != node.label {
                        return false;
                    }
                    let mut target_deps: Vec<u32> = t.depends_on.keys().copied().collect();
                    target_deps.sort();
                    target_deps == remapped_deps
                }).map(|(id, t)| (*id, t.label.clone()));

                if let Some((target_id, target_label)) = found {
                    id_mappings.insert(node.id, target_id);
                    duplicates.push(DuplicateRecord {
                        source_id: node.id,
                        target_id,
                        target_label,
                        reason: "same text and dependencies".to_string(),
                    });
                    deduplicated_count += 1;
                    if full && self.union_sources_into(target_id, &node, &id_mappings) {
                        sources_unioned += 1;
                        self.recompute_statement_status(target_id);
                    }
                } else {
                    let new_id = self.state.next_id;

                    // Remap dependency IDs
                    let mut new_depends_on = HashMap::new();
                    for (dep_id, weight) in &node.depends_on {
                        let mapped_id = id_mappings
                            .get(dep_id)
                            .ok_or_else(|| format!("Error: Dependency ID {} not found in mappings", dep_id))?;
                        new_depends_on.insert(*mapped_id, *weight);
                    }

                    id_mappings.insert(node.id, new_id);

                    let mut new_node = node.clone();
                    new_node.id = new_id;
                    new_node.depends_on = new_depends_on;
                    if full {
                        // Remap source-entity references; drop any that were not
                        // imported (filtered upstream) rather than dangling.
                        new_node.source_ids = node.source_ids.iter()
                            .filter_map(|sid| id_mappings.get(sid).copied())
                            .collect();
                    } else {
                        new_node.verification_status = VerificationStatus::Unproven;
                        new_node.verified_by = HashMap::new();
                        new_node.source_ids = Vec::new();
                    }

                    self.state.nodes.insert(new_id, new_node.clone());
                    self.state.next_id += 1;
                    imported_count += 1;
                }
                progressed = true;
            }

            if next_pending.is_empty() || !progressed {
                skipped_count += next_pending.len();
                break;
            }
            pending = next_pending;
        }

        // Carry relationship edges (full-fidelity only). An edge crosses only if
        // both endpoints were imported or deduplicated; already-present edges
        // (same remapped endpoints) are not duplicated.
        if full {
            for e in &source_state.because_of {
                if let (Some(&f), Some(&t)) = (id_mappings.get(&e.from), id_mappings.get(&e.to)) {
                    if !self.state.because_of.iter().any(|x| x.from == f && x.to == t) {
                        let mut ne = e.clone();
                        ne.from = f;
                        ne.to = t;
                        self.state.because_of.push(ne);
                        because_of_imported += 1;
                    }
                }
            }
            for e in &source_state.contradicts {
                if let (Some(&f), Some(&t)) = (id_mappings.get(&e.from), id_mappings.get(&e.to)) {
                    if !self.state.contradicts.iter().any(|x| x.from == f && x.to == t) {
                        let mut ne = e.clone();
                        ne.from = f;
                        ne.to = t;
                        ne.source_id = e.source_id.and_then(|s| id_mappings.get(&s).copied());
                        ne.resolution_source = e.resolution_source.and_then(|s| id_mappings.get(&s).copied());
                        ne.resolution_winner = e.resolution_winner.and_then(|s| id_mappings.get(&s).copied());
                        self.state.contradicts.push(ne);
                        contradicts_imported += 1;
                    }
                }
            }
        }

        self.flush()?;

        Ok(ImportManifest {
            sources_imported,
            because_of_imported,
            contradicts_imported,
            sources_unioned,
            imported_count,
            deduplicated_count,
            skipped_count,
            id_mappings,
            duplicates,
        })
    }

    /// Union an incoming duplicate's evidence into its dedup target: source
    /// strings, remapped source-entity ids, and verified_by entries the target
    /// lacks. Returns true if anything was added. Same-source repetition stacks
    /// as corroboration without inventing type diversity (braim ID:185/190) —
    /// the promotion math still counts distinct PRIMARY types only.
    fn union_sources_into(&mut self, target_id: u32, incoming: &Node, id_mappings: &HashMap<u32, u32>) -> bool {
        let remapped_sids: Vec<u32> = incoming.source_ids.iter()
            .filter_map(|sid| id_mappings.get(sid).copied())
            .collect();
        let Some(target) = self.state.nodes.get_mut(&target_id) else { return false };
        let mut changed = false;
        for s in &incoming.sources {
            if !target.sources.contains(s) {
                target.sources.push(s.clone());
                changed = true;
            }
        }
        for sid in remapped_sids {
            if !target.source_ids.contains(&sid) {
                target.source_ids.push(sid);
                changed = true;
            }
        }
        for (k, v) in &incoming.verified_by {
            if !target.verified_by.contains_key(k) {
                target.verified_by.insert(k.clone(), v.clone());
                changed = true;
            }
        }
        changed
    }

    /// Recompute a statement's verification from its (possibly just-unioned)
    /// sources plus dependency inheritance. Contested and invalid statements are
    /// left alone: those resolve only through their own lifecycles
    /// (resolve-contradiction / revalidate), never as an import side effect.
    fn recompute_statement_status(&mut self, statement_id: u32) {
        let Some(stmt) = self.state.nodes.get(&statement_id) else { return };
        if !stmt.node_type.is_statement_family()
            || stmt.invalid
            || matches!(stmt.verification_status, VerificationStatus::Invalid | VerificationStatus::Contested)
        {
            return;
        }
        let entity_types = self.fetch_source_entity_types(&stmt.source_ids);
        let source_derived =
            Self::calculate_verification_status_from_all_sources(&stmt.sources, &entity_types);
        let mut cap: Option<u8> = None;
        for dep_id in stmt.depends_on.keys() {
            if let Some(dep) = self.state.nodes.get(dep_id) {
                if !dep.node_type.is_statement_family() {
                    continue;
                }
                let r = dep.verification_status.rank();
                cap = Some(cap.map_or(r, |p: u8| p.min(r)));
            }
        }
        let new_status = match cap {
            Some(c) if source_derived.rank() > c => VerificationStatus::from_rank(c),
            _ => source_derived,
        };
        let stmt = self.state.nodes.get_mut(&statement_id).unwrap();
        stmt.verification_status = new_status;
        stmt.node_type = NodeType::from_verification_status(new_status);
    }
}

#[cfg(test)]
mod tests {
    use super::VerificationStatus as VS;

    #[test]
    fn from_rank_is_inverse_of_rank() {
        for s in [
            VS::Invalid,
            VS::Unproven,
            VS::Contested,
            VS::Partial,
            VS::Proven,
            VS::ProvenStrong,
        ] {
            assert_eq!(VS::from_rank(s.rank()), s, "from_rank(rank()) must round-trip for {s:?}");
        }
    }
}

#[cfg(test)]
mod error_tests {
    use super::GraphError;

    #[test]
    fn display_matches_legacy_messages() {
        assert_eq!(
            GraphError::EmptyDomainsSources.to_string(),
            "Error: domains and sources must not be empty"
        );
        assert_eq!(
            GraphError::StatementNoDependency.to_string(),
            "Error: Statement must have at least 1 dependency"
        );
        assert_eq!(
            GraphError::WeightsNotOne(1.4).to_string(),
            "Error: Weights must sum to 1.0 — got 1.4000"
        );
        assert_eq!(
            GraphError::DependencyNotFound(99).to_string(),
            "Error: Dependency ID 99 does not exist"
        );
        assert_eq!(
            GraphError::CompoundNoDependencies.to_string(),
            "Error: Compound concept must have dependencies"
        );
        assert_eq!(
            GraphError::ConceptExists {
                term: "X".to_string(),
                domains: vec!["d".to_string()],
                id: 3
            }
            .to_string(),
            "Error: Concept 'X' already exists in domain [\"d\"] (ID 3)"
        );
    }

    #[test]
    fn errors_are_matchable_not_just_strings() {
        let e = GraphError::DependencyNotFound(7);
        assert!(matches!(e, GraphError::DependencyNotFound(7)));
    }

    #[test]
    fn string_conversion_preserves_message() {
        let e = GraphError::EmptyDomainsSources;
        let s: String = e.clone().into();
        assert_eq!(s, e.to_string());
        assert_eq!(GraphError::from("boom".to_string()), GraphError::Other("boom".to_string()));
    }
}

#[cfg(test)]
mod because_of_tests {
    use super::*;

    fn temp_braim(tag: &str) -> Braim {
        let dir = std::env::temp_dir().join(format!("braim_bo_test_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Braim::new(dir.to_str().unwrap()).unwrap()
    }

    /// Two atomics (IDs 1,2) then `n` proven statements depending on them.
    /// Returns the statement IDs in creation order.
    fn seed(b: &mut Braim, n: usize) -> Vec<u32> {
        b.add_concept("X: x", vec!["d".into()], vec!["doc:x.md".into()], None).unwrap();
        b.add_concept("Y: y", vec!["d".into()], vec!["doc:y.md".into()], None).unwrap();
        let mut ids = Vec::new();
        for i in 0..n {
            let deps = HashMap::from([(1u32, 0.5), (2u32, 0.5)]);
            let id = b.add_statement(
                &format!("S{}: statement {}", i, i),
                vec!["d".into(), "d".into()],
                vec!["code:s.rs".into(), "doc:s.md".into()],
                deps,
                true,
            ).unwrap();
            ids.push(id);
        }
        ids
    }

    #[test]
    fn rejects_concept_endpoint() {
        let mut b = temp_braim("concept");
        let ids = seed(&mut b, 1);
        // concept ID 1 is not a statement
        let err = b.why_add(ids[0], 1, None).unwrap_err();
        assert!(err.contains("only statement endpoints"), "{err}");
    }

    #[test]
    fn enforces_single_cardinality() {
        let mut b = temp_braim("card");
        let ids = seed(&mut b, 3);
        b.why_add(ids[0], ids[1], None).unwrap();
        let err = b.why_add(ids[0], ids[2], None).unwrap_err();
        assert!(err.contains("already has a cause"), "{err}");
    }

    #[test]
    fn detects_cycle() {
        let mut b = temp_braim("cycle");
        let ids = seed(&mut b, 3);
        b.why_add(ids[0], ids[1], None).unwrap();
        b.why_add(ids[1], ids[2], None).unwrap();
        let err = b.why_add(ids[2], ids[0], None).unwrap_err();
        assert!(err.starts_with("Error: cycle detected:"), "{err}");
    }

    #[test]
    fn depth_warns_then_rejects() {
        let mut b = temp_braim("depth");
        let ids = seed(&mut b, 12);
        // Chain ids[0]->ids[1]->...; depth 7 (7th link) warns, depth 11 rejects.
        for i in 0..6 {
            assert!(b.why_add(ids[i], ids[i + 1], None).unwrap().is_none());
        }
        // 7th link → depth 7 → warning present
        assert!(b.why_add(ids[6], ids[7], None).unwrap().is_some());
        for i in 7..10 {
            assert!(b.why_add(ids[i], ids[i + 1], None).unwrap().is_some());
        }
        // 11th link → depth 11 → reject
        let err = b.why_add(ids[10], ids[11], None).unwrap_err();
        assert!(err.contains("chain depth limit reached"), "{err}");
    }

    #[test]
    fn causal_status_inherits_and_promotes() {
        let mut b = temp_braim("inherit");
        let ids = seed(&mut b, 2); // both proven
        b.why_add(ids[0], ids[1], None).unwrap();
        // both proven, untested → partial
        let chain = b.why_chain(ids[0]).unwrap();
        assert_eq!(chain.steps[0].causal_status, Some(VerificationStatus::Partial));
        assert!(chain.root_verified);
        // inverse test pass → proven
        b.why_test(ids[0], true, Some("test:ablation.txt".into())).unwrap();
        let chain = b.why_chain(ids[0]).unwrap();
        assert_eq!(chain.steps[0].causal_status, Some(VerificationStatus::Proven));
        assert!(chain.steps[0].edge_tested);
    }

    #[test]
    fn inverse_test_fail_refutes_link_not_statements() {
        let mut b = temp_braim("fail");
        let ids = seed(&mut b, 2);
        b.why_add(ids[0], ids[1], None).unwrap();
        b.why_test(ids[0], false, None).unwrap();
        // endpoint statements stay proven
        assert_eq!(b.state.nodes[&ids[0]].verification_status, VerificationStatus::Proven);
        assert_eq!(b.state.nodes[&ids[1]].verification_status, VerificationStatus::Proven);
        // edge is invalid
        let chain = b.why_chain(ids[0]).unwrap();
        assert!(chain.steps[0].edge_invalid);
        assert_eq!(chain.steps[0].causal_status, Some(VerificationStatus::Invalid));
    }

    #[test]
    fn invalidating_cause_flags_ancestors_without_invalidating() {
        let mut b = temp_braim("cascade");
        let ids = seed(&mut b, 4); // A B C D
        b.why_add(ids[0], ids[1], None).unwrap();
        b.why_add(ids[1], ids[2], None).unwrap();
        b.why_add(ids[2], ids[3], None).unwrap();
        b.invalidate_statement(ids[3], "disproven").unwrap();
        for anc in &ids[0..3] {
            let n = &b.state.nodes[anc];
            assert_ne!(n.verification_status, VerificationStatus::Invalid, "ancestor {anc} wrongly invalidated");
            assert!(n.metadata.contains_key("because_of_reinvestigate"), "ancestor {anc} not flagged");
        }
    }

    #[test]
    fn traversal_isolated_from_depends_on() {
        let mut b = temp_braim("iso");
        let ids = seed(&mut b, 2);
        b.why_add(ids[0], ids[1], None).unwrap();
        // why follows because_of only: chain is exactly [consequent, cause]
        let chain = b.why_chain(ids[0]).unwrap();
        let walked: Vec<u32> = chain.steps.iter().map(|s| s.id).collect();
        assert_eq!(walked, vec![ids[0], ids[1]]);
    }

    #[test]
    fn why_remove_enables_reassignment() {
        let mut b = temp_braim("remove");
        let ids = seed(&mut b, 3); // A B C
        b.why_add(ids[0], ids[1], None).unwrap();
        // cardinality blocks re-pointing while the edge exists
        assert!(b.why_add(ids[0], ids[2], None).is_err());
        // remove the current cause, then reassign to a different one
        let outcome = b.why_remove(ids[0]).unwrap();
        assert_eq!(outcome.cause, ids[1]);
        assert!(!outcome.was_invalid);
        b.why_add(ids[0], ids[2], None).unwrap();
        let chain = b.why_chain(ids[0]).unwrap();
        assert_eq!(chain.steps.iter().map(|s| s.id).collect::<Vec<_>>(), vec![ids[0], ids[2]]);
    }

    #[test]
    fn audit_surfaces_because_of_findings() {
        let mut b = temp_braim("audit_bo");
        let ids = seed(&mut b, 4); // A B C D (all proven via code+doc seed sources)
        // chain A → B → C → D
        b.why_add(ids[0], ids[1], None).unwrap();
        b.why_add(ids[1], ids[2], None).unwrap();
        b.why_add(ids[2], ids[3], None).unwrap();

        // baseline: all three links untested, root D proven (not unverified)
        let r = b.audit();
        assert_eq!(r.untested_causal_links.len(), 3);
        assert!(r.refuted_causal_links.is_empty());
        assert!(r.reinvestigate_flagged.is_empty());
        assert!(r.unverified_roots.is_empty(), "proven root D must not be flagged");

        // pass an inverse test on A→B → one fewer untested link
        b.why_test(ids[0], true, Some("test:t.txt".into())).unwrap();
        // invalidate root D first → flag walks up the still-active chain (A,B,C)
        b.invalidate_statement(ids[3], "disproven").unwrap();
        // then fail the inverse test on B→C → one refuted link
        b.why_test(ids[1], false, None).unwrap();

        let r = b.audit();
        // refuted: B→C
        assert_eq!(r.refuted_causal_links.len(), 1);
        assert_eq!((r.refuted_causal_links[0].from, r.refuted_causal_links[0].to), (ids[1], ids[2]));
        // untested active links remaining: only C→D (A→B passed, B→C now refuted/inactive)
        assert_eq!(r.untested_causal_links.len(), 1);
        assert_eq!((r.untested_causal_links[0].from, r.untested_causal_links[0].to), (ids[2], ids[3]));
        // reinvestigate flags on A, B, C (ancestors of D)
        let flagged: Vec<u32> = r.reinvestigate_flagged.iter().map(|n| n.id).collect();
        assert_eq!(flagged, vec![ids[0], ids[1], ids[2]]);
        // unverified root: D is now invalid (< proven) and still the cause end of C→D
        assert!(r.unverified_roots.iter().any(|n| n.id == ids[3]));
    }

    #[test]
    fn perspective_traverses_because_of_between_statements() {
        let mut b = temp_braim("persp_bo");
        let ids = seed(&mut b, 3); // statements A,B,C ; concepts 1,2
        // chain A → B → C (A consequent, C root cause)
        b.why_add(ids[0], ids[1], None).unwrap();
        b.why_add(ids[1], ids[2], None).unwrap();
        // proximity from the cause (C) reaches the consequent (A) via because_of,
        // running cause → consequent (mirrors dependency → dependent).
        let paths = b.proximity(&ids[2].to_string(), &ids[0].to_string()).unwrap();
        assert!(!paths.is_empty(), "because_of chain should be traversable cause → consequent");
        assert_eq!(paths[0].path, vec![ids[2], ids[1], ids[0]]);
        // a refuted link is not traversed
        b.why_test(ids[1], false, None).unwrap(); // refute B → C
        let broken = b.proximity(&ids[2].to_string(), &ids[0].to_string()).unwrap();
        assert!(broken.is_empty(), "refuted causal link must not be traversed");
    }

    #[test]
    fn why_remove_clears_refuted_edge() {
        let mut b = temp_braim("remove_refuted");
        let ids = seed(&mut b, 2);
        b.why_add(ids[0], ids[1], None).unwrap();
        b.why_test(ids[0], false, None).unwrap(); // refute the link
        let outcome = b.why_remove(ids[0]).unwrap();
        assert!(outcome.was_invalid);
        // no outgoing edge remains
        assert!(b.why_remove(ids[0]).is_err());
    }
}

#[cfg(test)]
mod defect_tests {
    use super::*;

    fn temp_braim(name: &str) -> Braim {
        let dir = std::env::temp_dir().join(format!("braim_defect_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Braim::new(dir.to_str().unwrap()).unwrap()
    }

    fn two_concepts_and_claim(b: &mut Braim) -> (u32, u32, u32) {
        let a = b.add_concept("Alpha: first concept", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second concept", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let mut deps = HashMap::new();
        deps.insert(a, 0.6);
        deps.insert(c, 0.4);
        let s = b.add_statement("alpha relates to beta", vec!["t".into()], vec!["narrative:claim".into()], deps, true).unwrap();
        (a, c, s)
    }

    #[test]
    fn attached_source_entity_is_not_an_orphan() {
        let mut b = temp_braim("orphan_source");
        let (_, _, s) = two_concepts_and_claim(&mut b);
        let src = b.add_source("evidence file", "code", Some("code:x.rs:1".into()), None).unwrap();
        b.add_source_to_statement(s, src).unwrap();
        let report = b.audit();
        assert!(
            !report.orphans.iter().any(|n| n.id == src),
            "attached source entity must not be reported as orphan"
        );
    }

    #[test]
    fn unattached_source_entity_still_orphan() {
        let mut b = temp_braim("orphan_source_neg");
        let src = b.add_source("dangling file", "code", Some("code:y.rs:1".into()), None).unwrap();
        let report = b.audit();
        assert!(report.orphans.iter().any(|n| n.id == src));
    }

    #[test]
    fn gap_clears_on_add_source_promotion() {
        let mut b = temp_braim("gap_promotion");
        let (a, c, s) = two_concepts_and_claim(&mut b);
        // Gap registered AFTER the statement exists (claims don't carry paths).
        let _ = b.perspective("Alpha", "Beta");
        assert!(
            b.state.gaps.iter().any(|g| (g.concept_a == a && g.concept_b == c) || (g.concept_a == c && g.concept_b == a)),
            "precondition: gap must be registered"
        );
        let src = b.add_source("evidence file", "code", Some("code:x.rs:1".into()), None).unwrap();
        b.add_source_to_statement(s, src).unwrap();
        assert!(
            !b.state.gaps.iter().any(|g| (g.concept_a == a && g.concept_b == c) || (g.concept_a == c && g.concept_b == a)),
            "gap must auto-clear when the connecting statement gains a source"
        );
    }

    #[test]
    fn statement_update_deps_set_and_recompute() {
        let mut b = temp_braim("stmt_update_deps");
        let (a, _, s) = two_concepts_and_claim(&mut b);
        let d = b.add_concept("Gamma: third concept", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let mut set = HashMap::new();
        set.insert(a, 0.7);
        set.insert(d, 0.3);
        let new_deps = b.update_statement_deps(s, None, None, Some(set)).unwrap();
        assert_eq!(new_deps.len(), 2);
        assert!(new_deps.contains_key(&d));
        let node = b.get_node(s).unwrap();
        assert_eq!(node.depends_on.len(), 2);
        // narrative-only sources → still unproven after recompute
        assert_eq!(node.verification_status, VerificationStatus::Unproven);
    }

    #[test]
    fn statement_update_deps_rejects_concept_target_and_bad_weights() {
        let mut b = temp_braim("stmt_update_deps_neg");
        let (a, c, s) = two_concepts_and_claim(&mut b);
        // concept target rejected
        assert!(b.update_statement_deps(a, None, None, Some(HashMap::from([(c, 1.0)]))).is_err());
        // weights must sum to 1.0
        assert!(b.update_statement_deps(s, None, None, Some(HashMap::from([(a, 0.4), (c, 0.4)]))).is_err());
        // concept update-deps still rejects statements
        assert!(b.update_deps(s, None, None, Some(HashMap::from([(a, 1.0)]))).is_err());
    }

    /// Build a source graph for import tests: two concepts, a proven statement
    /// (code+doc), a claim depending on that statement, a source entity attached
    /// to the proven statement, one because_of edge and one contradicts edge.
    fn import_fixture(name: &str) -> (Braim, u32, u32) {
        let mut src = temp_braim(name);
        let a = src.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = src.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let s1 = src.add_statement("proven base", vec!["t".into()],
            vec!["code:a.rs:1".into(), "doc:a.md:2".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        // statement depending on a statement — exercises the fixpoint pass
        let s2 = src.add_statement("dependent claim", vec!["t".into()],
            vec!["narrative:n".into()], HashMap::from([(s1, 1.0)]), true).unwrap();
        let se = src.add_source("evidence ledger", "test", Some("test:run.log:3".into()), None).unwrap();
        src.add_source_to_statement(s1, se).unwrap();
        src.why_add(s2, s1, Some("narrative:why".into())).unwrap();
        let s3 = src.add_statement("rival claim", vec!["t".into()],
            vec!["narrative:m".into()], HashMap::from([(a, 0.7), (c, 0.3)]), true).unwrap();
        src.contradict_statements(s2, s3, "disagree", None).unwrap();
        (src, s1, s2)
    }

    #[test]
    fn full_import_preserves_verification_sources_and_edges() {
        let (src, s1, _) = import_fixture("full_import_src");
        let src_path = src.data_dir.join("current.json");
        let src_status = src.get_node(s1).unwrap().verification_status;

        let mut dst = temp_braim("full_import_dst");
        let m = dst.import_graph(src_path.to_str().unwrap(), None, None, HashMap::new(), true).unwrap();

        // everything crossed: 2 concepts + 3 statements imported, 1 source entity
        assert_eq!(m.imported_count, 5);
        assert_eq!(m.sources_imported, 1);
        assert_eq!(m.because_of_imported, 1);
        assert_eq!(m.contradicts_imported, 1);

        // verification preserved, not reset
        let new_s1 = m.id_mappings[&s1];
        let n = dst.get_node(new_s1).unwrap();
        assert_eq!(n.verification_status, src_status, "trusted import must not reset verification");
        assert_eq!(n.source_ids.len(), 1, "source-entity reference remapped, not dropped");

        // carried edges point at remapped ids that exist
        for e in &dst.state.because_of {
            assert!(dst.state.nodes.contains_key(&e.from) && dst.state.nodes.contains_key(&e.to));
        }
        assert_eq!(dst.state.contradicts.len(), 1);
    }

    #[test]
    fn legacy_import_still_resets_and_drops() {
        let (src, s1, _) = import_fixture("legacy_import_src");
        let src_path = src.data_dir.join("current.json");

        let mut dst = temp_braim("legacy_import_dst");
        let m = dst.import_graph(src_path.to_str().unwrap(), None, None, HashMap::new(), false).unwrap();

        assert_eq!(m.sources_imported, 0);
        assert_eq!(m.because_of_imported, 0);
        assert!(dst.state.because_of.is_empty() && dst.state.contradicts.is_empty());
        let new_s1 = m.id_mappings[&s1];
        assert_eq!(dst.get_node(new_s1).unwrap().verification_status, VerificationStatus::Unproven);
        assert!(dst.get_node(new_s1).unwrap().source_ids.is_empty());
    }

    #[test]
    fn full_import_dedup_unions_sources_and_recomputes() {
        let mut dst = temp_braim("union_dst");
        let a = dst.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = dst.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        // target has the same statement with only a code source → partial
        let s = dst.add_statement("shared finding", vec!["t".into()],
            vec!["code:a.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        assert_eq!(dst.get_node(s).unwrap().verification_status, VerificationStatus::Partial);

        // source graph: same concepts + same statement text/deps, but with a doc source
        let mut src = temp_braim("union_src");
        let sa = src.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let sc = src.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        src.add_statement("shared finding", vec!["t".into()],
            vec!["doc:spec.md:9".into()], HashMap::from([(sa, 0.6), (sc, 0.4)]), true).unwrap();
        let src_path = src.data_dir.join("current.json");

        let m = dst.import_graph(src_path.to_str().unwrap(), None, None, HashMap::new(), true).unwrap();
        assert_eq!(m.sources_unioned, 1);

        let n = dst.get_node(s).unwrap();
        assert!(n.sources.contains(&"doc:spec.md:9".to_string()), "duplicate's source unioned into target");
        // code + doc = two distinct PRIMARY types → promoted by the union
        assert_eq!(n.verification_status, VerificationStatus::Proven,
            "independent corroboration with a new PRIMARY type must promote (ID:185/190)");
    }

    #[test]
    fn only_proven_admits_proven_strong() {
        let mut src = temp_braim("proven_strong_src");
        let a = src.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = src.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        // three distinct PRIMARY types → proven_strong
        let s = src.add_statement("strong fact", vec!["t".into()],
            vec!["code:a.rs:1".into(), "doc:a.md:2".into(), "test:t.log:3".into()],
            HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        assert_eq!(src.get_node(s).unwrap().verification_status, VerificationStatus::ProvenStrong);

        // an unproven statement that must NOT cross with --only-proven
        let junk = src.add_statement("unproven aside", vec!["t".into()],
            vec!["narrative:n".into()], HashMap::from([(a, 1.0)]), true).unwrap();
        let src_path = src.data_dir.join("current.json");

        let mut dst = temp_braim("proven_strong_dst");
        // --only-proven admits: the proven_strong statement (rank fix: != Proven
        // used to drop it) PLUS its concept closure (concepts are vocabulary and
        // are admitted as dependencies regardless of their own status).
        let m = dst.import_graph(src_path.to_str().unwrap(), None, Some(VerificationStatus::Proven), HashMap::new(), true).unwrap();
        assert_eq!(m.imported_count, 3, "proven_strong statement + its 2 concepts");
        let new_s = m.id_mappings[&s];
        assert_eq!(dst.get_node(new_s).unwrap().verification_status, VerificationStatus::ProvenStrong);
        assert!(!m.id_mappings.contains_key(&junk), "unproven statement must not cross");
    }

    #[test]
    fn domain_export_carries_dependency_closure() {
        // Working graph: billing statement standing on a concept from another
        // domain, plus an unrelated other-domain node. Export must vendored-carry
        // the closure (ID:220, fixing lossy slice ID:180) and leave the rest.
        let mut work = temp_braim("export_closure_src");
        let bill = work.add_concept("Invoice: payment request document", vec!["billing".into()], vec!["narrative:x".into()], None).unwrap();
        let other = work.add_concept("Account: customer record", vec!["crm".into()], vec!["narrative:x".into()], None).unwrap();
        let unrelated = work.add_concept("Ticket: support case", vec!["crm".into()], vec!["narrative:x".into()], None).unwrap();
        let s = work.add_statement("invoices belong to accounts", vec!["billing".into()],
            vec!["code:b.rs:1".into(), "doc:b.md:2".into()], HashMap::from([(bill, 0.6), (other, 0.4)]), true).unwrap();
        let se = work.add_source("billing spec", "doc", Some("doc:spec.md".into()), None).unwrap();
        work.add_source_to_statement(s, se).unwrap();

        let mut central = temp_braim("export_closure_dst");
        let m = central.import_state(work.state.clone(), Some("billing"), Some(VerificationStatus::Proven), HashMap::new(), true).unwrap();

        // statement + its billing concept + its cross-domain concept dep cross
        assert!(m.id_mappings.contains_key(&s), "proven billing statement crosses");
        assert!(m.id_mappings.contains_key(&bill));
        assert!(m.id_mappings.contains_key(&other), "cross-domain dependency must be vendored (ID:180)");
        assert_eq!(m.sources_imported, 1, "attached source entity crosses via closure");
        // unrelated other-domain node stays home
        assert!(!m.id_mappings.contains_key(&unrelated), "unrelated crm node must not cross");
        // fidelity: status preserved through the export path
        let ns = m.id_mappings[&s];
        assert_eq!(central.get_node(ns).unwrap().verification_status,
            work.get_node(s).unwrap().verification_status);
    }

    #[test]
    fn import_reads_sharded_source_dir() {
        let (mut src, s1, _) = import_fixture("sharded_source");
        let dir = src.data_dir.clone();
        src.shard_layout().unwrap();

        let mut dst = temp_braim("sharded_source_dst");
        let m = dst.import_graph(dir.to_str().unwrap(), None, None, HashMap::new(), true).unwrap();
        assert!(m.id_mappings.contains_key(&s1), "import must load a sharded source dir");
        assert_eq!(m.because_of_imported, 1);
    }

    #[test]
    fn merge_unions_evidence_rewires_referents_and_can_promote() {
        let mut b = temp_braim("merge_union");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        // Two statements saying the same thing with DIFFERENT primary evidence.
        let keep = b.add_statement("payment settles invoice", vec!["d".into()],
            vec!["code:pay.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        let dup = b.add_statement("payment settles the invoice", vec!["d".into()],
            vec!["doc:spec.md:9".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        assert_eq!(b.get_node(keep).unwrap().verification_status, VerificationStatus::Partial);

        // A third statement depends on BOTH — after merge its weights must still sum to 1.0.
        let referent = b.add_statement("settlement matters", vec!["d".into()],
            vec!["code:z.rs:1".into()], HashMap::from([(keep, 0.7), (dup, 0.3)]), true).unwrap();

        let out = b.merge_nodes(keep, dup).unwrap();
        assert!(b.get_node(dup).is_none(), "loser is removed");
        assert_eq!(out.referents_rewired, 1);

        let w = b.get_node(keep).unwrap();
        assert!(w.sources.contains(&"doc:spec.md:9".to_string()), "loser's evidence unioned");
        assert_eq!(w.verification_status, VerificationStatus::Proven,
            "code + doc from the merge promotes the survivor");
        assert_eq!(w.metadata.get("merged_from").map(String::as_str), Some(dup.to_string().as_str()));

        let r = b.get_node(referent).unwrap();
        assert_eq!(r.depends_on.len(), 1, "both edges collapsed onto the winner");
        let total: f64 = r.depends_on.values().sum();
        assert!((total - 1.0).abs() < 1e-9, "summed weights preserve the 1.0 invariant, got {}", total);
    }

    #[test]
    fn merge_moves_edges_and_reports_dependency_differences() {
        let mut b = temp_braim("merge_edges");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let extra = b.add_concept("Gamma: third", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let keep = b.add_statement("claim one", vec!["d".into()],
            vec!["code:a.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        // duplicate stands on a dependency the winner lacks
        let dup = b.add_statement("claim one restated", vec!["d".into()],
            vec!["doc:a.md:1".into()], HashMap::from([(a, 0.5), (extra, 0.5)]), true).unwrap();
        let cause = b.add_statement("root cause", vec!["d".into()],
            vec!["code:c.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        b.why_add(dup, cause, Some("narrative:why".into())).unwrap();

        let out = b.merge_nodes(keep, dup).unwrap();
        assert!(out.edges_rewired >= 1, "because_of edge moved to the winner");
        assert!(b.state.because_of.iter().any(|e| e.from == keep && e.to == cause));
        assert_eq!(out.dep_differences, vec![extra],
            "a dependency only the loser had is reported, never silently merged");
        assert!(!b.get_node(keep).unwrap().depends_on.contains_key(&extra),
            "the winner's assertion is left intact");
    }

    #[test]
    fn merge_refuses_unsafe_pairs() {
        let mut b = temp_braim("merge_refuse");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let s1 = b.add_statement("one", vec!["d".into()],
            vec!["code:a.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        let s2 = b.add_statement("two", vec!["d".into()],
            vec!["code:b.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();

        assert!(b.merge_nodes(s1, s1).is_err(), "same node");
        assert!(b.merge_nodes(s1, 9999).is_err(), "missing node");
        assert!(b.merge_nodes(s1, a).is_err(), "concept into statement");

        // depends-on either way means related, not duplicate
        let dependent = b.add_statement("depends on one", vec!["d".into()],
            vec!["code:c.rs:1".into()], HashMap::from([(s1, 1.0)]), true).unwrap();
        assert!(b.merge_nodes(dependent, s1).is_err(), "winner depends on loser");
        assert!(b.merge_nodes(s1, dependent).is_err(), "loser depends on winner");

        // refuted evidence must not be laundered into a live node
        b.invalidate_statement(s2, "refuted").unwrap();
        assert!(b.merge_nodes(s1, s2).is_err(), "invalid loser");
        assert!(b.merge_nodes(s2, s1).is_err(), "invalid winner");
    }

    #[test]
    fn rename_domain_rehomes_shards() {
        let mut b = temp_braim("rename_domain");
        let dir = b.data_dir.clone();
        let a = b.add_concept("Alpha: first", vec!["Billing".into()], vec!["narrative:x".into()], None).unwrap();
        b.add_concept("Beta: second", vec!["billing".into()], vec!["narrative:x".into()], None).unwrap();
        b.shard_layout().unwrap();
        let mut b = Braim::new(dir.to_str().unwrap()).unwrap();

        let touched = b.rename_domain("Billing", "braim_demo").unwrap();
        assert_eq!(touched, 1);
        assert_eq!(b.get_node(a).unwrap().domains, vec!["braim_demo".to_string()]);
        // shard re-homed: new file exists, old case-variant shard pruned
        assert!(dir.join("domains").join(Braim::shard_filename("braim_demo")).exists());
        assert!(!dir.join("domains").join(Braim::shard_filename("Billing")).exists());
        assert!(dir.join("domains").join(Braim::shard_filename("billing")).exists(), "unrelated lowercase domain untouched");
        // errors: unknown domain, identity rename
        assert!(b.rename_domain("nope", "x").is_err());
        assert!(b.rename_domain("billing", "billing").is_err());
    }

    #[test]
    fn sharded_versions_are_per_domain_and_incremental() {
        let mut b = temp_braim("sharded_versions");
        let dir = b.data_dir.clone();
        let a = b.add_concept("Alpha: first", vec!["billing".into()], vec!["narrative:x".into()], None).unwrap();
        b.add_concept("Beta: second", vec!["crm".into()], vec!["narrative:x".into()], None).unwrap();
        b.shard_layout().unwrap();

        let mut b = Braim::new(dir.to_str().unwrap()).unwrap();
        let v1 = b.version_save("first checkpoint").unwrap();
        // per-domain pin artifacts exist (ID:214/242)
        let billing_v1 = dir.join("domains").join(Braim::shard_version_filename("billing", 1));
        let crm_v1 = dir.join("domains").join(Braim::shard_version_filename("crm", 1));
        assert!(billing_v1.exists() && crm_v1.exists(), "each domain gets its own versioned snapshot");

        // change ONLY billing, checkpoint again
        let c = b.add_concept("Invoice: payment request", vec!["billing".into()], vec!["narrative:y".into()], None).unwrap();
        let v2 = b.version_save("billing changed").unwrap();
        assert!(dir.join("domains").join(Braim::shard_version_filename("billing", 2)).exists(),
            "changed domain advances to v2");
        assert!(!dir.join("domains").join(Braim::shard_version_filename("crm", 2)).exists(),
            "unchanged domain must NOT get a new snapshot — its pin stays stable");

        // list reflects both checkpoints; restore v1 drops the new node, keeps the rest
        let list = b.version_list().unwrap();
        assert_eq!(list.len(), 2);
        b.version_restore(v1).unwrap();
        assert!(b.get_node(c).is_none(), "node added after v1 gone on restore");
        assert!(b.get_node(a).is_some());
        // restore v2 brings it back
        b.version_restore(v2).unwrap();
        assert!(b.get_node(c).is_some());
        // reload from disk still clean (snapshots not merged into working view)
        let again = Braim::new(dir.to_str().unwrap()).unwrap();
        assert_eq!(again.state.nodes.len(), b.state.nodes.len());
    }

    #[test]
    fn shard_roundtrip_preserves_full_state() {
        let (mut src, s1, _) = import_fixture("shard_roundtrip");
        let dir = src.data_dir.clone();
        let nodes_before = src.state.nodes.len();
        let s1_status = src.get_node(s1).unwrap().verification_status;

        let domain_count = src.shard_layout().unwrap();
        assert!(domain_count >= 1);
        assert!(dir.join("domains").is_dir());
        assert!(dir.join("graph.json").exists());
        assert!(!dir.join("current.json").exists(), "single file archived, not left as dual source");
        assert!(dir.join("current.json.pre-shard").exists());

        // Reload from disk: merged view identical
        let reloaded = Braim::new(dir.to_str().unwrap()).unwrap();
        assert_eq!(reloaded.state.nodes.len(), nodes_before);
        assert_eq!(reloaded.get_node(s1).unwrap().verification_status, s1_status);
        assert_eq!(reloaded.state.because_of.len(), src.state.because_of.len());
        assert_eq!(reloaded.state.contradicts.len(), src.state.contradicts.len());
        assert_eq!(reloaded.state.next_id, src.state.next_id);
        assert_eq!(reloaded.state.dictionary.len(), src.state.dictionary.len());
    }

    #[test]
    fn sharded_mutation_persists_and_reloads() {
        let (mut b, _, _) = import_fixture("shard_mutate");
        let dir = b.data_dir.clone();
        b.shard_layout().unwrap();

        // mutate AFTER sharding — flush must route to the sharded writer
        let mut b = Braim::new(dir.to_str().unwrap()).unwrap();
        let g = b.add_concept("Gamma: third", vec!["newdomain".into()], vec!["narrative:z".into()], None).unwrap();

        let again = Braim::new(dir.to_str().unwrap()).unwrap();
        assert!(again.get_node(g).is_some(), "mutation in sharded mode must persist");
        assert_eq!(again.get_node(g).unwrap().domains, vec!["newdomain".to_string()]);
        // and the new domain got its own shard file
        let shard = dir.join("domains").join(Braim::shard_filename("newdomain"));
        assert!(shard.exists(), "new home domain must create its shard file");
    }

    #[test]
    fn case_colliding_domains_get_distinct_shards() {
        // Real data has both "Billing" and "billing" as distinct domains; on
        // case-insensitive filesystems raw names would be one file (ID:236).
        let a = Braim::shard_filename("Billing");
        let b = Braim::shard_filename("billing");
        assert_ne!(a, b, "distinct domains must never share a shard file");
        // and the sanitized prefix is still human-readable
        assert!(a.starts_with("billing-") && b.starts_with("billing-"));
        // determinism
        assert_eq!(a, Braim::shard_filename("Billing"));
    }

    #[test]
    fn serialization_is_deterministic_across_instances() {
        // Two graphs, identical operation sequences, separate HashMap seeds →
        // current.json must still be byte-identical (braim ID:218/226: diffable
        // packs and byte-level integrity both require canonical serialization).
        let build = |name: &str| -> String {
            let mut b = temp_braim(name);
            let a = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
            let c = b.add_concept("Beta: second", vec!["u".into()], vec!["narrative:y".into()], None).unwrap();
            let g = b.add_concept("Gamma: third", vec!["t".into()], vec!["narrative:z".into()], None).unwrap();
            let s = b.add_statement("alpha relates to beta", vec!["t".into()],
                vec!["code:x.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
            b.add_statement("beta relates to gamma", vec!["u".into()],
                vec!["doc:y.md:2".into()], HashMap::from([(c, 0.7), (g, 0.3)]), true).unwrap();
            b.set_meta(s, "scope", "agent_scratch").unwrap();
            std::fs::read_to_string(b.data_dir.join("current.json")).unwrap()
        };
        let one = build("determinism_a");
        let two = build("determinism_b");
        assert_eq!(one, two, "identical operations must produce byte-identical current.json");
    }

    #[test]
    fn revalidate_round_trips_invalidate() {
        let mut b = temp_braim("revalidate_roundtrip");
        let a = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let s = b.add_statement("alpha relates to beta", vec!["t".into()],
            vec!["code:x.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        assert_eq!(b.get_node(s).unwrap().verification_status, VerificationStatus::Partial);
        b.invalidate_statement(s, "test").unwrap();
        assert_eq!(b.get_node(s).unwrap().verification_status, VerificationStatus::Invalid);
        let (status, invalid_deps) = b.revalidate_statement(s).unwrap();
        assert_eq!(status, VerificationStatus::Partial, "one code source → partial after revalidate");
        assert!(invalid_deps.is_empty());
        let node = b.get_node(s).unwrap();
        assert!(!node.invalid);
        assert!(node.invalid_reason.is_none());
        assert_eq!(node.node_type, NodeType::Fact);
    }

    #[test]
    fn revalidate_skips_invalid_dep_in_cap() {
        let mut b = temp_braim("revalidate_skip_invalid_dep");
        let a = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        // S1 is a statement; S2 depends on S1 so the cascade reaches it.
        let s1 = b.add_statement("base claim", vec!["t".into()],
            vec!["code:a.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        let s2 = b.add_statement("dependent claim", vec!["t".into()],
            vec!["code:b.rs:1".into()], HashMap::from([(s1, 1.0)]), true).unwrap();
        b.invalidate_statement(s1, "retired").unwrap();
        // cascade reached s2
        assert_eq!(b.get_node(s2).unwrap().verification_status, VerificationStatus::Invalid);
        // revalidate s2 while s1 stays invalid: invalid dep is skipped, not re-poisoning.
        let (status, invalid_deps) = b.revalidate_statement(s2).unwrap();
        assert_eq!(status, VerificationStatus::Partial, "s2 revives to its own source-derived status");
        assert_eq!(invalid_deps, vec![s1], "the still-invalid dep is reported for re-anchoring");
        assert!(!b.get_node(s2).unwrap().invalid);
    }

    #[test]
    fn revalidate_clears_orphan_contested_but_refuses_active_contradiction() {
        let mut b = temp_braim("revalidate_orphan_contested");
        let a = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let s1 = b.add_statement("claim one", vec!["t".into()],
            vec!["code:a.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        let s2 = b.add_statement("claim two", vec!["t".into()],
            vec!["code:b.rs:1".into()], HashMap::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        b.contradict_statements(s1, s2, "conflict", None).unwrap();
        // active contradiction edge → revalidate must refuse
        assert!(b.revalidate_statement(s1).is_err(), "must not touch a genuinely contested node");
        // Orphan-contested trap: s3 inherits contested from s1, then its contested dep is
        // swapped out. update_statement_deps skips recompute for contested nodes, so s3 is
        // left contested with only concept deps and no contradiction edge of its own.
        let s3 = b.add_statement("inherits contested", vec!["t".into()],
            vec!["code:c.rs:1".into()], HashMap::from([(s1, 1.0)]), true).unwrap();
        assert_eq!(b.get_node(s3).unwrap().verification_status, VerificationStatus::Contested);
        b.update_statement_deps(s3, None, None, Some(HashMap::from([(a, 0.6), (c, 0.4)]))).unwrap();
        assert_eq!(b.get_node(s3).unwrap().verification_status, VerificationStatus::Contested,
            "update-deps leaves the orphan-contested node stuck");
        // revalidate recomputes it off the contested state.
        let (status, _) = b.revalidate_statement(s3).unwrap();
        assert_eq!(status, VerificationStatus::Partial, "orphan-contested node recomputes to its source-derived status");
    }
}
