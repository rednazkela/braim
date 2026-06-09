//! Semantic similarity for braim: embedding-backed dedup + discovery.
//!
//! Why this exists (braim ID:6629/6630): lexical `query` returns nothing for
//! paraphrase intents; on clean `Concept: definition` labels an embedding index
//! gives strong top-1 precision (dedup) and useful discovery. This module is the
//! Rust-native ship of that test-drive — fastembed (ONNX MiniLM) + a sidecar
//! vector store + brute-force cosine. It stays ADVISORY: it augments, never
//! overrides, the verification lifecycle.
//!
//! The heavy ML dependency is gated behind the `embeddings` cargo feature so the
//! base binary builds unchanged. Storage is a sidecar `.braim/embeddings.json`
//! (NOT current.json — same reason enrichments are sidecar: graph rewrites must
//! not clobber the index, and the JSON stays human-readable).

use crate::graph::{Braim, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const EMBED_MODEL_ID: &str = "AllMiniLML6V2";
pub const EMBED_SIDECAR: &str = "embeddings.json";

/// Cosine at/above which a candidate label is flagged as a possible duplicate at
/// write time. Deliberately ADVISORY, not a hard block: validation on real graphs
/// (braim ID:6628/6629) showed genuinely distinct concepts can score ~0.88
/// (Word Overlap Bias vs Reverse Word Overlap Bias), so blocking would
/// false-positive. The check warns; the human/agent decides.
pub const DEDUP_WARN_THRESHOLD: f32 = 0.80;

/// One stored vector plus the hash of the cleaned label it was computed from.
/// label_hash drives lazy staleness: if a node's cleaned label changes, its
/// entry is recomputed on the next `similar` run; everything else is cached.
#[derive(Serialize, Deserialize, Clone)]
pub struct EmbedEntry {
    pub label_hash: u64,
    pub vec: Vec<f32>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct EmbedIndex {
    /// Model id the vectors were produced by. A mismatch forces a full rebuild —
    /// vectors from different models are not comparable.
    pub model: String,
    pub vectors: HashMap<u32, EmbedEntry>,
}

impl EmbedIndex {
    pub fn load(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join(EMBED_SIDECAR);
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => EmbedIndex::default(),
        }
    }

    pub fn save(&self, data_dir: &std::path::Path) -> Result<(), String> {
        let path = data_dir.join(EMBED_SIDECAR);
        let s = serde_json::to_string(self).map_err(|e| format!("serialize index: {e}"))?;
        std::fs::write(&path, s).map_err(|e| format!("write {}: {e}", path.display()))
    }
}

/// Stable FNV-1a 64-bit hash. std's DefaultHasher is not stable across releases,
/// so we hash labels ourselves to keep the sidecar valid across upgrades.
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Strip a leading snake_case / dated slug prefix and de-underscore the rest, so
/// the embedder sees prose, not identifier noise (braim ID:6629: corpus quality
/// gates embedding usefulness). Clean `Concept: definition` labels pass through
/// unchanged because their prefix contains a space.
pub fn clean_label(label: &str) -> String {
    let s = label.trim();
    let cleaned = if let Some((prefix, rest)) = s.split_once(": ") {
        let looks_like_slug = !prefix.contains(' ')
            && (prefix.contains('_') || has_iso_date(prefix));
        if looks_like_slug { rest } else { s }
    } else {
        s
    };
    let out: String = cleaned.replace('_', " ");
    out.chars().take(500).collect()
}

fn has_iso_date(s: &str) -> bool {
    // crude YYYY-MM-DD presence check, no regex dep
    let b = s.as_bytes();
    if b.len() < 10 {
        return false;
    }
    for w in b.windows(10) {
        if w[0].is_ascii_digit() && w[1].is_ascii_digit() && w[2].is_ascii_digit()
            && w[3].is_ascii_digit() && w[4] == b'-' && w[5].is_ascii_digit()
            && w[6].is_ascii_digit() && w[7] == b'-' && w[8].is_ascii_digit()
            && w[9].is_ascii_digit()
        {
            return true;
        }
    }
    false
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

pub fn node_type_str(t: &NodeType) -> &'static str {
    match t {
        NodeType::Atomic => "atomic",
        NodeType::Compound => "compound",
        NodeType::Statement => "statement",
        NodeType::Claim => "claim",
        NodeType::Fact => "fact",
        NodeType::InvalidStatement => "invalid_statement",
        NodeType::ContestedStatement => "contested_statement",
        NodeType::Source => "source",
    }
}

/// (id, cleaned_label, raw_label, node_type_str) for every node with a non-empty
/// label, sorted by id for deterministic output.
pub fn corpus(braim: &Braim) -> Vec<(u32, String, String, &'static str)> {
    let mut rows: Vec<_> = braim
        .state
        .nodes
        .values()
        .filter(|n| !n.label.trim().is_empty())
        .map(|n| {
            (
                n.id,
                clean_label(&n.label),
                n.label.clone(),
                node_type_str(&n.node_type),
            )
        })
        .collect();
    rows.sort_by_key(|r| r.0);
    rows
}

/// Abstraction boundary so the embedding provider can be swapped (fastembed now,
/// a different local model or remote endpoint later) without touching callers.
pub trait Embedder {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

#[cfg(feature = "embeddings")]
pub struct FastEmbedder {
    model: fastembed::TextEmbedding,
}

#[cfg(feature = "embeddings")]
impl FastEmbedder {
    pub fn new() -> Result<Self, String> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
            .map_err(|e| format!("embedder init failed (model download/onnx): {e}"))?;
        Ok(Self { model })
    }
}

#[cfg(feature = "embeddings")]
impl Embedder for FastEmbedder {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        self.model
            .embed(texts.to_vec(), None)
            .map_err(|e| format!("embed failed: {e}"))
    }
}

