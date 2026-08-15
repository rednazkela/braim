//! Integration coverage for the scoped contradiction resolution feature
//! (braim-contradiction-scoped-resolution.md), fixtured on the real 179/235
//! incident: two statements citing the same boolean-flag-gated function under
//! different modes, wrongly treated as a winner/loser disagreement.
//!
//! No `[lib]` target exists for this crate (see Cargo.toml), so every
//! assertion here drives the real compiled binary via subprocess — the same
//! convention tests/concurrency.rs already uses. That also makes this file
//! double as the CLI-reachability proof: these calls exercise the exact same
//! entry point a user would.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_braim");

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("braim_csr_{}_{}", test, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn dir(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn braim(dir: &str, args: &[&str]) -> (bool, i32, String) {
    let out = Command::new(BIN)
        .arg("--data-dir")
        .arg(dir)
        .arg("--quiet")
        .args(args)
        .output()
        .expect("failed to run braim");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), out.status.code().unwrap_or(-1), combined)
}

fn current_json(dir: &str) -> Value {
    let text = fs::read_to_string(PathBuf::from(dir).join("current.json"))
        .expect("current.json must exist");
    serde_json::from_str(&text).expect("current.json must be valid JSON")
}

fn contradicts_edge<'a>(state: &'a Value, a: u32, b: u32) -> &'a Value {
    state["contradicts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            let (from, to) = (e["from"].as_u64().unwrap() as u32, e["to"].as_u64().unwrap() as u32);
            (from == a && to == b) || (from == b && to == a)
        })
        .expect("contradicts edge must exist between the two statements")
}

/// D1 + D2 + D3 end to end, fixtured on the 179/235 case: two statements
/// about the same boolean-flag-gated function are contested, one is
/// corroborated by a third PRIMARY source (must NOT auto-invalidate the
/// other — D2), then explicitly resolved --both-stand (D1), and `braim node`
/// shows the scoped resolution kind distinctly from a winner/loser one (D3).
#[test]
fn scoped_resolution_end_to_end() {
    let scratch = Scratch::new("end_to_end");
    let dir = scratch.dir();

    // Two concepts, IDs 1 and 2 (fresh graph, deterministic sequential IDs).
    let (ok, _, out) = braim(dir, &["concept", "add", "Boolean Flag: gates import behavior",
        "--domains", "test", "--sources", "narrative:x"]);
    assert!(ok, "{}", out);
    let (ok, _, out) = braim(dir, &["concept", "add", "Import Function: source-union routine",
        "--domains", "test", "--sources", "narrative:x"]);
    assert!(ok, "{}", out);

    // s1 (ID 3): "default import discards duplicate sources" — true when the flag is off.
    let (ok, _, out) = braim(dir, &["statement", "add", "default import discards duplicate sources",
        "--domains", "test", "--sources", "code:graph.rs:100", "--depends", "1:0.6,2:0.4"]);
    assert!(ok, "{}", out);
    // s2 (ID 4): "full-mode import unions duplicate sources" — true when the flag is on.
    let (ok, _, out) = braim(dir, &["statement", "add", "full-mode import unions duplicate sources",
        "--domains", "test", "--sources", "code:graph.rs:200", "--depends", "1:0.6,2:0.4"]);
    assert!(ok, "{}", out);
    let (s1, s2) = (3u32, 4u32);

    let (ok, _, out) = braim(dir, &["statement", "contradict", &s1.to_string(), &s2.to_string(),
        "--reason", "these look mutually exclusive"]);
    assert!(ok, "{}", out);

    let before = current_json(dir);
    let before_s2 = before["nodes"][s2.to_string()].clone();

    // D2: corroborate s1 with a third PRIMARY source (source entity ID 5).
    let (ok, _, out) = braim(dir, &["source", "add", "spec confirming the gate",
        "--type", "doc", "--location", "doc:spec.md:12"]);
    assert!(ok, "{}", out);
    let source_id = 5u32;
    let (ok, _, out) = braim(dir, &["statement", "add-source", &s1.to_string(), "--source-id", &source_id.to_string()]);
    assert!(ok, "{}", out);
    assert!(out.contains("Corroboration reached"), "must report the corroboration, got: {}", out);
    assert!(out.contains(&format!("ID:{}", s2)), "must name the other statement, got: {}", out);
    assert!(!out.contains("Auto-resolved"), "must NOT auto-resolve, got: {}", out);

    let after_corroboration = current_json(dir);
    assert_eq!(after_corroboration["nodes"][s2.to_string()], before_s2,
        "the uncorroborated side must be byte-identical after report-only Mechanism A");
    assert_eq!(after_corroboration["nodes"][s1.to_string()]["verification_status"], "contested",
        "the corroborated side must stay contested, not auto-promoted");
    let edge = contradicts_edge(&after_corroboration, s1, s2);
    assert_eq!(edge["resolved"], false, "the edge must remain unresolved after mere corroboration");

    // D1: explicit --both-stand resolution.
    let (ok, _, out) = braim(dir, &["statement", "resolve-contradiction", &s1.to_string(), &s2.to_string(),
        "--both-stand", "--reason", "different modes of the same function, not a disagreement"]);
    assert!(ok, "{}", out);
    assert!(out.contains("both stand"), "got: {}", out);

    let after_resolution = current_json(dir);
    assert_eq!(after_resolution["nodes"][s1.to_string()]["verification_status"], "contested",
        "both-stand must not touch s1's verification_status");
    assert_eq!(after_resolution["nodes"][s2.to_string()]["verification_status"], "contested",
        "both-stand must not touch s2's verification_status");
    assert_eq!(after_resolution["nodes"][s2.to_string()]["invalid"], false,
        "both-stand must never invalidate either side");
    let edge = contradicts_edge(&after_resolution, s1, s2);
    assert_eq!(edge["resolved"], true);
    assert_eq!(edge["resolution_kind"], "scoped");
    assert!(edge["resolution_winner"].is_null(), "a scoped resolution picks no winner");

    // D3: `braim node` distinguishes the scoped resolution kind.
    let (ok, _, out) = braim(dir, &["node", &s1.to_string()]);
    assert!(ok, "{}", out);
    assert!(out.contains("resolution_kind: scoped"), "braim node must surface resolution_kind, got: {}", out);
}

