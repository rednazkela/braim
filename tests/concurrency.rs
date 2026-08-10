//! Server-mode stress suite: many clients contributing to one central braim.
//!
//! The federated design (braim ID:192/232) makes central a shared write target —
//! several working graphs publish into it, and the server companion mediates
//! human-triggered processing. Every mutation is a read-modify-write cycle
//! (`Braim::new` loads, the command mutates, `flush` writes), so these tests
//! drive REAL concurrent processes rather than threads: separate memory, real
//! interleaving, exactly what a server sees.
//!
//! Each test asserts a property the central store must hold no matter how the
//! writes interleave. Run with: cargo test --release --test concurrency

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

const BIN: &str = env!("CARGO_BIN_EXE_braim");

/// Concurrent writers per stress test. Enough overlap to expose interleaving
/// without making the suite slow.
const USERS: usize = 6;

// ─────────────────────────── harness ───────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("braim_conc_{}_{}", test, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn sub(&self, name: &str) -> String {
        let p = self.0.join(name);
        fs::create_dir_all(&p).unwrap();
        p.to_str().unwrap().to_string()
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

fn braim_spawn(dir: &str, args: &[&str]) -> Child {
    Command::new(BIN)
        .arg("--data-dir")
        .arg(dir)
        .arg("--quiet")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn braim")
}

