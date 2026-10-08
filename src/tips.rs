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

pub fn emit_tip_duplicate_sources(dups: &[String], quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    let dup_list = dups
        .iter()
        .map(|d| format!("\"{}\"", d))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "⚠ duplicate source entries detected: [{}]. Consider using distinct citations (line numbers, sections) per source slot.",
        dup_list
    );
}

pub fn emit_tip_primary_tertiary_mix(quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    eprintln!(
        "⚠ source taxonomy mix: PRIMARY (doc:, code:, etc.) and TERTIARY (inference:, logic:) on the same statement. Inference is a derivation, not evidence — prefer PRIMARY-only sources here, and record reasoning in label or as a separate inference-only statement that --depends on this one."
    );
}

pub fn emit_tip_duplicate_domains(counts: &std::collections::HashMap<String, usize>, quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }

    let dup_list = counts
        .iter()
        .filter(|&(_, &count)| count > 1)
        .map(|(domain, &count)| format!("\"{}\"×{}", domain, count))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "⚠ duplicate domain entries detected: [{}]. Consider using distinct domains (e.g. \"library,operations,finance\") for clearer categorization.",
        dup_list
    );
}

pub fn emit_tip_decomposable_compound(
    label: &str,
    atomics: &[(u32, String)],
    dep_spec: &str,
    quiet: bool,
) {
    if quiet || tip_disabled() {
        return;
    }

    let atomic_names = atomics
        .iter()
        .map(|(_, name)| format!("'{}'", name))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "⚠ label '{}' contains existing atomic names: {}. Consider adding this as a compound depending on those atomics: braim concept add '{}' --depends '{}'",
        label, atomic_names, label, dep_spec
    );
}

fn tip_disabled() -> bool {
    std::env::var("BRAIM_NO_TIPS").is_ok()
}

fn status_rank(status: &VerificationStatus) -> u8 {
    match status {
        VerificationStatus::Invalid => 0,
        VerificationStatus::Unproven => 1,
        VerificationStatus::Contested => 2,
        VerificationStatus::Partial => 3,
        VerificationStatus::Proven => 4,
        VerificationStatus::ProvenStrong => 5,
    }
}

// ---------------------------------------------------------------------------
// SIC ID:608 — the store follows the shell's cwd.
// ---------------------------------------------------------------------------
// `--data-dir` defaults to the RELATIVE ".braim", so a `cd` anywhere in a
// command chain silently re-points the graph. On 2026-09-11 that wrote nodes
// 28-36 into /home/magnimus/sonar/.braim (27 nodes) instead of the session
// store at /home/magnimus/sonar/sonar/.braim (390 nodes), and nothing in any
// output said which store had been used.
//
// BRAIM_DATA_DIR (wired on the clap arg) is the fix. This is the detector for
// the case where it is NOT set: if the cwd chain holds more than one .braim,
// name the alternates so the wrong one is visible on the first command rather
// than after nine writes.

/// Other `.braim` directories reachable from `cwd` — its ancestors, and its
/// immediate children one level down. Pure so it is testable without touching
/// the process cwd or the environment.
pub fn ambiguous_stores(cwd: &std::path::Path) -> Vec<std::path::PathBuf> {
    let resolved = cwd.join(".braim");
    let mut found = Vec::new();

    for ancestor in cwd.ancestors().skip(1) {
        let candidate = ancestor.join(".braim");
        if candidate.is_dir() && candidate != resolved {
            found.push(candidate);
        }
    }

    if let Ok(entries) = std::fs::read_dir(cwd) {
        let mut children: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.file_name().map(|n| n != ".braim").unwrap_or(false))
            .map(|p| p.join(".braim"))
            .filter(|p| p.is_dir())
            .collect();
        children.sort();
        found.append(&mut children);
    }

    found
}

/// Warn when the store was resolved from the cwd-relative default and the cwd
/// chain offers another one. Silent when `--data-dir` or BRAIM_DATA_DIR pinned
/// it, and silent when there is nothing to confuse it with.
pub fn emit_tip_ambiguous_store(data_dir: &str, quiet: bool) {
    if quiet || tip_disabled() {
        return;
    }
    // Only the unpinned default is ambiguous. An explicit path — from the flag
    // or from BRAIM_DATA_DIR — is a deliberate choice and needs no warning.
    if data_dir != ".braim" || std::env::var("BRAIM_DATA_DIR").is_ok() {
        return;
    }

    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(_) => return,
    };
    let others = ambiguous_stores(&cwd);
    if others.is_empty() {
        return;
    }

    eprintln!(
        "braim tip: store resolved from cwd to {}. Also present: {}. Set BRAIM_DATA_DIR to pin one.",
        cwd.join(".braim").display(),
        others
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[cfg(test)]
mod ambiguous_store_tests {
    use super::ambiguous_stores;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("braim-sic608-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lone_store_is_not_ambiguous() {
        let root = scratch("lone");
        fs::create_dir_all(root.join("work/.braim")).unwrap();
        assert!(ambiguous_stores(&root.join("work")).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn child_store_is_reported() {
        // The exact 2026-09-11 shape: cwd is the parent, the intended store is
        // one level down, and both exist.
        let root = scratch("child");
        fs::create_dir_all(root.join("sonar/.braim")).unwrap();
        fs::create_dir_all(root.join("sonar/sonar/.braim")).unwrap();
        let found = ambiguous_stores(&root.join("sonar"));
        assert_eq!(found, vec![root.join("sonar/sonar/.braim")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ancestor_store_is_reported() {
        let root = scratch("ancestor");
        fs::create_dir_all(root.join("sonar/.braim")).unwrap();
        fs::create_dir_all(root.join("sonar/sonar/.braim")).unwrap();
        let found = ambiguous_stores(&root.join("sonar/sonar"));
        assert_eq!(found, vec![root.join("sonar/.braim")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_resolved_store_is_never_its_own_alternate() {
        let root = scratch("self");
        fs::create_dir_all(root.join("a/.braim")).unwrap();
        let found = ambiguous_stores(&root.join("a"));
        assert!(!found.contains(&root.join("a/.braim")));
        let _ = fs::remove_dir_all(&root);
    }
}
