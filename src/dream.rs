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
    /// no-relation | proposed | verified | contradiction
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
    const VERDICTS: [&str; 4] = ["no-relation", "proposed", "verified", "contradiction"];
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