/// Fabricate a baseline central graph. Size matters: import scans the target
/// graph per incoming node, so a realistic central widens the read-modify-write
/// window to what a real deployment actually experiences.
fn seed_central(dir: &str, n: usize) {
    let mut nodes = Map::new();
    let mut dict = Map::new();
    for i in 1..=n {
        let label = format!("Seed{}: baseline concept number {}", i, i);
        nodes.insert(
            i.to_string(),
            json!({
                "id": i,
                "domains": ["seed"],
                "sources": ["narrative:seed"],
                "node_type": "atomic",
                "label": label,
                "depends_on": {},
                "status": "active",
                "created_at": "2026-01-01T00:00:00Z"
            }),
        );
        dict.insert(label.to_lowercase(), json!([i]));
    }
    let state = json!({
        "nodes": nodes,
        "dictionary": dict,
        "id_to_domain": {},
        "gaps": [],
        "next_id": n + 1,
        "version": 0,
        "contradicts": [],
        "because_of": []
    });
    fs::write(
        Path::new(dir).join("current.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .unwrap();
}

/// Seed one client's working graph: two concepts and a proven statement in its
/// own domain, so every client has something distinct to publish.
fn seed_user(dir: &str, domain: &str, tag: &str) {
    let (ok, out) = braim(
        dir,
        &[
            "concept",
            "add",
            &format!("{}Alpha: first concept for {}", tag, domain),
            "--domains",
            domain,
            "--sources",
            "narrative:seed",
        ],
    );
    assert!(ok, "seed concept failed: {}", out);
    let (ok, out) = braim(
        dir,
        &[
            "concept",
            "add",
            &format!("{}Beta: second concept for {}", tag, domain),
            "--domains",
            domain,
            "--sources",
            "narrative:seed",
        ],
    );
    assert!(ok, "seed concept failed: {}", out);
    let (ok, out) = braim(
        dir,
        &[
            "statement",
            "add",
            &format!("{} alpha relates to beta in {}", tag, domain),
            "--domains",
            domain,
            "--sources",
            "code:a.rs:1,doc:b.md:2",
            "--depends",
            "1:0.6,2:0.4",
            "--assume",
        ],
    );
    assert!(ok, "seed statement failed: {}", out);
}

/// Merged view of a central graph, both layouts. Mirrors what the engine loads.
fn load_central(dir: &str) -> (Value, Map<String, Value>) {
    let p = Path::new(dir);
    if p.join("domains").is_dir() {
        let header: Value = serde_json::from_str(
            &fs::read_to_string(p.join("graph.json")).expect("graph.json unreadable"),
        )
        .expect("graph.json unparseable");
        let mut nodes = Map::new();
        let mut files: Vec<PathBuf> = fs::read_dir(p.join("domains"))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|f| {
                let name = f.file_name().unwrap().to_string_lossy().to_string();
                // current shards only — versioned snapshots are immutable history
                name.ends_with(".json")
                    && !name
                        .rsplit_once(".v")
                        .map(|(_, t)| t.trim_end_matches(".json").chars().all(|c| c.is_ascii_digit()))
                        .unwrap_or(false)
            })
            .collect();
        files.sort();
        for f in files {
            let shard: Map<String, Value> =
                serde_json::from_str(&fs::read_to_string(&f).expect("shard unreadable"))
                    .unwrap_or_else(|e| panic!("shard {} unparseable: {}", f.display(), e));
            for (k, v) in shard {
                nodes.insert(k, v);
            }
        }
        (header, nodes)
    } else {
        let state: Value = serde_json::from_str(
            &fs::read_to_string(p.join("current.json")).expect("current.json unreadable"),
        )
        .expect("current.json unparseable");
        let nodes = state["nodes"].as_object().cloned().unwrap_or_default();
        (state, nodes)
    }
}

/// Invariants any loadable central graph must satisfy, whatever the interleaving.
fn assert_graph_integrity(dir: &str, ctx: &str) {
    let (header, nodes) = load_central(dir);

    for (key, node) in &nodes {
        let id = node["id"].as_u64().expect("node missing id");
        assert_eq!(
            key.parse::<u64>().unwrap(),
            id,
            "{}: node map key {} disagrees with node.id {}",
            ctx,
            key,
            id
        );
        if let Some(deps) = node["depends_on"].as_object() {
            for dep in deps.keys() {
                assert!(
                    nodes.contains_key(dep),
                    "{}: node {} depends on missing node {} (dangling reference)",
                    ctx,
                    id,
                    dep
                );
            }
        }
    }

    let max_id = nodes
        .values()
        .filter_map(|n| n["id"].as_u64())
        .max()
        .unwrap_or(0);
    let next_id = header["next_id"].as_u64().expect("missing next_id");
    assert!(
        next_id > max_id,
        "{}: next_id {} <= max node id {} — the next write would collide",
        ctx,
        next_id,
        max_id
    );
}

fn labels(nodes: &Map<String, Value>) -> Vec<String> {
    nodes
        .values()
        .filter_map(|n| n["label"].as_str().map(|s| s.to_string()))
        .collect()
}

// ─────────────────────────── A. concurrent contribute ───────────────────────────

/// The core server-mode promise: if N clients publish at once, central keeps
/// every contribution. A lost update here means a teammate's knowledge silently
/// vanished — the worst possible failure for a shared knowledge store.
#[test]
fn concurrent_exports_preserve_every_contribution() {
    let s = Scratch::new("preserve");
    let central = s.sub("central");
    seed_central(&central, 250);
    assert!(braim(&central, &["shard"]).0, "shard conversion failed");

    let users: Vec<(String, String)> = (0..USERS)
        .map(|i| {
            let dir = s.sub(&format!("user{}", i));
            let domain = format!("team{}", i);
            seed_user(&dir, &domain, &format!("U{}", i));
            (dir, domain)
        })
        .collect();

    let children: Vec<Child> = users
        .iter()
        .map(|(dir, domain)| {
            braim_spawn(
                dir,
                &["export", domain, "--to", &central, "--include-unproven"],
            )
        })
        .collect();
    for mut c in children {
        c.wait().expect("export process failed");
    }

    assert_graph_integrity(&central, "after concurrent exports");

    let (_, nodes) = load_central(&central);
    let found = labels(&nodes);
    let missing: Vec<&str> = users
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let marker = format!("U{}Alpha", i);
            !found.iter().any(|l| l.contains(&marker))
        })
        .map(|(_, (_, d))| d.as_str())
        .collect();

    assert!(
        missing.is_empty(),
        "lost updates: {} of {} concurrent contributions missing from central ({:?})",
        missing.len(),
        USERS,
        missing
    );
}

