use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use chrono::Utc;

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
    pub domains: Vec<String>,
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

pub struct AuditReport {
    pub orphans: Vec<Node>,
    pub pending: Vec<Node>,
    pub gaps: Vec<GapRecord>,
    pub deprecated_referenced: Vec<Node>,
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

pub struct ImportManifest {
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
        let path = PathBuf::from(data_dir);
        fs::create_dir_all(&path).map_err(|e| format!("Failed to create data dir: {}", e))?;

        let current_path = path.join("current.json");
        let mut state: GraphState = if current_path.exists() {
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
        })
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

    fn flush(&mut self) -> Result<(), String> {
        let path = self.data_dir.join("current.json");
        let content = serde_json::to_string_pretty(&self.state)
            .map_err(|e| format!("Failed to serialize state: {}", e))?;
        fs::write(&path, content)
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

        let candidates: Vec<(u32, f64)> = self
            .state
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.status == NodeStatus::Active
                    && node.depends_on.contains_key(&current)
                    && !visited.contains(&node.id)
            })
            .map(|(_, node)| (node.id, node.depends_on[&current]))
            .collect();

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
        }

        AuditReport {
            orphans,
            pending,
            gaps: self.state.gaps.clone(),
            deprecated_referenced,
        }
    }

    pub fn version_save(&mut self, description: &str) -> Result<u32, String> {
        self.state.version += 1;
        let version_num = self.state.version;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let meta = VersionMeta {
            description: description.to_string(),
            saved_at: now,
            data: self.state.clone(),
        };

        let filename = format!("v{:04}.json", version_num);
        let path = self.data_dir.join(&filename);
        let content = serde_json::to_string_pretty(&meta)
            .map_err(|e| format!("Failed to serialize version: {}", e))?;
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write version file: {}", e))?;

        self.flush()?;
        Ok(version_num)
    }

    pub fn version_restore(&mut self, n: u32) -> Result<(), String> {
        let filename = format!("v{:04}.json", n);
        let path = self.data_dir.join(&filename);

        let content = fs::read_to_string(&path)
            .map_err(|_| format!("Error: Version {} not found", n))?;
        let meta: VersionMeta = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse version file: {}", e))?;

        self.state = meta.data;
        self.flush()?;
        Ok(())
    }

    pub fn version_list(&self) -> Result<Vec<VersionMeta>, String> {
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
                        versions.push(meta);
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

        self.flush()?;
        Ok(cascade_ids)
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

    pub fn import_graph(
        &mut self,
        source_path: &str,
        filter_domain: Option<&str>,
        only_proven: bool,
        domain_mappings: HashMap<String, String>,
    ) -> Result<ImportManifest, String> {
        // Load source graph
        let source_content = fs::read_to_string(source_path)
            .map_err(|e| format!("Error reading source file: {}", e))?;
        let mut source_state: GraphState = serde_json::from_str(&source_content)
            .map_err(|e| format!("Error parsing source graph: {}", e))?;

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

        // Collect nodes by type for ordered processing
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
                NodeType::Source => {} // skip source nodes during import
            }
        }

        // Process atomics first
        for node in atomics {
            if let Some(domain_filter) = filter_domain {
                if !node.domains.contains(&domain_filter.to_string()) {
                    skipped_count += 1;
                    continue;
                }
            }

            if only_proven && node.verification_status != VerificationStatus::Proven {
                skipped_count += 1;
                continue;
            }

            let key = (node.label.to_lowercase(), node.domains.clone());
            let mut is_duplicate = false;

            // Check for existing atomic with same name and domain
            for (target_id, target_node) in &self.state.nodes {
                if target_node.node_type == NodeType::Atomic
                    && target_node.label.to_lowercase() == key.0
                    && target_node.domains == key.1
                {
                    id_mappings.insert(node.id, *target_id);
                    duplicates.push(DuplicateRecord {
                        source_id: node.id,
                        target_id: *target_id,
                        target_label: target_node.label.clone(),
                        reason: "same name and domain".to_string(),
                    });
                    is_duplicate = true;
                    deduplicated_count += 1;
                    break;
                }
            }

            if !is_duplicate {
                let new_id = self.state.next_id;
                id_mappings.insert(node.id, new_id);

                let mut new_node = node.clone();
                new_node.id = new_id;
                new_node.verification_status = VerificationStatus::Unproven;
                new_node.verified_by = HashMap::new();

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
            if let Some(domain_filter) = filter_domain {
                if !node.domains.contains(&domain_filter.to_string()) {
                    skipped_count += 1;
                    continue;
                }
            }

            if only_proven && node.verification_status != VerificationStatus::Proven {
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
            let mut is_duplicate = false;

            // Check for existing compound with same name and domain
            for (target_id, target_node) in &self.state.nodes {
                if target_node.node_type == NodeType::Compound
                    && target_node.label.to_lowercase() == key.0
                    && target_node.domains == key.1
                {
                    id_mappings.insert(node.id, *target_id);
                    duplicates.push(DuplicateRecord {
                        source_id: node.id,
                        target_id: *target_id,
                        target_label: target_node.label.clone(),
                        reason: "same name and domain".to_string(),
                    });
                    is_duplicate = true;
                    deduplicated_count += 1;
                    break;
                }
            }

            if !is_duplicate {
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
                new_node.verification_status = VerificationStatus::Unproven;
                new_node.verified_by = HashMap::new();

                self.state.nodes.insert(new_id, new_node.clone());
                self.state.next_id += 1;
                imported_count += 1;

                // Update dictionary
                let lower_label = node.label.to_lowercase();
                self.state.dictionary.entry(lower_label).or_insert_with(Vec::new).push(new_id);
            }
        }

        // Process statements
        for node in statements {
            if let Some(domain_filter) = filter_domain {
                if !node.domains.contains(&domain_filter.to_string()) {
                    skipped_count += 1;
                    continue;
                }
            }

            if only_proven && node.verification_status != VerificationStatus::Proven {
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

            // Remap dependency IDs for comparison
            let mut remapped_deps = Vec::new();
            for dep_id in node.depends_on.keys() {
                if let Some(mapped_id) = id_mappings.get(dep_id) {
                    remapped_deps.push(*mapped_id);
                }
            }
            remapped_deps.sort();

            let mut is_duplicate = false;

            // Check for existing statement with same text and remapped dependencies
            for (target_id, target_node) in &self.state.nodes {
                if target_node.node_type.is_statement_family()
                    && target_node.label == node.label
                {
                    let mut target_deps: Vec<u32> = target_node.depends_on.keys().copied().collect();
                    target_deps.sort();

                    if target_deps == remapped_deps {
                        id_mappings.insert(node.id, *target_id);
                        duplicates.push(DuplicateRecord {
                            source_id: node.id,
                            target_id: *target_id,
                            target_label: target_node.label.clone(),
                            reason: "same text and dependencies".to_string(),
                        });
                        is_duplicate = true;
                        deduplicated_count += 1;
                        break;
                    }
                }
            }

            if !is_duplicate {
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
                new_node.verification_status = VerificationStatus::Unproven;
                new_node.verified_by = HashMap::new();

                self.state.nodes.insert(new_id, new_node.clone());
                self.state.next_id += 1;
                imported_count += 1;
            }
        }

        self.flush()?;

        Ok(ImportManifest {
            imported_count,
            deduplicated_count,
            skipped_count,
            id_mappings,
            duplicates,
        })
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
}