/// Refresh the sidecar index: (re)embed only nodes whose cleaned label is missing
/// or whose hash changed. Returns the number of vectors (re)computed.
pub fn refresh_index(
    embedder: &mut dyn Embedder,
    index: &mut EmbedIndex,
    rows: &[(u32, String, String, &'static str)],
    rebuild: bool,
) -> Result<usize, String> {
    if rebuild || index.model != EMBED_MODEL_ID {
        index.model = EMBED_MODEL_ID.to_string();
        index.vectors.clear();
    }
    let present: std::collections::HashSet<u32> = rows.iter().map(|r| r.0).collect();
    index.vectors.retain(|id, _| present.contains(id)); // drop deleted nodes

    let mut stale_ids = Vec::new();
    let mut stale_texts = Vec::new();
    for (id, clean, _, _) in rows {
        let h = fnv1a(clean);
        let fresh = index.vectors.get(id).map(|e| e.label_hash == h).unwrap_or(false);
        if !fresh {
            stale_ids.push((*id, h));
            stale_texts.push(clean.clone());
        }
    }
    if stale_texts.is_empty() {
        return Ok(0);
    }
    let vecs = embedder.embed(&stale_texts)?;
    if vecs.len() != stale_ids.len() {
        return Err(format!(
            "embedder returned {} vectors for {} inputs",
            vecs.len(),
            stale_ids.len()
        ));
    }
    for ((id, h), v) in stale_ids.into_iter().zip(vecs.into_iter()) {
        index.vectors.insert(id, EmbedEntry { label_hash: h, vec: v });
    }
    Ok(stale_texts.len())
}

/// Cosine at/above which a statement is flagged for restating the label of one
/// of its own dependencies — a single-concept elaboration that adds no
/// relationship ('Concept: description' labels are already self-documenting).
pub const ECHO_WARN_THRESHOLD: f32 = 0.75;

pub struct DupPair {
    pub id_a: u32,
    pub id_b: u32,
    pub score: f32,
}

pub struct EchoWarning {
    pub statement_id: u32,
    pub dep_id: u32,
    pub score: f32,
}

/// All-pairs near-duplicate scan over the index. Skips source entities (their
/// identity is a location, not a label), invalid statements, and directly
/// connected pairs — a statement scoring high against its own dependency is the
/// echo check's territory, not a duplicate.
pub fn semantic_duplicates(braim: &Braim, index: &EmbedIndex, threshold: f32) -> Vec<DupPair> {
    let mut ids: Vec<u32> = index
        .vectors
        .keys()
        .copied()
        .filter(|id| {
            braim
                .state
                .nodes
                .get(id)
                .map(|n| n.node_type != NodeType::Source && n.node_type != NodeType::InvalidStatement)
                .unwrap_or(false)
        })
        .collect();
    ids.sort_unstable();

    let connected = |a: u32, b: u32| -> bool {
        let dep = |x: u32, y: u32| {
            braim
                .state
                .nodes
                .get(&x)
                .map(|n| n.depends_on.contains_key(&y))
                .unwrap_or(false)
        };
        dep(a, b) || dep(b, a)
    };

    let mut pairs = Vec::new();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            if connected(a, b) {
                continue;
            }
            let s = cosine(&index.vectors[&a].vec, &index.vectors[&b].vec);
            if s >= threshold {
                pairs.push(DupPair { id_a: a, id_b: b, score: s });
            }
        }
    }
    pairs.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap_or(std::cmp::Ordering::Equal));
    pairs
}

/// Label-echo scan: active statement-family nodes whose label scores >= threshold
/// against the label of one of their own concept dependencies.
pub fn label_echoes(braim: &Braim, index: &EmbedIndex, threshold: f32) -> Vec<EchoWarning> {
    let mut warnings = Vec::new();
    let mut ids: Vec<u32> = braim.state.nodes.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let node = &braim.state.nodes[&id];
        if !node.node_type.is_statement_family() || node.node_type == NodeType::InvalidStatement {
            continue;
        }
        let Some(stmt_vec) = index.vectors.get(&id) else { continue };
        let mut dep_ids: Vec<u32> = node.depends_on.keys().copied().collect();
        dep_ids.sort_unstable();
        for dep_id in dep_ids {
            let is_concept = braim
                .state
                .nodes
                .get(&dep_id)
                .map(|d| matches!(d.node_type, NodeType::Atomic | NodeType::Compound))
                .unwrap_or(false);
            if !is_concept {
                continue;
            }
            let Some(dep_vec) = index.vectors.get(&dep_id) else { continue };
            let s = cosine(&stmt_vec.vec, &dep_vec.vec);
            if s >= threshold {
                warnings.push(EchoWarning { statement_id: id, dep_id, score: s });
            }
        }
    }
    warnings.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap_or(std::cmp::Ordering::Equal));
    warnings
}

