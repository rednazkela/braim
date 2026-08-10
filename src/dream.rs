//! Dream candidate generation: pick node pairs worth an LLM's attention.
//!
//! "Dreaming" walks pairs of unconnected nodes so an LLM can look for a relation
//! the graph is missing. The pair space makes brute force impossible — a
//! 3,189-node graph has 5,083,266 pairs — so braim does the cheap deterministic
//! half (which pairs are worth reading) and the LLM does the expensive half
//! (whether a relation is real and whether sources prove it). That split is what
//! makes overnight tokens buy judgment instead of shuffling (braim ID:255).
//!
//! Nothing here writes statements. Candidate generation is read-only by
//! construction: an LLM asked whether two nodes relate will nearly always say
//! yes, so a dream loop must land its output as unproven claims in a separate
//! working graph and earn promotion through genuinely re-grounded PRIMARY
//! sources like any other statement (braim ID:256).

use crate::graph::{Braim, NodeType, VerificationStatus};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Marker file designating a data dir as a central/shared graph. Dreaming is
/// refused there: a dream is a hypothesis needing review, and an unattended
/// central has no reviewer.
pub const CENTRAL_MARKER: &str = ".braim.central";

/// Ledger of pairs already adjudicated, so successive nights advance instead of
/// re-treading the same ground.
pub const DREAM_LEDGER: &str = "dreams.json";

/// A source cited by more nodes than this is a hub (a whole file, a broad doc).
/// Its pairs are combinatorially many and individually weak, so it is skipped
/// rather than allowed to flood the worklist.
const MAX_SOURCE_FANOUT: usize = 25;

/// Likewise for two-hop bridges: a bridging node connected to everything relates
/// nothing in particular.
const MAX_BRIDGE_DEGREE: usize = 40;

/// Semantic pairs below this cosine are not "surprisingly close" enough to spend
/// a judgment call on.
pub const DEFAULT_MIN_SEMANTIC: f32 = 0.72;

/// No single structural signal is conclusive, so one strategy alone cannot reach
/// the top of the worklist. Capping the base score leaves headroom for the
/// agreement bonus — the point being that independent signals concurring is
/// stronger evidence than any one of them saturating.
const SINGLE_SIGNAL_CEILING: f32 = 0.85;

#[derive(Serialize, Clone, Debug, PartialEq, Eq, Hash, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    /// Both nodes cite the same PRIMARY source but were never linked.
    SharedSource,
    /// A–B and B–C exist, A–C does not: a transitive-closure candidate.
    TwoHop,
    /// Semantically close, structurally distant — the "unexpected" signal.
    Semantic,
    /// A pair a real query wanted and found no path for (gap register).
    RegisteredGap,
}

