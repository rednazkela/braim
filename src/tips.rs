use crate::graph::{Braim, Node, VerificationStatus};

pub fn emit_tip_statement_add(node: &Node, braim: &Braim, quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    if node.depends_on.len() >= 2 {
        let weights: Vec<_> = node.depends_on.values().collect();
        if weights.windows(2).all(|w| (w[0] - w[1]).abs() < 0.001) {
            let weight = weights[0];
            eprintln!(
                "braim tip: dependencies have equal weight ({}×{:.2}). If one is more central, see DEPENDENCY WEIGHTS in --help.",
                node.depends_on.len(),
                weight
            );
            return;
        }
    }

    let primary_count = node
        .sources
        .iter()
        .filter(|s| {
            let (source_type, _) = Braim::parse_source(s);
            source_type.tier() == "PRIMARY"
        })
        .count();

    if primary_count == 0 {
        eprintln!(
            "braim tip: this is a claim (unproven). Run 'braim statement verify-suggest {}' for promotion candidates.",
            node.id
        );
        return;
    }

    let source_derived = Braim::calculate_verification_status_from_sources(&node.sources);
    if source_derived != node.verification_status {
        let mut min_status = source_derived.clone();
        let mut weakest_dep_id = 0u32;

        for (&dep_id, _) in &node.depends_on {
            if let Some(dep) = braim.get_node(dep_id) {
                if status_rank(&dep.verification_status) < status_rank(&min_status) {
                    min_status = dep.verification_status.clone();
                    weakest_dep_id = dep_id;
                }
            }
        }

        eprintln!(
            "braim tip: source-derived status was {:?}; capped to {:?} by weakest dep ID:{}. Verify the dep to upgrade.",
            source_derived, node.verification_status, weakest_dep_id
        );
        return;
    }

    if node.verification_status == VerificationStatus::ProvenStrong {
        eprintln!("braim tip: maximum verification reached (3+ PRIMARY types).");
        return;
    }
}

pub fn emit_tip_invalidate(cascaded_ids: &[u32], quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    if cascaded_ids.len() >= 3 {
        eprintln!(
            "braim tip: {} dependents also became invalid. Use 'braim query <term> --include-invalid' to audit them.",
            cascaded_ids.len()
        );
    }
}

pub fn emit_tip_query_no_results(include_claims: bool, quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    if !include_claims {
        eprintln!("braim tip: default returns facts only. Try 'braim query <term> --include-claims' for unverified statements.");
    } else {
        eprintln!("braim tip: no matches. Check 'braim domains' for the right domain or 'braim list' to browse.");
    }
}

pub fn emit_tip_concept_add(node: &Node, quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    if node.depends_on.len() == 1 {
        eprintln!("braim tip: compound with 1 dependency is structurally an atomic concept. Consider 'concept add' without --depends.");
        return;
    }

    let primary_count = node
        .sources
        .iter()
        .filter(|s| {
            let (source_type, _) = Braim::parse_source(s);
            source_type.tier() == "PRIMARY"
        })
        .count();

    if primary_count == 0 {
        eprintln!("braim tip: concept has no PRIMARY-typed source. Will not anchor downstream verification.");
        return;
    }
}

fn tip_disabled() -> bool {
    std::env::var("BRAIM_NO_TIPS").is_ok()
}

fn status_rank(status: &VerificationStatus) -> u8 {
    match status {
        VerificationStatus::Invalid => 0,
        VerificationStatus::Unproven => 1,
        VerificationStatus::Partial => 2,
        VerificationStatus::Proven => 3,
        VerificationStatus::ProvenStrong => 4,
    }
}