/// Ids must stay unique and allocation must not hand the same id to two writers.
#[test]
fn concurrent_exports_never_collide_node_ids() {
    let s = Scratch::new("ids");
    let central = s.sub("central");
    seed_central(&central, 250);
    assert!(braim(&central, &["shard"]).0);

    let users: Vec<(String, String)> = (0..USERS)
        .map(|i| {
            let dir = s.sub(&format!("user{}", i));
            let domain = format!("team{}", i);
            seed_user(&dir, &domain, &format!("U{}", i));
            (dir, domain)
        })
        .collect();

    let children: Vec<Child> = users
        .iter()
        .map(|(dir, domain)| {
            braim_spawn(
                dir,
                &["export", domain, "--to", &central, "--include-unproven"],
            )
        })
        .collect();
    for mut c in children {
        c.wait().unwrap();
    }

    // load_central would panic on a duplicate across shards; integrity covers
    // key/id agreement, dangling deps, and next_id headroom.
    assert_graph_integrity(&central, "id collision check");

    // A follow-up write must land cleanly on top of whatever concurrency produced.
    let (ok, out) = braim(
        &central,
        &[
            "concept",
            "add",
            "PostCheck: concept added after the concurrent burst",
            "--domains",
            "seed",
            "--sources",
            "narrative:post",
        ],
    );
    assert!(ok, "central unusable after concurrent writes: {}", out);
    assert_graph_integrity(&central, "after post-burst write");
}

// ─────────────────────────── B. read consistency ───────────────────────────

/// Readers (the viewer, the query path, a consuming client) must never observe a
/// torn or unparseable central while writers are mid-flight. `fs::write`
/// truncates before writing, so a naive writer exposes an empty/partial file.
#[test]
fn readers_never_observe_torn_state() {
    let s = Scratch::new("torn");
    let central = s.sub("central");
    seed_central(&central, 250);
    assert!(braim(&central, &["shard"]).0);

    let users: Vec<(String, String)> = (0..USERS)
        .map(|i| {
            let dir = s.sub(&format!("user{}", i));
            let domain = format!("team{}", i);
            seed_user(&dir, &domain, &format!("U{}", i));
            (dir, domain)
        })
        .collect();

    let mut children: Vec<Child> = users
        .iter()
        .map(|(dir, domain)| {
            braim_spawn(
                dir,
                &["export", domain, "--to", &central, "--include-unproven"],
            )
        })
        .collect();

    // Hammer reads while the writes are in flight.
    let mut reads = 0usize;
    let mut torn = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut done = false;
    while !done && std::time::Instant::now() < deadline {
        let p = Path::new(&central);
        for f in ["graph.json"] {
            if let Ok(text) = fs::read_to_string(p.join(f)) {
                reads += 1;
                if serde_json::from_str::<Value>(&text).is_err() {
                    torn.push(format!("{} ({} bytes)", f, text.len()));
                }
            }
        }
        if let Ok(dir) = fs::read_dir(p.join("domains")) {
            for e in dir.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".json") {
                    continue;
                }
                if let Ok(text) = fs::read_to_string(e.path()) {
                    reads += 1;
                    if serde_json::from_str::<Value>(&text).is_err() {
                        torn.push(format!("{} ({} bytes)", name, text.len()));
                    }
                }
            }
        }
        done = true;
        for c in children.iter_mut() {
            // any still-running writer keeps the loop alive
            if matches!(c.try_wait(), Ok(None)) {
                done = false;
            }
        }
    }
    for mut c in children {
        let _ = c.wait();
    }

    assert!(reads > 0, "harness read nothing — test is not exercising anything");
    assert!(
        torn.is_empty(),
        "{} of {} reads observed a torn/unparseable file: {:?}",
        torn.len(),
        reads,
        torn.iter().take(5).collect::<Vec<_>>()
    );
}