impl Strategy {
    pub fn parse(s: &str) -> Result<Strategy, String> {
        match s.trim().to_lowercase().replace('_', "-").as_str() {
            "shared-source" | "source" => Ok(Strategy::SharedSource),
            "two-hop" | "twohop" => Ok(Strategy::TwoHop),
            "semantic" => Ok(Strategy::Semantic),
            "gap" | "registered-gap" => Ok(Strategy::RegisteredGap),
            other => Err(format!(
                "Error: unknown dream strategy '{}' (expected shared-source, two-hop, semantic, or gap)",
                other
            )),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Strategy::SharedSource => "shared-source",
            Strategy::TwoHop => "two-hop",
            Strategy::Semantic => "semantic",
            Strategy::RegisteredGap => "gap",
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Candidate {
    pub a: u32,
    pub b: u32,
    pub a_label: String,
    pub b_label: String,
    pub a_domains: Vec<String>,
    pub b_domains: Vec<String>,
    /// Every strategy that nominated this pair. More than one is corroborating
    /// structural evidence and raises the score.
    pub strategies: Vec<&'static str>,
    pub score: f32,
    /// Why this pair surfaced, in words the adjudicating LLM can act on.
    pub rationale: String,
}

/// One adjudicated pair. Written by the dream loop, read back to skip pairs.
#[derive(Serialize, serde::Deserialize, Clone, Debug)]
pub struct LedgerEntry {
    pub a: u32,
    pub b: u32,
    /// no-relation | proposed | verified | contradiction | duplicate
    pub verdict: String,
    pub note: Option<String>,
    pub recorded_at: String,
}

fn pair_key(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

pub fn ledger_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(DREAM_LEDGER)
}

pub fn load_ledger(data_dir: &std::path::Path) -> Vec<LedgerEntry> {
    std::fs::read_to_string(ledger_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn record_ledger(
    data_dir: &std::path::Path,
    a: u32,
    b: u32,
    verdict: &str,
    note: Option<String>,
) -> Result<(), String> {
    const VERDICTS: [&str; 5] =
        ["no-relation", "proposed", "verified", "contradiction", "duplicate"];
    if !VERDICTS.contains(&verdict) {
        return Err(format!(
            "Error: verdict must be one of {} (got '{}')",
            VERDICTS.join(", "),
            verdict
        ));
    }
    let (a, b) = pair_key(a, b);
    let mut ledger = load_ledger(data_dir);
    ledger.retain(|e| pair_key(e.a, e.b) != (a, b));
    ledger.push(LedgerEntry {
        a,
        b,
        verdict: verdict.to_string(),
        note,
        recorded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    let text = serde_json::to_string_pretty(&ledger)
        .map_err(|e| format!("Failed to serialize dream ledger: {}", e))?;
    std::fs::write(ledger_path(data_dir), text)
        .map_err(|e| format!("Failed to write dream ledger: {}", e))
}

/// Refuse to dream on a shared/central graph — a dream is a hypothesis awaiting
/// review, and nobody reviews an unattended central.
pub fn refuse_if_central(data_dir: &std::path::Path) -> Result<(), String> {
    if data_dir.join(CENTRAL_MARKER).exists() {
        return Err(format!(
            "Error: {} is marked central ({} present) and dreaming is refused there.\n\
             Dreams are unreviewed hypotheses; they belong in a local working graph where \
             a human sees them before anything is published. Dream locally, review, then \
             `braim export <domain> --to {}`.",
            data_dir.display(),
            CENTRAL_MARKER,
            data_dir.display()
        ));
    }
    Ok(())
}

/// A load-bearing cause, ranked by how much of the graph rests on it.
///
/// braim cannot tell a *constraint* from any other cause — that is a judgement
/// about meaning. What it can compute is **leverage**: how many statements would
/// need re-examining if this one stopped being true. Ranking by leverage and
/// letting the LLM decide which of the top entries are actually relaxable keeps
/// the same split that makes pair-dreaming work (braim ID:323).
#[derive(Serialize, Clone, Debug)]
pub struct ConstraintCandidate {
    pub id: u32,
    pub label: String,
    pub domains: Vec<String>,
    pub verification: String,
    /// Statements that reach this one through because_of, transitively. The
    /// blast radius if the constraint were lifted.
    pub impact: usize,
    /// Consequents citing it directly.
    pub direct: usize,
    /// Longest causal chain hanging off it.
    pub depth: usize,
    /// Label uses limitation vocabulary. **Advisory annotation, never a filter** —
    /// on this graph the same lexicon matched 61 of 161 statements, most of them
    /// false positives ("Invoice Payment *must* be recorded"), so it informs the
    /// reader and contributes nothing to the score.
    pub reads_as_limitation: bool,
    pub score: f32,
    pub rationale: String,
}

/// Share of its impact an unproven cause keeps. Evidence scales leverage but
/// must not erase it: a high-impact assumption is exactly the thing worth
/// testing, so it stays visible instead of sinking below well-evidenced trivia.
const UNPROVEN_FLOOR: f32 = 0.4;

/// How much a constraint's own evidence should count. An unproven cause is an
/// opinion — relaxing it is meaningless — so it is discounted but still listed;
/// a measured one is worth acting on. Never zero: impact alone is informative.
fn evidence_weight(status: VerificationStatus) -> f32 {
    match status {
        VerificationStatus::ProvenStrong => 1.0,
        VerificationStatus::Proven => 0.85,
        VerificationStatus::Partial => 0.6,
        VerificationStatus::Contested => 0.25,
        VerificationStatus::Unproven => 0.0,
        VerificationStatus::Invalid => 0.0,
    }
}

/// Vocabulary that *reads* like a limitation. Annotation only — see the field doc.
fn reads_as_limitation(label: &str) -> bool {
    const WORDS: [&str; 14] = [
        "cannot", "can not", "no inverse", "not supported", "unsupported",
        "blocked", "blocker", "limitation", "prevents", "impossible",
        "lacks", "missing", "forbidden", "refuses",
    ];
    let lower = label.to_lowercase();
    WORDS.iter().any(|w| lower.contains(w))
}

/// A ranking plus what the limit hid. A cap that reports nothing reads as
/// "that was everything", which is the one thing a leverage ranking must not
/// imply — the caller decides whether to look further.
#[derive(Serialize, Clone, Debug)]
pub struct ConstraintRanking {
    /// Causes that scored, before `limit` was applied.
    pub ranked: usize,
    /// The top `limit` of them.
    pub shown: Vec<ConstraintCandidate>,
}

impl ConstraintRanking {
    /// Causes the limit cut off.
    pub fn dropped(&self) -> usize {
        self.ranked.saturating_sub(self.shown.len())
    }
}

/// Rank causes by leverage. Pure read — computes nothing an LLM is needed for.
pub fn constraints(braim: &Braim, limit: usize, include_scratch: bool) -> ConstraintRanking {
    let elig: HashSet<u32> = eligible(braim, include_scratch).into_iter().collect();

    // because_of runs consequent -> cause, so invert it to walk a cause's blast
    // radius outward.
    let mut consequents: HashMap<u32, Vec<u32>> = HashMap::new();
    for e in &braim.state.because_of {
        // A refuted causal link (`why-test` failed) is not leverage: the
        // consequent no longer rests on this cause, even though both statements
        // are still valid. Graph::because_of_active_outgoing and the perspective
        // walk both skip these edges; leverage must agree with them.
        if e.invalid {
            continue;
        }
        if elig.contains(&e.from) && elig.contains(&e.to) {
            consequents.entry(e.to).or_default().push(e.from);
        }
    }
    for v in consequents.values_mut() {
        v.sort();
        v.dedup();
    }

    // Breadth-first over the consequent tree. The visited set makes this
    // cycle-safe: a causal loop is a graph defect, not a reason to hang.
    let reach = |root: u32| -> (usize, usize) {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut frontier = vec![root];
        let mut depth = 0usize;
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for id in frontier {
                for c in consequents.get(&id).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if *c != root && seen.insert(*c) {
                        next.push(*c);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            depth += 1;
            frontier = next;
        }
        (seen.len(), depth)
    };

    let mut causes: Vec<u32> = consequents.keys().copied().collect();
    causes.sort();

    let mut out: Vec<ConstraintCandidate> = causes
        .into_iter()
        .filter_map(|id| {
            let node = braim.state.nodes.get(&id)?;
            // Only statements carry causal meaning, and a refuted node is not a
            // constraint worth relaxing — it is already dead.
            if !node.node_type.is_statement_family()
                || node.invalid
                || node.verification_status == VerificationStatus::Invalid
            {
                return None;
            }
            let (impact, depth) = reach(id);
            if impact == 0 {
                return None;
            }
            Some(ConstraintCandidate {
                id,
                label: node.label.clone(),
                domains: node.domains.clone(),
                verification: node.verification_status.label().to_string(),
                impact,
                direct: consequents.get(&id).map(|v| v.len()).unwrap_or(0),
                depth,
                reads_as_limitation: reads_as_limitation(&node.label),
                score: 0.0,
                rationale: String::new(),
            })
        })
        .collect();

    let max_impact = out.iter().map(|c| c.impact).max().unwrap_or(1) as f32;
    for c in out.iter_mut() {
        let node = &braim.state.nodes[&c.id];
        let ev = evidence_weight(node.verification_status);
        // Impact sets the ceiling, evidence scales it. Both factors are <= 1 by
        // construction (impact <= max_impact, and the floor plus its complement
        // is exactly 1), so the product needs no clamp.
        c.score =
            (c.impact as f32 / max_impact) * (UNPROVEN_FLOOR + (1.0 - UNPROVEN_FLOOR) * ev);
        c.rationale = format!(
            "{} statement(s) rest on this ({} directly, {} level(s) deep); it is {}{}",
            c.impact,
            c.direct,
            c.depth,
            c.verification,
            if c.reads_as_limitation { ", and its wording reads as a limitation" } else { "" },
        );
    }

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.impact.cmp(&a.impact))
            .then(a.id.cmp(&b.id))
    });
    let ranked = out.len();
    out.truncate(limit);
    ConstraintRanking { ranked, shown: out }
}

/// Nodes eligible to be dreamed about: active concepts and statements that are
/// neither invalid nor transient agent scratch. Source entities are excluded —
/// they are provenance, not knowledge.
fn eligible(braim: &Braim, include_scratch: bool) -> Vec<u32> {
    let mut ids: Vec<u32> = braim
        .state
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.node_type != NodeType::Source
                && !n.invalid
                && n.verification_status != VerificationStatus::Invalid
                && (include_scratch
                    || n.metadata.get("scope").map(|s| s != "agent_scratch").unwrap_or(true))
        })
        .map(|(id, _)| *id)
        .collect();
    ids.sort();
    ids
}

fn link(adj: &mut HashMap<u32, HashMap<u32, f64>>, a: u32, b: u32, w: f64) {
    if a == b {
        return;
    }
    let ea = adj.entry(a).or_default().entry(b).or_insert(w);
    *ea = ea.max(w);
    let eb = adj.entry(b).or_default().entry(a).or_insert(w);
    *eb = eb.max(w);
}

/// Undirected "already related" graph, restricted to the eligible set.
///
/// Crucially this is NOT just depends_on. braim relates two concepts THROUGH a
/// statement — "Payment requires Invoice" makes Payment and Invoice related
/// without any edge between them. Treating only depends_on as connection would
/// offer every co-member pair as a discovery and, worse, make two-hop yield
/// statement-to-statement pairs instead of concept-to-concept ones. So every
/// pair of a statement's dependencies is linked at the weaker of the two weights.
fn adjacency(braim: &Braim, eligible: &HashSet<u32>) -> HashMap<u32, HashMap<u32, f64>> {
    let mut adj: HashMap<u32, HashMap<u32, f64>> = HashMap::new();
    let mut ids: Vec<&u32> = braim.state.nodes.keys().collect();
    ids.sort();
    for id in ids {
        if !eligible.contains(id) {
            continue;
        }
        let node = &braim.state.nodes[id];
        let mut deps: Vec<(u32, f64)> = node
            .depends_on
            .iter()
            .filter(|(d, _)| eligible.contains(d))
            .map(|(d, w)| (*d, *w))
            .collect();
        deps.sort_by_key(|(d, _)| *d);
        for (d, w) in &deps {
            link(&mut adj, *id, *d, *w);
        }
        for i in 0..deps.len() {
            for j in (i + 1)..deps.len() {
                // Co-membership is only as strong as its weaker leg.
                let w = deps[i].1.min(deps[j].1);
                link(&mut adj, deps[i].0, deps[j].0, w);
            }
        }
    }
    adj
}

pub struct DreamOptions {
    pub strategies: Vec<Strategy>,
    pub limit: usize,
    pub min_semantic: f32,
    pub include_scratch: bool,
    /// Re-dream pairs already in the ledger.
    pub replay: bool,
}

/// Build the ranked worklist. Pure read: never mutates the graph.
pub fn candidates(
    braim: &Braim,
    opts: &DreamOptions,
    semantic_pairs: &[(u32, u32, f32)],
) -> Vec<Candidate> {
    let elig_vec = eligible(braim, opts.include_scratch);
    let elig: HashSet<u32> = elig_vec.iter().copied().collect();
    let adj = adjacency(braim, &elig);

    let seen: HashSet<(u32, u32)> = if opts.replay {
        HashSet::new()
    } else {
        load_ledger(&braim.data_dir)
            .iter()
            .map(|e| pair_key(e.a, e.b))
            .collect()
    };

    // (score, strategy, rationale) accumulated per pair.
    let mut acc: HashMap<(u32, u32), (f32, Vec<&'static str>, Vec<String>)> = HashMap::new();
    let mut add = |acc: &mut HashMap<(u32, u32), (f32, Vec<&'static str>, Vec<String>)>,
                   a: u32,
                   b: u32,
                   score: f32,
                   s: Strategy,
                   why: String| {
        if a == b {
            return;
        }
        let key = pair_key(a, b);
        if seen.contains(&key) {
            return;
        }
        // Already directly connected: nothing to discover.
        if adj.get(&a).map(|m| m.contains_key(&b)).unwrap_or(false) {
            return;
        }
        let e = acc.entry(key).or_insert((0.0, Vec::new(), Vec::new()));
        e.0 = e.0.max(score.min(SINGLE_SIGNAL_CEILING));
        if !e.1.contains(&s.label()) {
            e.1.push(s.label());
            e.2.push(why);
        }
    };

    for strategy in &opts.strategies {
        match strategy {
            Strategy::SharedSource => {
                let mut by_source: HashMap<&str, Vec<u32>> = HashMap::new();
                for id in &elig_vec {
                    let node = &braim.state.nodes[id];
                    for s in &node.sources {
                        let is_primary = matches!(
                            s.split(':').next().unwrap_or(""),
                            "code" | "doc" | "schema" | "config" | "transcript" | "test"
                        );
                        if is_primary {
                            by_source.entry(s.as_str()).or_default().push(*id);
                        }
                    }
                }
                let mut sources: Vec<(&&str, &Vec<u32>)> = by_source.iter().collect();
                sources.sort_by_key(|(s, _)| **s);
                for (src, ids) in sources {
                    let fanout = ids.len();
                    if fanout < 2 || fanout > MAX_SOURCE_FANOUT {
                        continue;
                    }
                    // Rarer shared source ⇒ more specific coincidence ⇒ stronger.
                    let score = (2.0 / fanout as f32).min(1.0);
                    for i in 0..ids.len() {
                        for j in (i + 1)..ids.len() {
                            add(
                                &mut acc,
                                ids[i],
                                ids[j],
                                score,
                                Strategy::SharedSource,
                                format!("both cite {} but are not linked", src),
                            );
                        }
                    }
                }
            }
            Strategy::TwoHop => {
                let mut bridges: Vec<&u32> = adj.keys().collect();
                bridges.sort();
                for bridge in bridges {
                    let neighbours = &adj[bridge];
                    let degree = neighbours.len();
                    if degree < 2 || degree > MAX_BRIDGE_DEGREE {
                        continue;
                    }
                    let mut ns: Vec<(&u32, &f64)> = neighbours.iter().collect();
                    ns.sort_by_key(|(id, _)| **id);
                    for i in 0..ns.len() {
                        for j in (i + 1)..ns.len() {
                            let (a, wa) = ns[i];
                            let (b, wb) = ns[j];
                            // Strong edges through a NARROW bridge mean the two
                            // ends genuinely share that bridge's context.
                            let score =
                                ((*wa * *wb) as f32 * (2.0 / degree as f32)).clamp(0.0, 1.0);
                            let blabel = braim.state.nodes[bridge].label.clone();
                            add(
                                &mut acc,
                                *a,
                                *b,
                                score,
                                Strategy::TwoHop,
                                format!(
                                    "both connect to ID:{} '{}' but not to each other",
                                    bridge,
                                    truncate(&blabel, 60)
                                ),
                            );
                        }
                    }
                }
            }
            Strategy::Semantic => {
                for (a, b, cos) in semantic_pairs {
                    if *cos < opts.min_semantic || !elig.contains(a) || !elig.contains(b) {
                        continue;
                    }
                    // Structurally distant: not adjacent and sharing no neighbour.
                    let na = adj.get(a);
                    let nb = adj.get(b);
                    let shares_neighbour = match (na, nb) {
                        (Some(x), Some(y)) => x.keys().any(|k| y.contains_key(k)),
                        _ => false,
                    };
                    if shares_neighbour {
                        continue;
                    }
                    add(
                        &mut acc,
                        *a,
                        *b,
                        *cos,
                        Strategy::Semantic,
                        format!("labels are semantically close (cosine {:.2}) yet more than two hops apart", cos),
                    );
                }
            }
            Strategy::RegisteredGap => {
                for gap in &braim.state.gaps {
                    if !elig.contains(&gap.concept_a) || !elig.contains(&gap.concept_b) {
                        continue;
                    }
                    // A real query wanted this path and found none: the strongest
                    // signal available, because a human already cared.
                    add(
                        &mut acc,
                        gap.concept_a,
                        gap.concept_b,
                        1.0,
                        Strategy::RegisteredGap,
                        format!("registered gap: {}", truncate(&gap.note, 90)),
                    );
                }
            }
        }
    }

    let mut out: Vec<Candidate> = acc
        .into_iter()
        .filter_map(|((a, b), (score, strategies, whys))| {
            let na = braim.state.nodes.get(&a)?;
            let nb = braim.state.nodes.get(&b)?;
            // Agreement across independent signals is itself evidence.
            let bonus = 0.12 * (strategies.len().saturating_sub(1)) as f32;
            Some(Candidate {
                a,
                b,
                a_label: na.label.clone(),
                b_label: nb.label.clone(),
                a_domains: na.domains.clone(),
                b_domains: nb.domains.clone(),
                strategies,
                score: (score + bonus).min(1.0),
                rationale: whys.join("; "),
            })
        })
        .collect();

    out.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(x.a.cmp(&y.a))
            .then(x.b.cmp(&y.b))
    });
    out.truncate(opts.limit);
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;

    fn temp(name: &str) -> Braim {
        let dir = std::env::temp_dir().join(format!("braim_dream_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Braim::new(dir.to_str().unwrap()).unwrap()
    }

    fn opts(strategies: Vec<Strategy>) -> DreamOptions {
        DreamOptions {
            strategies,
            limit: 100,
            min_semantic: DEFAULT_MIN_SEMANTIC,
            include_scratch: false,
            replay: false,
        }
    }

    /// A statement with real evidence, so verification can be varied on purpose.
    fn stmt(b: &mut Braim, text: &str, deps: (u32, u32), sources: Vec<String>) -> u32 {
        b.add_statement(text, vec!["t".into()], sources,
            Map::from([(deps.0, 0.6), (deps.1, 0.4)]), true).unwrap()
    }

    #[test]
    fn leverage_counts_transitive_impact_not_just_direct() {
        let mut b = temp("leverage_transitive");
        let x = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let y = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();

        // deep: root <- mid <- leaf   (root has 1 direct, 2 transitive)
        let deep_root = stmt(&mut b, "deep root", (x, y), vec!["code:a.rs:1".into()]);
        let mid = stmt(&mut b, "mid", (x, y), vec!["code:b.rs:1".into()]);
        let leaf = stmt(&mut b, "leaf", (x, y), vec!["code:c.rs:1".into()]);
        b.why_add(mid, deep_root, Some("narrative:w".into())).unwrap();
        b.why_add(leaf, mid, Some("narrative:w".into())).unwrap();

        // wide: root <- one consequent only
        let wide_root = stmt(&mut b, "wide root", (x, y), vec!["code:d.rs:1".into()]);
        let only = stmt(&mut b, "only consequent", (x, y), vec!["code:e.rs:1".into()]);
        b.why_add(only, wide_root, Some("narrative:w".into())).unwrap();

        let out = constraints(&b, 10, false).shown;
        let deep = out.iter().find(|c| c.id == deep_root).expect("deep root ranked");
        let wide = out.iter().find(|c| c.id == wide_root).expect("wide root ranked");

        assert_eq!(deep.impact, 2, "impact is transitive, not just direct consequents");
        assert_eq!(deep.direct, 1);
        assert_eq!(deep.depth, 2);
        assert_eq!(wide.impact, 1);
        assert!(deep.score > wide.score, "more of the graph rests on the deeper chain");
        assert_eq!(out[0].id, deep_root, "ranked best-first");
        // A leaf carries nothing and is not a candidate at all.
        assert!(!out.iter().any(|c| c.id == leaf), "a cause with no consequents is not load-bearing");
    }

    #[test]
    fn evidence_discounts_an_unproven_cause_at_equal_impact() {
        let mut b = temp("leverage_evidence");
        let x = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let y = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();

        // Identical shape; only the cause's own evidence differs.
        let measured = stmt(&mut b, "measured cause", (x, y),
            vec!["code:a.rs:1".into(), "doc:a.md:1".into()]);   // proven
        let opinion = stmt(&mut b, "opinion cause", (x, y),
            vec!["narrative:hunch".into()]);                     // unproven
        let c1 = stmt(&mut b, "consequent one", (x, y), vec!["code:c.rs:1".into()]);
        let c2 = stmt(&mut b, "consequent two", (x, y), vec!["code:d.rs:1".into()]);
        b.why_add(c1, measured, Some("narrative:w".into())).unwrap();
        b.why_add(c2, opinion, Some("narrative:w".into())).unwrap();

        let out = constraints(&b, 10, false).shown;
        let m = out.iter().find(|c| c.id == measured).unwrap();
        let o = out.iter().find(|c| c.id == opinion).unwrap();
        assert_eq!(m.impact, o.impact, "same blast radius by construction");
        assert!(m.score > o.score, "a measured constraint outranks an equally load-bearing opinion");
        assert!(o.score > 0.0, "but the opinion stays visible — it may be the assumption worth testing");
    }

    #[test]
    fn refuted_causes_are_excluded_and_cycles_terminate() {
        let mut b = temp("leverage_edges");
        let x = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let y = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let dead = stmt(&mut b, "refuted cause", (x, y), vec!["code:a.rs:1".into()]);
        let live = stmt(&mut b, "live consequent", (x, y), vec!["code:b.rs:1".into()]);
        b.why_add(live, dead, Some("narrative:w".into())).unwrap();

        // A causal loop is a graph defect; ranking must terminate regardless.
        let p = stmt(&mut b, "loop p", (x, y), vec!["code:p.rs:1".into()]);
        let q = stmt(&mut b, "loop q", (x, y), vec!["code:q.rs:1".into()]);
        b.why_add(p, q, Some("narrative:w".into())).unwrap();
        b.state.because_of.push(crate::graph::BecauseOfEdge {
            from: q, to: p, source: None, created_at: String::new(),
            test_source: None, invalid: false, invalid_reason: None,
        });

        b.invalidate_statement(dead, "refuted").unwrap();
        let out = constraints(&b, 10, false).shown;   // must return, not hang
        assert!(!out.iter().any(|c| c.id == dead), "a refuted cause is already dead, not a constraint");
        assert!(out.iter().any(|c| c.id == p || c.id == q), "cyclic causes still rank");
    }

    #[test]
    fn the_limit_reports_what_it_hid() {
        let mut b = temp("leverage_limit");
        let x = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let y = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        for i in 0..3 {
            let cause = stmt(&mut b, &format!("cause number {}", i), (x, y), vec![format!("code:c{}.rs:1", i)]);
            let effect = stmt(&mut b, &format!("effect number {}", i), (x, y), vec![format!("code:e{}.rs:1", i)]);
            b.why_add(effect, cause, Some("narrative:w".into())).unwrap();
        }

        let r = constraints(&b, 1, false);
        assert_eq!(r.shown.len(), 1, "the limit is honoured");
        assert_eq!(r.ranked, 3, "but the full count is reported");
        assert_eq!(r.dropped(), 2, "so a cap never reads as completeness");

        let all = constraints(&b, 10, false);
        assert_eq!(all.dropped(), 0, "nothing hidden when the limit exceeds the ranking");
    }

    #[test]
    fn a_refuted_causal_edge_carries_no_leverage() {
        let mut b = temp("leverage_refuted_edge");
        let x = b.add_concept("Alpha: first", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let y = b.add_concept("Beta: second", vec!["t".into()], vec!["narrative:x".into()], None).unwrap();
        let cause = stmt(&mut b, "the load-bearing cause", (x, y), vec!["code:c.rs:1".into()]);
        let live = stmt(&mut b, "a consequent that still holds", (x, y), vec!["code:l.rs:1".into()]);
        let refuted = stmt(&mut b, "a consequent whose link was disproved", (x, y), vec!["code:r.rs:1".into()]);
        b.why_add(live, cause, Some("narrative:w".into())).unwrap();
        // why_add refuses a second cause per consequent, so the refuted edge is
        // placed directly — the shape `why-test` leaves behind on failure.
        b.state.because_of.push(crate::graph::BecauseOfEdge {
            from: refuted, to: cause, source: None, created_at: String::new(),
            test_source: None, invalid: true,
            invalid_reason: Some("inverse test failed".into()),
        });

        let out = constraints(&b, 10, false).shown;
        let c = out.iter().find(|c| c.id == cause).expect("the live link still makes it a cause");
        assert_eq!(c.impact, 1, "a disproved link must not inflate the blast radius");
        assert_eq!(c.direct, 1, "nor the direct count");
    }

    #[test]
    fn shared_source_pairs_unlinked_nodes_only() {
        let mut b = temp("shared_source");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["code:pay.rs:10".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["code:pay.rs:10".into()], None).unwrap();
        // third cites a different source → must not pair with either
        let _z = b.add_concept("Zeta: third", vec!["d".into()], vec!["code:other.rs:1".into()], None).unwrap();

        let out = candidates(&b, &opts(vec![Strategy::SharedSource]), &[]);
        assert_eq!(out.len(), 1, "only the co-citing pair qualifies");
        assert_eq!((out[0].a, out[0].b), (a.min(c), a.max(c)));
        assert!(out[0].rationale.contains("code:pay.rs:10"));

        // once linked by a statement, the pair is no longer a discovery
        b.add_statement("alpha relates to beta", vec!["d".into()], vec!["code:x.rs:1".into()],
            Map::from([(a, 0.6), (c, 0.4)]), true).unwrap();
        let out = candidates(&b, &opts(vec![Strategy::SharedSource]), &[]);
        assert!(
            !out.iter().any(|c2| (c2.a, c2.b) == (a.min(c), a.max(c))),
            "directly linked pairs must not be offered as candidates"
        );
    }

    #[test]
    fn two_hop_finds_the_missing_edge_and_skips_hubs() {
        let mut b = temp("two_hop");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        let bridge = b.add_concept("Gamma: bridge", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        // a → bridge and c → bridge, but a and c never meet
        b.add_statement("alpha uses gamma", vec!["d".into()], vec!["code:a.rs:1".into()],
            Map::from([(a, 0.5), (bridge, 0.5)]), true).unwrap();
        b.add_statement("beta uses gamma", vec!["d".into()], vec!["code:b.rs:1".into()],
            Map::from([(c, 0.5), (bridge, 0.5)]), true).unwrap();

        let out = candidates(&b, &opts(vec![Strategy::TwoHop]), &[]);
        assert!(
            out.iter().any(|x| (x.a, x.b) == (a.min(c), a.max(c))),
            "a and c share a bridge and must be offered: {:?}",
            out.iter().map(|x| (x.a, x.b)).collect::<Vec<_>>()
        );
        assert!(out.iter().all(|x| x.strategies.contains(&"two-hop")));
    }

    #[test]
    fn ledger_excludes_adjudicated_pairs_unless_replayed() {
        let mut b = temp("ledger");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["code:same.rs:1".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["code:same.rs:1".into()], None).unwrap();

        assert_eq!(candidates(&b, &opts(vec![Strategy::SharedSource]), &[]).len(), 1);
        record_ledger(&b.data_dir, a, c, "no-relation", Some("unrelated".into())).unwrap();
        assert!(
            candidates(&b, &opts(vec![Strategy::SharedSource]), &[]).is_empty(),
            "an adjudicated pair must not be offered again"
        );
        // order-independent: the ledger keys on the unordered pair
        record_ledger(&b.data_dir, c, a, "no-relation", None).unwrap();
        assert_eq!(load_ledger(&b.data_dir).len(), 1, "re-recording must update, not duplicate");

        let mut replay = opts(vec![Strategy::SharedSource]);
        replay.replay = true;
        assert_eq!(candidates(&b, &replay, &[]).len(), 1, "--replay reconsiders seen pairs");
    }

    #[test]
    fn rejects_bad_verdicts_and_unknown_strategies() {
        let b = temp("validation");
        assert!(record_ledger(&b.data_dir, 1, 2, "maybe", None).is_err());
        assert!(record_ledger(&b.data_dir, 1, 2, "verified", None).is_ok());
        assert!(Strategy::parse("nonsense").is_err());
        assert_eq!(Strategy::parse("two_hop").unwrap(), Strategy::TwoHop);
    }

    #[test]
    fn central_marker_refuses_dreaming() {
        let b = temp("central");
        assert!(refuse_if_central(&b.data_dir).is_ok());
        std::fs::write(b.data_dir.join(CENTRAL_MARKER), "central").unwrap();
        let err = refuse_if_central(&b.data_dir).unwrap_err();
        assert!(err.contains("marked central"), "got: {}", err);
    }

    #[test]
    fn scratch_and_invalid_nodes_are_excluded() {
        let mut b = temp("exclusions");
        let a = b.add_concept("Alpha: first", vec!["d".into()], vec!["code:same.rs:1".into()], None).unwrap();
        let c = b.add_concept("Beta: second", vec!["d".into()], vec!["code:same.rs:1".into()], None).unwrap();
        assert_eq!(candidates(&b, &opts(vec![Strategy::SharedSource]), &[]).len(), 1);

        b.set_meta(c, "scope", "agent_scratch").unwrap();
        assert!(
            candidates(&b, &opts(vec![Strategy::SharedSource]), &[]).is_empty(),
            "transient agent scratch is not dream material"
        );
        let mut with_scratch = opts(vec![Strategy::SharedSource]);
        with_scratch.include_scratch = true;
        assert_eq!(candidates(&b, &with_scratch, &[]).len(), 1, "--include-scratch overrides");
        let _ = a;
    }

    #[test]
    fn multi_strategy_agreement_scores_above_single_signal() {
        let mut b = temp("agreement");
        // pair 1: shared source only
        let p = b.add_concept("Pone: p", vec!["d".into()], vec!["code:one.rs:1".into()], None).unwrap();
        let q = b.add_concept("Qone: q", vec!["d".into()], vec!["code:one.rs:1".into()], None).unwrap();
        // pair 2: shared source AND a two-hop bridge
        let x = b.add_concept("Xtwo: x", vec!["d".into()], vec!["code:two.rs:1".into()], None).unwrap();
        let y = b.add_concept("Ytwo: y", vec!["d".into()], vec!["code:two.rs:1".into()], None).unwrap();
        let bridge = b.add_concept("Bridge: shared", vec!["d".into()], vec!["narrative:x".into()], None).unwrap();
        b.add_statement("x uses bridge", vec!["d".into()], vec!["code:x.rs:1".into()],
            Map::from([(x, 0.5), (bridge, 0.5)]), true).unwrap();
        b.add_statement("y uses bridge", vec!["d".into()], vec!["code:y.rs:1".into()],
            Map::from([(y, 0.5), (bridge, 0.5)]), true).unwrap();

        let out = candidates(&b, &opts(vec![Strategy::SharedSource, Strategy::TwoHop]), &[]);
        let two_sig = out.iter().find(|c| (c.a, c.b) == (x.min(y), x.max(y))).expect("xy pair present");
        let one_sig = out.iter().find(|c| (c.a, c.b) == (p.min(q), p.max(q))).expect("pq pair present");
        assert_eq!(two_sig.strategies.len(), 2, "xy nominated by both strategies");
        assert!(
            two_sig.score > one_sig.score,
            "corroborating signals must outrank a single one ({} vs {})",
            two_sig.score,
            one_sig.score
        );
        assert_eq!(out[0].a, two_sig.a, "worklist is sorted best-first");
    }
}