/// D1: a winner-resolved contradiction shows resolution_kind=winner via
/// `braim node`, distinguishable from the scoped case above (D3).
#[test]
fn winner_resolution_shows_resolution_kind_winner() {
    let scratch = Scratch::new("winner_kind");
    let dir = scratch.dir();

    braim(dir, &["concept", "add", "Alpha: first", "--domains", "test", "--sources", "narrative:x"]);
    braim(dir, &["concept", "add", "Beta: second", "--domains", "test", "--sources", "narrative:x"]);
    braim(dir, &["statement", "add", "claim one", "--domains", "test", "--sources", "code:a.rs:1", "--depends", "1:0.6,2:0.4"]);
    braim(dir, &["statement", "add", "claim two", "--domains", "test", "--sources", "code:b.rs:1", "--depends", "1:0.6,2:0.4"]);
    let (s1, s2) = (3u32, 4u32);
    braim(dir, &["statement", "contradict", &s1.to_string(), &s2.to_string(), "--reason", "conflict"]);

    let (ok, _, out) = braim(dir, &["statement", "resolve-contradiction", &s1.to_string(), &s2.to_string(),
        "--winner", &s1.to_string(), "--reason", "spec confirms claim one"]);
    assert!(ok, "{}", out);

    let (ok, _, out) = braim(dir, &["node", &s2.to_string()]);
    assert!(ok, "{}", out);
    assert!(out.contains("resolution_kind: winner"), "got: {}", out);
}

/// D5 (reachability proof): the real compiled binary's clap parsing — not
/// application logic — rejects --winner and --both-stand together, and never
/// reaches resolve_contradiction / resolve_contradiction_both_stand.
#[test]
fn winner_and_both_stand_together_is_a_clap_usage_error() {
    let scratch = Scratch::new("clap_conflict");
    let dir = scratch.dir();

    let (ok, code, out) = braim(dir, &["statement", "resolve-contradiction", "1", "2",
        "--winner", "1", "--both-stand", "--reason", "x"]);
    assert!(!ok, "must be rejected");
    assert_eq!(code, 2, "clap usage errors exit 2, distinct from the application's exit(1) on Err");
    assert!(!out.contains("no active contradiction edge"),
        "must never reach application logic, got: {}", out);
}