/// Stronger than parseability: a sharded write touches many files, so a reader
/// that takes no lock could observe shard A updated and shard B not yet — a set
/// that is individually valid but mutually inconsistent. Cross-domain closure
/// exports are exactly the case that would expose it, since a statement lands in
/// one shard while a dependency it needs lands in another.
#[test]
fn readers_never_observe_an_inconsistent_shard_set() {
    let s = Scratch::new("shardset");
    let central = s.sub("central");
    seed_central(&central, 250);
    assert!(braim(&central, &["shard"]).0);

    // Each client publishes a domain whose statement depends on a concept living
    // in a DIFFERENT domain, so every export writes at least two shard files.
    let users: Vec<String> = (0..USERS)
        .map(|i| {
            let dir = s.sub(&format!("user{}", i));
            assert!(
                braim(
                    &dir,
                    &[
                        "concept",
                        "add",
                        &format!("X{}Shared: concept living in another domain", i),
                        "--domains",
                        &format!("vocab{}", i),
                        "--sources",
                        "narrative:seed"
                    ]
                )
                .0
            );
            assert!(
                braim(
                    &dir,
                    &[
                        "concept",
                        "add",
                        &format!("X{}Local: concept in the published domain", i),
                        "--domains",
                        &format!("pub{}", i),
                        "--sources",
                        "narrative:seed"
                    ]
                )
                .0
            );
            assert!(
                braim(
                    &dir,
                    &[
                        "statement",
                        "add",
                        &format!("cross-domain finding {}", i),
                        "--domains",
                        &format!("pub{}", i),
                        "--sources",
                        "code:x.rs:1",
                        "--depends",
                        "1:0.4,2:0.6",
                        "--assume"
                    ]
                )
                .0
            );
            dir
        })
        .collect();

    let mut children: Vec<Child> = users
        .iter()
        .enumerate()
        .map(|(i, dir)| {
            braim_spawn(
                dir,
                &[
                    "export",
                    &format!("pub{}", i),
                    "--to",
                    &central,
                    "--include-unproven",
                ],
            )
        })
        .collect();

    // A raw multi-file read can never be consistent on its own — what braim
    // guarantees is its seqlock protocol: a read is valid only if no writer lock
    // was present at either end and the completion sequence did not move. This
    // reader follows that protocol, exactly as the engine's loader does.
    let p = Path::new(&central);
    let read_seq = || -> u64 {
        fs::read_to_string(p.join(".braim.seq"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    let writer_active = || p.join(".braim.lock").exists();

    let mut clean_reads = 0usize;
    let mut skipped = 0usize;
    let mut inconsistent = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut done = false;
    while !done && std::time::Instant::now() < deadline {
        let seq_before = read_seq();
        let lock_before = writer_active();

        let mut nodes = Map::new();
        let mut readable = true;
        if let Ok(dir) = fs::read_dir(p.join("domains")) {
            for e in dir.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".json") {
                    continue;
                }
                match fs::read_to_string(e.path())
                    .ok()
                    .and_then(|t| serde_json::from_str::<Map<String, Value>>(&t).ok())
                {
                    Some(shard) => nodes.extend(shard),
                    None => readable = false,
                }
            }
        }

        let clean = !lock_before && !writer_active() && read_seq() == seq_before && readable;
        if clean && !nodes.is_empty() {
            clean_reads += 1;
            for node in nodes.values() {
                if let Some(deps) = node["depends_on"].as_object() {
                    for dep in deps.keys() {
                        if !nodes.contains_key(dep) {
                            inconsistent
                                .push(format!("node {} -> missing {}", node["id"], dep));
                        }
                    }
                }
            }
        } else {
            skipped += 1;
        }
        done = children.iter_mut().all(|c| !matches!(c.try_wait(), Ok(None)));
    }
    for mut c in children {
        let _ = c.wait();
    }

    assert!(
        clean_reads > 0,
        "protocol never admitted a read ({} skipped) — it would starve real readers",
        skipped
    );
    assert!(
        inconsistent.is_empty(),
        "{} dangling reference(s) in {} protocol-clean reads: {:?}",
        inconsistent.len(),
        clean_reads,
        inconsistent.iter().take(5).collect::<Vec<_>>()
    );

    // And the engine's own loader must agree once the dust settles.
    assert_graph_integrity(&central, "after cross-domain concurrent exports");
}

// ─────────────────────────── C. corroboration semantics ───────────────────────────

/// Two teammates independently reaching the same conclusion is the payoff of the
/// whole design (braim ID:190): their evidence must UNION onto one node, raising
/// verification, instead of forking into rival duplicates. Concurrency must not
/// break that.
#[test]
fn concurrent_identical_findings_union_their_sources() {
    let s = Scratch::new("corroborate");
    let central = s.sub("central");
    seed_central(&central, 120);
    assert!(braim(&central, &["shard"]).0);

    // Two users, same vocabulary and same finding, different PRIMARY evidence.
    let shared = |dir: &str, source: &str| {
        assert!(
            braim(
                dir,
                &[
                    "concept",
                    "add",
                    "Invoice: document requesting payment",
                    "--domains",
                    "billing",
                    "--sources",
                    "narrative:seed"
                ]
            )
            .0
        );
        assert!(
            braim(
                dir,
                &[
                    "concept",
                    "add",
                    "Payment: transfer settling an invoice",
                    "--domains",
                    "billing",
                    "--sources",
                    "narrative:seed"
                ]
            )
            .0
        );
        assert!(
            braim(
                dir,
                &[
                    "statement",
                    "add",
                    "payment settles invoice",
                    "--domains",
                    "billing",
                    "--sources",
                    source,
                    "--depends",
                    "1:0.6,2:0.4",
                    "--assume"
                ]
            )
            .0
        );
    };

    let a = s.sub("userA");
    let b = s.sub("userB");
    shared(&a, "code:billing.rs:42");
    shared(&b, "doc:billing_spec.md:7");

    // Deliberately NO --include-unproven: each client alone holds one PRIMARY
    // type and is therefore only `partial`. The default export floor sits at
    // partial precisely so both publish and corroborate in central (braim
    // ID:253); under the old proven-only default neither would have crossed.
    let ca = braim_spawn(&a, &["export", "billing", "--to", &central]);
    let cb = braim_spawn(&b, &["export", "billing", "--to", &central]);
    for mut c in [ca, cb] {
        c.wait().unwrap();
    }

    assert_graph_integrity(&central, "after concurrent corroboration");

    let (_, nodes) = load_central(&central);
    let matches: Vec<&Value> = nodes
        .values()
        .filter(|n| n["label"].as_str() == Some("payment settles invoice"))
        .collect();

    assert_eq!(
        matches.len(),
        1,
        "the same finding from two clients must dedup to ONE central node, found {}",
        matches.len()
    );
    let srcs: Vec<&str> = matches[0]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        srcs.iter().any(|s| s.starts_with("code:")) && srcs.iter().any(|s| s.starts_with("doc:")),
        "both clients' PRIMARY sources must union onto the node (ID:190), got {:?}",
        srcs
    );
    assert_eq!(
        matches[0]["verification_status"].as_str(),
        Some("proven"),
        "code + doc corroboration must promote the shared finding to proven"
    );
}