pub struct Hit {
    pub id: u32,
    pub score: f32,
    pub node_type: &'static str,
    pub label: String,
}

/// Brute-force cosine top-k over the index. At < ~100k nodes a linear scan is
/// sub-10ms; an ANN index is deliberately deferred until the graph outgrows it.
pub fn top_k(
    query_vec: &[f32],
    index: &EmbedIndex,
    rows: &[(u32, String, String, &'static str)],
    k: usize,
    min_score: f32,
    exclude: Option<u32>,
) -> Vec<Hit> {
    let meta: HashMap<u32, (&'static str, &String)> =
        rows.iter().map(|(id, _, raw, t)| (*id, (*t, raw))).collect();
    let mut scored: Vec<Hit> = index
        .vectors
        .iter()
        .filter(|(id, _)| exclude.map(|e| **id != e).unwrap_or(true))
        .filter_map(|(id, e)| {
            let s = cosine(query_vec, &e.vec);
            if s < min_score {
                return None;
            }
            // index ids are always a subset of rows (refresh_index retains only
            // present nodes), so a miss here means a deleted node — skip it.
            let (t, raw) = meta.get(id)?;
            Some(Hit {
                id: *id,
                score: s,
                node_type: t,
                label: (*raw).clone(),
            })
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn temp_braim(name: &str) -> Braim {
        let dir = std::env::temp_dir().join(format!("braim_embed_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Braim::new(dir.to_str().unwrap()).unwrap()
    }

    fn entry(v: Vec<f32>) -> EmbedEntry {
        EmbedEntry { label_hash: 0, vec: v }
    }

    #[test]
    fn duplicates_flagged_connected_pairs_skipped() {
        let mut b = temp_braim("dups");
        let a = b.add_concept("Alpha: first concept", vec!["t".into()], vec!["code:x.rs".into()], None).unwrap();
        let c = b.add_concept("Beta: second concept", vec!["t".into()], vec!["code:x.rs".into()], None).unwrap();
        let mut deps = HashMap::new();
        deps.insert(a, 0.6);
        deps.insert(c, 0.4);
        let s = b.add_statement("alpha relates to beta", vec!["t".into()], vec!["code:x.rs".into()], deps, true).unwrap();

        let mut index = EmbedIndex::default();
        // a and c identical vectors -> duplicate pair; s connected to both -> skipped
        index.vectors.insert(a, entry(vec![1.0, 0.0]));
        index.vectors.insert(c, entry(vec![1.0, 0.0]));
        index.vectors.insert(s, entry(vec![1.0, 0.0]));

        let dups = semantic_duplicates(&b, &index, 0.8);
        assert_eq!(dups.len(), 1, "only the unconnected a-c pair should be flagged");
        assert_eq!((dups[0].id_a, dups[0].id_b), (a.min(c), a.max(c)));
        assert!(dups[0].score > 0.99);
    }

    #[test]
    fn orthogonal_vectors_not_duplicates() {
        let mut b = temp_braim("ortho");
        let a = b.add_concept("Alpha: first concept", vec!["t".into()], vec!["code:x.rs".into()], None).unwrap();
        let c = b.add_concept("Beta: second concept", vec!["t".into()], vec!["code:x.rs".into()], None).unwrap();
        let mut index = EmbedIndex::default();
        index.vectors.insert(a, entry(vec![1.0, 0.0]));
        index.vectors.insert(c, entry(vec![0.0, 1.0]));
        assert!(semantic_duplicates(&b, &index, 0.8).is_empty());
    }

    #[test]
    fn echo_detected_on_own_dependency_only() {
        let mut b = temp_braim("echo");
        let a = b.add_concept("Alpha: first concept", vec!["t".into()], vec!["code:x.rs".into()], None).unwrap();
        let c = b.add_concept("Beta: second concept", vec!["t".into()], vec!["code:x.rs".into()], None).unwrap();
        let mut deps = HashMap::new();
        deps.insert(a, 0.7);
        deps.insert(c, 0.3);
        let s = b.add_statement("alpha restated", vec!["t".into()], vec!["code:x.rs".into()], deps, true).unwrap();

        let mut index = EmbedIndex::default();
        index.vectors.insert(a, entry(vec![1.0, 0.0]));      // echoed by s
        index.vectors.insert(c, entry(vec![0.0, 1.0]));      // orthogonal to s
        index.vectors.insert(s, entry(vec![1.0, 0.0]));

        let echoes = label_echoes(&b, &index, ECHO_WARN_THRESHOLD);
        assert_eq!(echoes.len(), 1);
        assert_eq!(echoes[0].statement_id, s);
        assert_eq!(echoes[0].dep_id, a);
    }
}
