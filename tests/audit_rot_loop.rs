//! Integration coverage for the audit rot counter (braim-audit-rot-loop.md).
//!
//! The counter is deliberately NOT a field on the graph: it is a query over
//! the review queue that already sits beside it, so these tests assert the
//! query's behaviour and that `current.json` stays untouched by it.
//!
//! No `[lib]` target exists for this crate (see Cargo.toml), so every
//! assertion here drives the real compiled binary via subprocess — the same
//! convention tests/concurrency.rs and tests/contradiction_scoped_resolution.rs
//! already use.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_braim");

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("braim_arl_{}_{}", test, std::process::id()));
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

fn braim(dir: &str, args: &[&str]) -> (bool, String) {
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
    (out.status.success(), combined)
}

/// Two concepts and a statement, so there are stored nodes a finding can name.
fn seed(dir: &str) {
    let (ok, out) = braim(dir, &["concept", "add", "Alpha: a seeded concept",
        "--domains", "t", "--sources", "code:a.rs:1"]);
    assert!(ok, "seed concept 1: {}", out);
    let (ok, out) = braim(dir, &["concept", "add", "Beta: another seeded concept",
        "--domains", "t", "--sources", "code:b.rs:1"]);
    assert!(ok, "seed concept 2: {}", out);
    let (ok, out) = braim(dir, &["statement", "add", "Alpha relates to Beta",
        "--domains", "t", "--sources", "code:a.rs:1,doc:b.md:1",
        "--depends", "1:0.6,2:0.4"]);
    assert!(ok, "seed statement: {}", out);
}

/// The first line of `--count` is what the Stop hook reads with awk.
fn tally(dir: &str) -> u32 {
    let (ok, out) = braim(dir, &["dream", "review", "--count"]);
    assert!(ok, "review --count failed: {}", out);
    out.lines()
        .find_map(|l| l.strip_prefix("audit_pending "))
        .expect("--count must print an audit_pending line first")
        .trim()
        .parse()
        .expect("audit_pending must carry a number")
}

#[test]
fn an_empty_queue_reports_zero_rot_with_every_kind_named() {
    let s = Scratch::new("empty");
    seed(s.dir());
    assert_eq!(tally(s.dir()), 0);
    let (_, out) = braim(s.dir(), &["dream", "review", "--count"]);
    for kind in ["anchor", "reground", "independence", "unsupported"] {
        assert!(out.contains(kind), "--count must name {} even at zero: {}", kind, out);
    }
}

#[test]
fn only_the_four_audit_kinds_count_toward_rot() {
    let s = Scratch::new("kinds");
    seed(s.dir());

    // The queue's own kinds are review-worthy but are not rot: they describe
    // what a night noticed, not a defect sitting in a stored node.
    for kind in ["note", "merge", "unraised", "duplicate", "rate"] {
        let (ok, out) = braim(s.dir(), &["dream", "flag", "not a rot finding",
            "--kind", kind, "--nodes", "1"]);
        assert!(ok, "flag {}: {}", kind, out);
    }
    assert_eq!(tally(s.dir()), 0, "non-audit kinds must not raise the rot counter");

    for kind in ["anchor", "reground", "independence", "unsupported"] {
        let (ok, out) = braim(s.dir(), &["dream", "flag", "a real defect in a stored node",
            "--kind", kind, "--nodes", "1"]);
        assert!(ok, "flag {}: {}", kind, out);
    }
    assert_eq!(tally(s.dir()), 4);
}

#[test]
fn the_counter_falls_when_a_finding_is_signed_off() {
    let s = Scratch::new("falls");
    seed(s.dir());
    for _ in 0..3 {
        braim(s.dir(), &["dream", "flag", "anchor drifted", "--kind", "anchor", "--nodes", "1"]);
    }
    assert_eq!(tally(s.dir()), 3);

    let (ok, out) = braim(s.dir(), &["dream", "reviewed", "1", "--note", "re-anchored"]);
    assert!(ok, "sign-off: {}", out);
    assert_eq!(tally(s.dir()), 2, "a signed-off finding stops counting");

    // Cleared items are kept, so the queue can still be audited.
    let (_, all) = braim(s.dir(), &["dream", "review", "--all"]);
    assert!(all.contains("cleared"), "a cleared item must remain readable: {}", all);
}

#[test]
fn a_finding_must_name_a_node_that_exists() {
    let s = Scratch::new("exists");
    seed(s.dir());
    let (ok, out) = braim(s.dir(), &["dream", "flag", "about a node that is not there",
        "--kind", "anchor", "--nodes", "999"]);
    assert!(!ok, "flagging a nonexistent node must fail: {}", out);
    assert_eq!(tally(s.dir()), 0);
}

#[test]
fn the_counter_is_a_query_not_a_field_on_the_graph() {
    let s = Scratch::new("nofield");
    seed(s.dir());
    let before = fs::read_to_string(PathBuf::from(s.dir()).join("current.json")).unwrap();

    for _ in 0..5 {
        braim(s.dir(), &["dream", "flag", "rot", "--kind", "reground", "--nodes", "2"]);
    }
    assert_eq!(tally(s.dir()), 5);

    let after = fs::read_to_string(PathBuf::from(s.dir()).join("current.json")).unwrap();
    assert_eq!(before, after, "raising rot findings must not touch current.json");
    assert!(
        PathBuf::from(s.dir()).join("reviews.json").exists(),
        "the counter lives in reviews.json beside the graph"
    );
}

#[test]
fn the_json_tally_carries_every_kind_and_a_total() {
    let s = Scratch::new("json");
    seed(s.dir());
    braim(s.dir(), &["dream", "flag", "rot", "--kind", "anchor", "--nodes", "1"]);
    braim(s.dir(), &["dream", "flag", "rot", "--kind", "independence", "--nodes", "2"]);

    let (ok, out) = braim(s.dir(), &["dream", "review", "--count", "--json"]);
    assert!(ok, "--count --json failed: {}", out);
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("must be valid JSON");
    assert_eq!(v["anchor"], 1);
    assert_eq!(v["independence"], 1);
    assert_eq!(v["reground"], 0);
    assert_eq!(v["unsupported"], 0);
    assert_eq!(v["total"], 2);
}