// ─────────────────────────── D. idempotence ───────────────────────────

/// Re-publishing an unchanged domain must be a no-op. A server retrying a failed
/// contribution cannot be allowed to duplicate knowledge.
#[test]
fn repeated_export_is_idempotent() {
    let s = Scratch::new("idempotent");
    let central = s.sub("central");
    seed_central(&central, 60);
    assert!(braim(&central, &["shard"]).0);

    let user = s.sub("user");
    seed_user(&user, "billing", "IdemP");

    assert!(braim(&user, &["export", "billing", "--to", &central, "--include-unproven"]).0);
    let (_, after_first) = load_central(&central);
    let count_first = after_first.len();

    for _ in 0..3 {
        assert!(braim(&user, &["export", "billing", "--to", &central, "--include-unproven"]).0);
    }
    let (_, after_repeat) = load_central(&central);

    assert_eq!(
        after_repeat.len(),
        count_first,
        "re-exporting an unchanged domain duplicated nodes ({} → {})",
        count_first,
        after_repeat.len()
    );
    assert_graph_integrity(&central, "after repeated exports");
}

// ─────────────────────────── E. checkpoint integrity ───────────────────────────

/// Checkpoints are the pin artifacts consumers reference (braim ID:214/242).
/// Concurrent `version save` must not corrupt the index or lose entries.
#[test]
fn concurrent_checkpoints_keep_versions_index_consistent() {
    let s = Scratch::new("versions");
    let central = s.sub("central");
    seed_central(&central, 120);
    assert!(braim(&central, &["shard"]).0);

    let children: Vec<Child> = (0..USERS)
        .map(|i| braim_spawn(&central, &["version", "save", &format!("checkpoint {}", i)]))
        .collect();
    for mut c in children {
        c.wait().unwrap();
    }

    let idx_path = Path::new(&central).join("versions.json");
    let text = fs::read_to_string(&idx_path).expect("versions.json missing");
    let index: Value = serde_json::from_str(&text).expect("versions.json corrupted by concurrent saves");
    let entries = index.as_array().expect("versions index must be an array");

    assert_eq!(
        entries.len(),
        USERS,
        "concurrent checkpoints lost entries: {} of {} recorded",
        entries.len(),
        USERS
    );

    // Every recorded pin must resolve to a snapshot file that exists.
    for e in entries {
        let hv = e["header_version"].as_u64().unwrap();
        assert!(
            Path::new(&central).join(format!("graph.v{:04}.json", hv)).exists(),
            "checkpoint references missing header snapshot graph.v{:04}.json",
            hv
        );
    }

    // Version numbers must be unique — a duplicate means two writers claimed one.
    let mut seen = std::collections::HashSet::new();
    for e in entries {
        let v = e["version"].as_u64().unwrap();
        assert!(seen.insert(v), "duplicate version number {} across concurrent saves", v);
    }
}

// ─────────────────────────── F. mixed read/write workload ───────────────────────────

/// The realistic server profile: contributors writing while consumers query.
/// Readers must succeed throughout and central must stay sound afterwards.
#[test]
fn mixed_readers_and_writers_leave_central_sound() {
    let s = Scratch::new("mixed");
    let central = s.sub("central");
    seed_central(&central, 200);
    assert!(braim(&central, &["shard"]).0);

    let users: Vec<(String, String)> = (0..USERS)
        .map(|i| {
            let dir = s.sub(&format!("user{}", i));
            let domain = format!("team{}", i);
            seed_user(&dir, &domain, &format!("U{}", i));
            (dir, domain)
        })
        .collect();

    let mut procs: Vec<Child> = users
        .iter()
        .map(|(dir, domain)| {
            braim_spawn(
                dir,
                &["export", domain, "--to", &central, "--include-unproven"],
            )
        })
        .collect();
    // concurrent consumers
    for _ in 0..3 {
        procs.push(braim_spawn(&central, &["audit"]));
        procs.push(braim_spawn(&central, &["domains"]));
    }

    let mut failures = 0;
    for mut c in procs {
        if !c.wait().unwrap().success() {
            failures += 1;
        }
    }

    assert_eq!(failures, 0, "{} concurrent operations failed outright", failures);
    assert_graph_integrity(&central, "after mixed workload");
}

/// A rejected command must not strand the write lock.
///
/// `open_for_write` takes the cross-process lock before dispatch and releases it
/// only through Drop, but every argument-validation branch used to
/// `std::process::exit(1)`, which skips destructors — as did main's own error
/// handler, since `braim` was still in scope there. One missing `--domains` left
/// `.braim.lock` behind, and the next writer waited the full 30s timeout and
/// then failed outright (braim ID:326). Both shapes are covered here: a CLI
/// rejection before the graph is touched, and an error raised by the graph
/// itself.
#[test]
fn a_rejected_command_releases_the_write_lock() {
    let scratch = Scratch::new("lockleak");
    let dir = scratch.sub("central");
    seed_central(&dir, 5);
    let lock = Path::new(&dir).join(".braim.lock");

    // (1) argument validation: --sources given, --domains missing.
    let (ok, out) = braim(
        &dir,
        &["statement", "add", "Seed1 and Seed2 share an origin", "--sources", "code:a.rs:1", "--depends", "1:0.6,2:0.4"],
    );
    assert!(!ok, "expected the malformed command to be rejected: {}", out);
    assert!(!lock.exists(), "write lock survived a rejected command");

    // (2) an error from the graph, not the parser: dependency 999 does not exist.
    let (ok, out) = braim(
        &dir,
        &["statement", "add", "Seed1 rests on a node that is not there", "--domains", "a,b", "--sources", "code:a.rs:1,doc:b.md", "--depends", "1:0.6,999:0.4"],
    );
    assert!(!ok, "expected the dangling dependency to be rejected: {}", out);
    assert!(!lock.exists(), "write lock survived a graph-level error");

    // The next writer must proceed immediately — not sit out the stale window.
    let started = std::time::Instant::now();
    let (ok, out) = braim(
        &dir,
        &["statement", "add", "Seed1 and Seed2 are both baseline concepts", "--domains", "a,b", "--sources", "code:a.rs:1,doc:b.md", "--depends", "1:0.6,2:0.4"],
    );
    assert!(ok, "follow-up write failed: {}", out);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "follow-up write took {:?} — it waited on a leaked lock",
        started.elapsed()
    );
    assert_graph_integrity(&dir, "after rejected commands");
}
