//! Integration coverage for the rctags route enforcement: the PreToolUse
//! route gate (`braim route-gate`), its bypass and fail-open ledgers, the
//! Stop gate's tool-bypass filing, `braim init`'s conditional install, the
//! rctags pointer fallback, and `braim audit`'s symbol-anchor check.
//!
//! No `[lib]` target exists for this crate (see Cargo.toml), so every
//! assertion drives the real compiled binary via subprocess, the convention
//! tests/audit_rot_loop.rs already uses. `rctags` is a stub shell script
//! placed first on PATH: the live index is mutable and is never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_braim");

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("braim_re_{}_{}", test, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn sub(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A stand-in `rctags`, returned as the directory to prefix PATH with. `health` records its cwd when RCTAGS_STUB_CWD_FILE is
/// set and answers per RCTAGS_STUB_MODE (healthy | empty | broken), and —
/// only in the healthy case — adds a `read_not_enriched` line plus two
/// `enrich:` suggestion lines when RCTAGS_STUB_READ_NOT_ENRICHED is set;
/// `query <sym> --exact` answers from `./.rctags/index.json`, one entry per
/// line.
fn install_stub(bin_dir: &Path) -> PathBuf {
    fs::create_dir_all(bin_dir).unwrap();
    let stub = bin_dir.join("rctags");
    fs::write(
        &stub,
        r#"#!/bin/sh
case "$1" in
  health)
    [ -n "$RCTAGS_STUB_CWD_FILE" ] && pwd > "$RCTAGS_STUB_CWD_FILE"
    case "${RCTAGS_STUB_MODE:-healthy}" in
      broken) echo "stub: broken" >&2; exit 1;;
      empty) printf 'indexed_files: 0\nmissing_files: 0 (0.0%%)\nunparsed_files: 0\nverdict: OK\n'; exit 0;;
      *) printf 'indexed_files: 3\nmissing_files: 0 (0.0%%)\nunparsed_files: 0\nverdict: OK\n';
         if [ -n "$RCTAGS_STUB_READ_NOT_ENRICHED" ]; then
           printf 'read_not_enriched: %s\n' "$RCTAGS_STUB_READ_NOT_ENRICHED"
           printf '  enrich: rctags enrich fooSymbol --file src/foo.rs --summary "..."\n'
           printf '  enrich: rctags enrich barSymbol --file src/bar.rs --summary "..."\n'
         fi
         exit 0;;
    esac;;
  query)
    printf '['
    grep -F "\"name\":\"$2\"," .rctags/index.json | sed 's/,$//' | paste -sd, -
    printf ']\n'
    exit 0;;
  *) exit 1;;
esac
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin_dir.to_path_buf()
}

fn write_index(root: &Path, entries: &[&str]) {
    fs::create_dir_all(root.join(".rctags")).unwrap();
    let body = format!("[\n{}\n]\n", entries.join(",\n"));
    fs::write(root.join(".rctags").join("index.json"), body).unwrap();
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run `braim --data-dir <dir> --quiet <args>` with `stdin` fed, `cwd` set,
/// PATH prefixed by `path_prefix` (None = a PATH with no rctags at all), and
/// the given extra env.
fn braim(
    data_dir: &Path,
    args: &[&str],
    stdin: Option<&str>,
    cwd: &Path,
    path_prefix: Option<&Path>,
    env: &[(&str, &str)],
) -> Run {
    let mut cmd = Command::new(BIN);
    cmd.arg("--data-dir").arg(data_dir).arg("--quiet").args(args).current_dir(cwd);
    let base = std::env::var("PATH").unwrap_or_default();
    match path_prefix {
        Some(p) => cmd.env("PATH", format!("{}:{}", p.display(), base)),
        None => cmd.env("PATH", "/nonexistent-dir-for-this-test"),
    };
    cmd.env_remove("BRAIM_AUDIT_THRESHOLD").env_remove("BRAIM_DATA_DIR");
    cmd.env("HOME", cwd.join("home_for_test"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to run braim");
    {
        use std::io::Write;
        let mut si = child.stdin.take().unwrap();
        if let Some(s) = stdin {
            si.write_all(s.as_bytes()).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

fn pretool(cmd: &str, cwd: &Path) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": cmd},
        "cwd": cwd.to_string_lossy()
    })
    .to_string()
}

fn deny_reason(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let h = v.get("hookSpecificOutput")?;
    if h.get("permissionDecision")?.as_str()? != "deny" {
        return None;
    }
    Some(h.get("permissionDecisionReason")?.as_str()?.to_string())
}

fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

fn seed(data_dir: &Path, cwd: &Path, bin: &Path) {
    let r = braim(data_dir, &["concept", "add", "Alpha: a seeded concept", "--domains", "t", "--sources", "code:a.rs:1"], None, cwd, Some(bin), &[]);
    assert_eq!(r.code, 0, "seed 1: {}{}", r.stdout, r.stderr);
    let r = braim(data_dir, &["concept", "add", "Beta: another seeded concept", "--domains", "t", "--sources", "code:b.rs:1"], None, cwd, Some(bin), &[]);
    assert_eq!(r.code, 0, "seed 2: {}{}", r.stdout, r.stderr);
}

// ───────────────────────── D1: dispatch ─────────────────────────

#[test]
fn route_gate_always_exits_zero_whatever_stdin_holds() {
    let s = Scratch::new("exit0");
    let data = s.sub("store");
    let cwd = s.sub("cwd");
    let bin = install_stub(&s.path().join("bin"));

    let r = braim(&data, &["route-gate"], Some(&pretool("ls", &cwd)), &cwd, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let r = braim(&data, &["route-gate"], Some(""), &cwd, Some(&bin), &[]);
    assert_eq!(r.code, 0, "empty stdin: {}{}", r.stdout, r.stderr);
    let r = braim(&data, &["route-gate"], Some("{not json"), &cwd, Some(&bin), &[]);
    assert_eq!(r.code, 0, "malformed stdin: {}{}", r.stdout, r.stderr);
    assert!(r.code != 2);
}

#[test]
fn route_gate_is_dispatched_read_only_and_never_touches_the_write_lock() {
    let s = Scratch::new("lock");
    let data = s.sub("store");
    let cwd = s.sub("cwd");
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &cwd, &bin);
    let lock = data.join(".braim.lock");

    // No lock before, no lock after.
    assert!(!lock.exists());
    let r = braim(&data, &["route-gate"], Some(&pretool("ls", &cwd)), &cwd, Some(&bin), &[]);
    assert_eq!(r.code, 0);
    assert!(!lock.exists(), "route-gate must not create .braim.lock");

    // A fresh foreign lock: a writer would wait the full 30s WAIT_TIMEOUT.
    fs::write(&lock, "999999\n").unwrap();
    let started = Instant::now();
    let r = braim(&data, &["route-gate"], Some(&pretool("ls", &cwd)), &cwd, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(started.elapsed() < Duration::from_secs(5), "route-gate waited on the write lock: {:?}", started.elapsed());
    assert!(lock.exists(), "route-gate must not remove a lock it never took");

    // Negative control proving the sentinel is real: a write command against
    // the same pre-locked store does not return within the same bound.
    let mut child = Command::new(BIN)
        .arg("--data-dir").arg(&data).arg("--quiet")
        .args(["statement", "add", "Alpha relates to Beta", "--domains", "t", "--sources", "code:a.rs:1,doc:b.md:1", "--depends", "1:0.6,2:0.4"])
        .current_dir(&cwd)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    let started = Instant::now();
    let mut finished = None;
    while started.elapsed() < Duration::from_secs(5) {
        if let Some(st) = child.try_wait().unwrap() {
            finished = Some(st);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    match finished {
        None => {
            let _ = child.kill();
            let _ = child.wait();
        }
        Some(st) => {
            let out = child.wait_with_output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(!st.success() && err.contains("write lock"), "writer returned early without the lock error: {}", err);
        }
    }
    let _ = fs::remove_file(&lock);
}

#[test]
fn route_gate_is_listed_in_is_read_only() {
    let src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    let start = src.find("fn is_read_only(").expect("is_read_only exists");
    let body = &src[start..];
    let end = body.find("\n}\n").expect("is_read_only closes");
    let block = &body[..end];
    assert!(block.contains("| Commands::StopGate"), "control: StopGate is in the matches! block");
    assert!(block.contains("| Commands::RouteGate"), "RouteGate must be dispatched read-only");
}

// ───────────────────────── D3 / D5: bypass and deny ─────────────────────────

#[test]
fn a_bare_grep_under_an_indexed_root_is_denied_with_exit_zero_and_the_redirect() {
    let s = Scratch::new("deny");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let sub = s.sub("repo/src/deep");
    let bin = install_stub(&s.path().join("bin"));

    let r = braim(&data, &["route-gate"], Some(&pretool("grep -rn foo src/", &sub)), &sub, Some(&bin), &[]);
    assert_eq!(r.code, 0, "deny is never a nonzero exit: {}", r.stderr);
    let reason = deny_reason(&r.stdout).expect(&format!("expected deny JSON on stdout, got: {}", r.stdout));
    assert!(reason.contains("rctags query"), "{}", reason);
    assert!(reason.contains("rctags load"), "{}", reason);
    assert!(reason.contains("RCTAGS_BYPASS=1"), "{}", reason);
    assert!(reason.contains("the bypass is recorded"), "{}", reason);
    assert!(reason.contains(&root.to_string_lossy().to_string()), "names the indexed root: {}", reason);

    // An allowed command prints nothing.
    let r = braim(&data, &["route-gate"], Some(&pretool("cat foo.txt", &sub)), &sub, Some(&bin), &[]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.trim().is_empty(), "allow prints nothing: {}", r.stdout);

    // The module never uses exit 2 for anything.
    let src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/route_gate.rs")).unwrap();
    assert!(!src.contains("exit(2)"), "route_gate.rs must never exit 2");
}

#[test]
fn a_declared_bypass_is_allowed_and_logged_by_the_gate_at_the_root() {
    let s = Scratch::new("bypass");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let sub = s.sub("repo/src");
    let bin = install_stub(&s.path().join("bin"));
    let log = root.join(".rctags").join("bypass.log");
    let permit = root.join(".rctags").join("bypass_permit");
    let arm = || {
        let r = braim(&data, &["route-gate"], Some(&pretool("rctags query foo", &sub)), &sub, Some(&bin), &[]);
        assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
        assert!(permit.is_file(), "a real rctags call must arm the permit");
    };

    // Unearned: no prior rctags call this test, so the first bypass is denied.
    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=1 grep foo", &sub)), &sub, Some(&bin), &[]);
    assert_eq!(r.code, 0);
    assert!(deny_reason(&r.stdout).is_some(), "unearned bypass must be denied: {}", r.stdout);
    assert!(!log.exists(), "a denied bypass writes no ledger line");

    // Earned: arm, then the bypass is allowed and logged.
    arm();
    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=1 grep foo", &sub)), &sub, Some(&bin), &[]);
    assert_eq!(r.code, 0);
    assert!(deny_reason(&r.stdout).is_none(), "earned bypass must be allowed: {}", r.stdout);
    assert!(!permit.exists(), "the permit is consumed");
    let l = lines(&log);
    assert_eq!(l.len(), 1, "exactly one line at the ROOT, not the cwd: {:?}", l);
    assert!(!sub.join(".rctags").exists(), "nothing is written under the cwd");
    let fields: Vec<&str> = l[0].split('\t').collect();
    assert_eq!(fields.len(), 3, "ts\\tcwd\\tcommand: {:?}", fields);
    assert!(fields[0].ends_with('Z') && fields[0].contains('T'), "ISO-8601 timestamp: {}", fields[0]);
    assert_eq!(fields[1], sub.to_string_lossy());
    assert_eq!(fields[2], "RCTAGS_BYPASS=1 grep foo");

    // Spent: an immediate second bypass with no new rctags call is denied.
    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=1 grep bar", &sub)), &sub, Some(&bin), &[]);
    assert!(deny_reason(&r.stdout).is_some(), "a second bypass without a new rctags call must be denied");
    assert_eq!(lines(&log).len(), 1, "the denied second attempt writes no ledger line");

    // Mid-pipeline and env-prefixed forms also qualify, once re-armed.
    arm();
    let r = braim(&data, &["route-gate"], Some(&pretool("cat f | RCTAGS_BYPASS=1 grep x", &sub)), &sub, Some(&bin), &[]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none());
    assert_eq!(lines(&log).len(), 2);

    // Negative controls (each needs its own arm, since the prior bypass spent it).
    arm();
    let r = braim(&data, &["route-gate"], Some(&pretool("grep foo", &sub)), &sub, Some(&bin), &[]);
    assert!(deny_reason(&r.stdout).is_some(), "without the token the same command is denied");
    assert_eq!(lines(&log).len(), 2, "a denial writes no bypass line");
    assert!(permit.is_file(), "a denial for lacking the token at all does not consume the permit");

    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=10 grep foo", &sub)), &sub, Some(&bin), &[]);
    assert!(deny_reason(&r.stdout).is_some(), "RCTAGS_BYPASS=10 is not a bypass");
    assert_eq!(lines(&log).len(), 2);
    assert!(permit.is_file(), "RCTAGS_BYPASS=10 is not a bypass attempt, so it does not consume the permit either");

    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=1 cat foo", &sub)), &sub, Some(&bin), &[]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none());
    assert_eq!(lines(&log).len(), 2, "a bypass token on a non-grep command is not a defection");
    assert!(permit.is_file(), "a non-grep command never consumes the permit, earned or not");
}

// ───────────────────────── D4: health probe and fail-open ─────────────────────────

#[test]
fn the_gate_probes_rctags_from_the_indexed_root_and_fails_open_when_unhealthy() {
    let s = Scratch::new("health");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let sub = s.sub("repo/tests");
    let bin = install_stub(&s.path().join("bin"));
    let cwd_file = s.path().join("stub_cwd");
    let health_log = root.join(".rctags").join("route_gate_health.log");
    let bypass_log = root.join(".rctags").join("bypass.log");
    let payload = pretool("grep foo", &sub);
    let cwd_env = cwd_file.to_string_lossy().to_string();

    // (a) healthy: denied, and the probe ran from the ROOT, not the subdir.
    let r = braim(&data, &["route-gate"], Some(&payload), &sub, Some(&bin), &[("RCTAGS_STUB_CWD_FILE", &cwd_env)]);
    assert!(deny_reason(&r.stdout).is_some(), "healthy stub → denied: {}", r.stdout);
    let probed = PathBuf::from(fs::read_to_string(&cwd_file).unwrap().trim());
    assert_eq!(probed.canonicalize().unwrap(), root.canonicalize().unwrap());
    assert!(!health_log.exists(), "a healthy probe writes no health-log line");

    // (b) zero-entry OK is vacuous: allowed, logged.
    let r = braim(&data, &["route-gate"], Some(&payload), &sub, Some(&bin), &[("RCTAGS_STUB_MODE", "empty")]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none(), "{}", r.stdout);
    let l = lines(&health_log);
    assert_eq!(l.len(), 1, "{:?}", l);
    assert!(l[0].contains("indexed-files-zero"), "{}", l[0]);

    // (c) exit 1: allowed, logged.
    let r = braim(&data, &["route-gate"], Some(&payload), &sub, Some(&bin), &[("RCTAGS_STUB_MODE", "broken")]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none());
    let l = lines(&health_log);
    assert_eq!(l.len(), 2);
    assert!(l[1].contains("rctags-exit-1"), "{}", l[1]);

    // (c) no rctags anywhere and no pointer: allowed, logged.
    let r = braim(&data, &["route-gate"], Some(&payload), &sub, None, &[]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none());
    let l = lines(&health_log);
    assert_eq!(l.len(), 3);
    let fields: Vec<&str> = l[2].split('\t').collect();
    assert_eq!(fields.len(), 4, "ts\\tcwd\\treason\\tcommand: {:?}", fields);
    assert_eq!(fields[2], "rctags-not-found");
    assert_eq!(fields[3], "grep foo");

    // (d) log separation.
    assert!(!bypass_log.exists(), "fail-open never writes to bypass.log");
}

// ───────────────────────── D9: the Stop gate reads the ledger ─────────────────────────

fn stop_payload(cwd: &Path) -> String {
    serde_json::json!({
        "hook_event_name": "Stop",
        "last_assistant_message": "done [braim: none | audit: clean]",
        "cwd": cwd.to_string_lossy()
    })
    .to_string()
}

fn state_json(home: &Path) -> serde_json::Value {
    let dir = home.join(".claude").join("braim_stop_state");
    let entry = fs::read_dir(&dir).expect("state dir exists").next().expect("one state file").unwrap();
    serde_json::from_str(&fs::read_to_string(entry.path()).unwrap()).unwrap()
}

fn review_items(data: &Path) -> Vec<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(data.join("reviews.json")).unwrap_or_else(|_| "[]".into())).unwrap()
}

fn append_bypass(root: &Path, n: usize, from: usize) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(root.join(".rctags").join("bypass.log")).unwrap();
    for i in from..from + n {
        writeln!(f, "2026-10-08T12:00:{:02}Z\t{}\tRCTAGS_BYPASS=1 grep needle{} src/", i, root.display(), i).unwrap();
    }
}

#[test]
fn the_stop_gate_files_one_tool_bypass_item_per_threshold_crossing() {
    let s = Scratch::new("stop");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);
    let home = root.join("home_for_test");
    let gate = |msg_cwd: &Path| braim(&data, &["stop-gate"], Some(&stop_payload(msg_cwd)), &root, Some(&bin), &[]);

    // (1) three lines: below threshold, nothing filed, watermark untouched.
    append_bypass(&root, 3, 1);
    let r = gate(&root);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(review_items(&data).is_empty());
    assert_eq!(state_json(&home)["last_seen_bypass"], 0);

    // (2) five lines: exactly one item listing all five commands.
    append_bypass(&root, 2, 4);
    let r = gate(&root);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let items = review_items(&data);
    assert_eq!(items.len(), 1, "{:?}", items);
    assert_eq!(items[0]["kind"], "tool-bypass");
    let note = items[0]["note"].as_str().unwrap();
    for i in 1..=5 {
        assert!(note.contains(&format!("grep needle{} src/", i)), "note lists command {}: {}", i, note);
    }
    let st = state_json(&home);
    assert_eq!(st["last_seen_bypass"], 5);
    assert_eq!(st["last_seen_pending"], 1, "baseline bumped by the gate-filed item");
    let r = braim(&data, &["dream", "review", "--count", "--json"], None, &root, Some(&bin), &[]);
    let v: serde_json::Value = serde_json::from_str(r.stdout.trim()).unwrap();
    assert_eq!(v["tool-bypass"], 1);

    // (3) the next 'audit: clean' is not blocked for the rise the gate caused.
    let r = gate(&root);
    assert_eq!(r.code, 0, "blocked for the gate's own filing: {}", r.stderr);
    assert_eq!(review_items(&data).len(), 1);

    // (4) a truncated log resets the watermark and files nothing.
    fs::write(root.join(".rctags").join("bypass.log"), "2026-10-08T13:00:00Z\t/x\tRCTAGS_BYPASS=1 grep z\n").unwrap();
    let r = gate(&root);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(state_json(&home)["last_seen_bypass"], 1);
    assert_eq!(review_items(&data).len(), 1);

    // (6) a cwd outside any indexed tree is a no-op.
    let before = state_json(&home);
    let outside = s.sub("elsewhere");
    let r = gate(&outside);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(state_json(&home)["last_seen_bypass"], before["last_seen_bypass"]);
    assert_eq!(review_items(&data).len(), 1);
}

#[test]
fn sub_threshold_bypasses_accumulate_across_turns() {
    let s = Scratch::new("accum");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);
    let home = root.join("home_for_test");

    append_bypass(&root, 4, 1);
    let r = braim(&data, &["stop-gate"], Some(&stop_payload(&root)), &root, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(review_items(&data).is_empty(), "4 < 5: nothing yet");
    assert_eq!(state_json(&home)["last_seen_bypass"], 0, "the watermark moves only on a filing");

    append_bypass(&root, 4, 5);
    let r = braim(&data, &["stop-gate"], Some(&stop_payload(&root)), &root, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let items = review_items(&data);
    assert_eq!(items.len(), 1);
    let note = items[0]["note"].as_str().unwrap();
    for i in 1..=8 {
        assert!(note.contains(&format!("grep needle{} src/", i)), "lists line {}: {}", i, note);
    }
    assert_eq!(state_json(&home)["last_seen_bypass"], 8);
}

// ─────────── D20: the Stop gate blocks a turn with unenriched reads ───────────

#[test]
fn the_stop_gate_blocks_on_unenriched_reads_and_names_the_enrich_commands() {
    let s = Scratch::new("enrich");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);

    // Healthy index, nothing unread-and-unenriched: the gate does not block.
    let r = braim(&data, &["stop-gate"], Some(&stop_payload(&root)), &root, Some(&bin), &[]);
    assert_eq!(r.code, 0, "clean health must not block: {}", r.stderr);

    // Two unenriched reads: blocked, exit 2, the count and both ready-to-run
    // enrich commands appear in the message (stderr, since block! prints there).
    let r = braim(
        &data,
        &["stop-gate"],
        Some(&stop_payload(&root)),
        &root,
        Some(&bin),
        &[("RCTAGS_STUB_READ_NOT_ENRICHED", "2")],
    );
    assert_eq!(r.code, 2, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stderr.contains("2 symbol(s)"), "{}", r.stderr);
    assert!(r.stderr.contains("read_not_enriched=2"), "{}", r.stderr);
    assert!(r.stderr.contains("rctags enrich fooSymbol --file src/foo.rs"), "{}", r.stderr);
    assert!(r.stderr.contains("rctags enrich barSymbol --file src/bar.rs"), "{}", r.stderr);

    // Fail-open: no rctags on PATH at all must not block, even though the
    // index exists — same posture as the route gate's own rctags-not-found.
    let r = braim(
        &data,
        &["stop-gate"],
        Some(&stop_payload(&root)),
        &root,
        None,
        &[("RCTAGS_STUB_READ_NOT_ENRICHED", "2")],
    );
    assert_eq!(r.code, 0, "no rctags on PATH must fail open: {}", r.stderr);

    // A cwd outside any indexed tree is also a no-op, matching the bypass sweep.
    let outside = s.sub("elsewhere");
    let r = braim(
        &data,
        &["stop-gate"],
        Some(&stop_payload(&outside)),
        &outside,
        Some(&bin),
        &[("RCTAGS_STUB_READ_NOT_ENRICHED", "2")],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
}

// ───────────────────────── D12 / D19: init-time probe and the pointer ─────────────────────────

fn settings_of(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn init_installs_the_route_gate_only_when_rctags_answers_healthy() {
    let s = Scratch::new("init");
    let bin = install_stub(&s.path().join("bin"));

    // Failing rctags: hint printed, four hooks installed, no PreToolUse, no pointer.
    let proj = s.sub("broken");
    let data = proj.join(".braim");
    let settings = proj.join(".claude").join("settings.json");
    let r = braim(&data, &["init", "--settings", settings.to_str().unwrap()], None, &proj, Some(&bin), &[("RCTAGS_STUB_MODE", "broken")]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("route gate was NOT installed"), "{}", r.stdout);
    let v = settings_of(&settings);
    assert!(v["hooks"].get("PreToolUse").is_none(), "{}", v);
    assert_eq!(v["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
    assert_eq!(v["hooks"]["PreCompact"].as_array().unwrap().len(), 1);
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
    assert!(!data.join("rctags_bin").exists(), "a failing probe writes no pointer");

    // Unindexed project: the reindex hint, no PreToolUse.
    let proj = s.sub("empty");
    let data = proj.join(".braim");
    let settings = proj.join(".claude").join("settings.json");
    let r = braim(&data, &["init", "--settings", settings.to_str().unwrap()], None, &proj, Some(&bin), &[("RCTAGS_STUB_MODE", "empty")]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("rctags reindex ."), "{}", r.stdout);
    assert!(settings_of(&settings)["hooks"].get("PreToolUse").is_none());

    // Healthy: the entry is added and the pointer written.
    let proj = s.sub("healthy");
    let data = proj.join(".braim");
    let settings = proj.join(".claude").join("settings.json");
    let r = braim(&data, &["init", "--settings", settings.to_str().unwrap()], None, &proj, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let v = settings_of(&settings);
    let pre = v["hooks"]["PreToolUse"].as_array().expect("PreToolUse installed");
    assert_eq!(pre.len(), 1);
    assert_eq!(pre[0]["matcher"], "Bash");
    let cmd = pre[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.starts_with("braim route-gate || echo '{\"hookSpecificOutput\""), "{}", cmd);
    assert!(cmd.contains("\"permissionDecision\":\"allow\""), "{}", cmd);
    let pointer = fs::read_to_string(data.join("rctags_bin")).expect("pointer written");
    assert_eq!(PathBuf::from(pointer.trim()), bin.join("rctags"));
}

#[test]
fn the_recorded_pointer_resolves_rctags_when_path_cannot() {
    let s = Scratch::new("pointer");
    let bin = install_stub(&s.path().join("bin"));
    let proj = s.sub("repo");
    write_index(&proj, &[]);
    let data = proj.join(".braim");
    let settings = proj.join(".claude").join("settings.json");
    let sub = s.sub("repo/src");

    let r = braim(&data, &["init", "--settings", settings.to_str().unwrap()], None, &proj, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let pointer = data.join("rctags_bin");
    assert_eq!(PathBuf::from(fs::read_to_string(&pointer).unwrap().trim()), bin.join("rctags"));

    // PATH without rctags: still denied, resolved through the pointer.
    let r = braim(&data, &["route-gate"], Some(&pretool("grep foo", &sub)), &sub, None, &[]);
    assert_eq!(r.code, 0);
    assert!(deny_reason(&r.stdout).is_some(), "pointer fallback must deny: {}{}", r.stdout, r.stderr);
    let health_log = proj.join(".rctags").join("route_gate_health.log");
    assert!(!health_log.exists());

    // Negative control: without the pointer the same call fails open.
    fs::remove_file(&pointer).unwrap();
    let r = braim(&data, &["route-gate"], Some(&pretool("grep foo", &sub)), &sub, None, &[]);
    assert_eq!(r.code, 0);
    assert!(deny_reason(&r.stdout).is_none(), "{}", r.stdout);
    let l = lines(&health_log);
    assert_eq!(l.len(), 1);
    assert!(l[0].contains("\trctags-not-found\t"), "{}", l[0]);
}

// ───────────────────────── D15: audit's symbol-anchor check ─────────────────────────

const FIXTURE_INDEX: [&str; 5] = [
    r#"{"name":"from_verification_status","file":"/home/magnimus/braim/src/graph.rs","line_start":30,"line_end":38,"type":"function","enriched":false,"language":"rust"}"#,
    r#"{"name":"from_verification_status","file":"src/graph.rs","line_start":141,"line_end":150,"type":"function","enriched":false,"language":"rust"}"#,
    r#"{"name":"a_deleted_symbol_name","file":"src/other.rs","line_start":1,"line_end":2,"type":"function","enriched":false,"language":"rust"}"#,
    r#"{"name":"run","file":"src/main.rs","line_start":1,"line_end":2,"type":"function","enriched":false,"language":"rust"}"#,
    r#"{"name":"run","file":"src/stop_gate.rs","line_start":1,"line_end":2,"type":"function","enriched":false,"language":"rust"}"#,
];

fn stale_ids(audit_stdout: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    let mut in_section = false;
    for line in audit_stdout.lines() {
        if line.contains("Stale symbol anchors") {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("──") {
            break;
        }
        if in_section {
            if let Some(rest) = line.trim().strip_prefix("✗ ID:") {
                let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                ids.push(n.parse().unwrap());
            }
        }
    }
    ids
}

#[test]
fn audit_lists_only_symbol_anchors_rctags_cannot_find_at_that_file() {
    let s = Scratch::new("audit");
    let bin = install_stub(&s.path().join("bin"));
    let proj = s.sub("repo");
    write_index(&proj, &FIXTURE_INDEX);
    let data = proj.join(".braim");
    seed(&data, &proj, &bin);

    let add = |label: &str, source: &str| -> u32 {
        let r = braim(&data, &["statement", "add", label, "--domains", "t", "--sources", &format!("{},doc:spec.md:1", source), "--depends", "1:0.6,2:0.4"], None, &proj, Some(&bin), &[]);
        assert_eq!(r.code, 0, "{}: {}{}", source, r.stdout, r.stderr);
        let idx = r.stdout.find("ID:").expect("prints the new id");
        r.stdout[idx + 3..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()
    };
    let present = add("Alpha and Beta share a present symbol", "code:src/graph.rs:from_verification_status");
    let deleted = add("Alpha and Beta share a deleted symbol", "code:src/graph.rs:a_deleted_symbol_name");
    let line = add("Alpha and Beta share a line anchor", "code:src/graph.rs:1300");
    let range = add("Alpha and Beta share a range anchor", "code:src/graph.rs:2952-2975");
    let abs = add("Alpha and Beta share an absolute anchor", "code:/home/magnimus/rctags/src/indexer.rs:reindex_diff_opts");
    let old = add("Alpha and Beta share an old-shape anchor", "code:rctags:src/graph.rs#from_verification_status");
    let file_only = add("Alpha and Beta share a file-only anchor", "code:src/dream.rs");

    let r = braim(&data, &["audit"], None, &proj, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("── Stale symbol anchors"), "{}", r.stdout);
    assert!(r.stdout.contains("rctags indexes functions only"), "{}", r.stdout);
    let ids = stale_ids(&r.stdout);
    assert_eq!(ids, vec![deleted], "only the deleted symbol is listed; present={} line={} range={} abs={} old={} file_only={}: {}", present, line, range, abs, old, file_only, r.stdout);

    // Fail-open: no rctags resolvable → "not checked", not "clean".
    let r = braim(&data, &["audit"], None, &proj, None, &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("rctags unavailable — symbol anchors not checked"), "{}", r.stdout);
    assert!(stale_ids(&r.stdout).is_empty());
}

// ─────── Item 3: short receipts, once-only verification notice, the fire log ───────

fn stop_payload_no_tag(cwd: &Path) -> String {
    serde_json::json!({
        "hook_event_name": "Stop",
        "last_assistant_message": "This is a reply with no braim tag at all.",
        "cwd": cwd.to_string_lossy()
    })
    .to_string()
}

fn fires_log_lines(home: &Path) -> Vec<serde_json::Value> {
    let dir = home.join(".claude").join("braim_stop_state");
    let path = fs::read_dir(&dir)
        .expect("state dir exists")
        .find_map(|e| {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".fires.jsonl") { Some(e.path()) } else { None }
        })
        .expect("a *.fires.jsonl file exists next to the state file");
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_missing_tag_gets_the_long_notice_once_then_the_short_repeat_and_never_requotes_the_reply() {
    let s = Scratch::new("notag");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);
    let home = root.join("home_for_test");
    let gate = || braim(&data, &["stop-gate"], Some(&stop_payload_no_tag(&root)), &root, Some(&bin), &[]);

    // First fire this store has ever seen: the long B.1 notice (names the
    // standing policy, invites the model to verify the gate is real, and
    // references — never re-quotes in full — the gated reply).
    let r = gate();
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("standing policy"), "{}", r.stderr);
    assert!(r.stderr.contains("Before tagging, reconsider it once"), "{}", r.stderr);
    assert!(r.stderr.contains("This is a reply with no braim tag"), "first line referenced: {}", r.stderr);
    assert!(!r.stderr.contains("---- YOUR PREVIOUS REPLY"), "the full-reply echo must be gone: {}", r.stderr);
    assert!(r.stderr.contains("sha1 "), "{}", r.stderr);

    // Second fire, same store: the short B.2 repeat, not the long notice
    // again, and small — the acceptance budget (<400 bytes per miss) applies
    // to this steady-state repeat, not the one-time long form above.
    let r = gate();
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Tag missing."), "{}", r.stderr);
    assert!(!r.stderr.contains("standing policy"), "B.1 must not repeat: {}", r.stderr);
    assert!(!r.stderr.contains("---- YOUR PREVIOUS REPLY"), "{}", r.stderr);
    assert!(r.stderr.len() < 400, "B.2 should be small: {} bytes: {}", r.stderr.len(), r.stderr);

    // Both fires logged one machine-parseable JSONL line each, kind no_tag.
    let fires = fires_log_lines(&home);
    assert_eq!(fires.len(), 2, "{:?}", fires);
    for f in &fires {
        assert_eq!(f["miss_kind"], "no_tag");
        assert_eq!(f["reply_sha1_8"].as_str().unwrap().len(), 8);
        assert!(f["fired_at"].as_i64().unwrap() > 0);
    }
}

#[test]
fn a_tag_only_follow_up_passes_and_fabricated_ids_still_fail_with_their_own_miss_kind() {
    let s = Scratch::new("firekinds");
    let data = s.sub("store");
    let root = s.sub("repo");
    write_index(&root, &[]);
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);
    let home = root.join("home_for_test");

    // A tag-only reply (no echoed prior content) with a well-formed,
    // honest tag passes cleanly.
    let r = braim(&data, &["stop-gate"], Some(&stop_payload(&root)), &root, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);

    // Fabricated id: logged IDs must be verified; this one does not exist.
    let payload = serde_json::json!({
        "hook_event_name": "Stop",
        "last_assistant_message": "done [braim: logged 999999 | audit: clean]",
        "cwd": root.to_string_lossy()
    })
    .to_string();
    let r = braim(&data, &["stop-gate"], Some(&payload), &root, Some(&bin), &[]);
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("no-such-node"), "{}", r.stderr);

    let fires = fires_log_lines(&home);
    assert_eq!(fires.last().unwrap()["miss_kind"], "bad_ids", "{:?}", fires);
}

// ─────────────── Item 2: session-aware `braim policy` curriculum ───────────────

fn prompt_payload(session_id: &str, cwd: &Path) -> String {
    serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": session_id,
        "cwd": cwd.to_string_lossy()
    })
    .to_string()
}

#[test]
fn the_policy_hooks_emit_full_curriculum_once_per_session_then_a_tail() {
    let s = Scratch::new("policy");
    let data = s.sub("store");
    let root = s.sub("repo");
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);

    let perturn = |session: &str| braim(&data, &["policy", "perturn"], Some(&prompt_payload(session, &root)), &root, Some(&bin), &[]);
    let traits = |session: &str| braim(&data, &["policy", "traits"], Some(&prompt_payload(session, &root)), &root, Some(&bin), &[]);

    // First prompt of session "s1": both hooks emit their full content.
    let r = perturn("s1");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("PER-TURN BRAIM LOGGING"), "{}", r.stdout);
    let r = traits("s1");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("EVIDENCE_CAPTURE_DISCIPLINE"), "{}", r.stdout);

    // Second prompt, same session: perturn (registered first) goes silent —
    // its content was already delivered — and traits (registered second, so
    // its output lands LAST) carries only the one-line provenance tail.
    let r = perturn("s1");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.trim().is_empty(), "perturn should be silent on turn 2: {}", r.stdout);
    let r = traits("s1");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("End your reply with the provenance tag"), "{}", r.stdout);
    assert!(!r.stdout.contains("EVIDENCE_CAPTURE_DISCIPLINE"), "full curriculum must not repeat: {}", r.stdout);
    assert!(r.stdout.len() < 300, "the tail should be small: {} bytes: {}", r.stdout.len(), r.stdout);

    // A new session gets the full curriculum again from both hooks.
    let r = perturn("s2");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("PER-TURN BRAIM LOGGING"), "{}", r.stdout);
    let r = traits("s2");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("EVIDENCE_CAPTURE_DISCIPLINE"), "{}", r.stdout);
}

#[test]
fn perturn_full_curriculum_adds_the_claude_md_standing_instruction_only_when_absent() {
    let s = Scratch::new("claudemd");
    let data = s.sub("store");
    let root = s.sub("repo");
    let bin = install_stub(&s.path().join("bin"));
    seed(&data, &root, &bin);

    let r = braim(&data, &["policy", "perturn"], Some(&prompt_payload("s1", &root)), &root, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("Per-turn provenance close"), "{}", r.stdout);

    // A CLAUDE.md that already mentions braim: the standing-instruction text
    // is not folded in a second time.
    fs::write(root.join("CLAUDE.md"), "# Project\n\nSee braim for provenance.\n").unwrap();
    let r = braim(&data, &["policy", "perturn"], Some(&prompt_payload("s2", &root)), &root, Some(&bin), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(!r.stdout.contains("Per-turn provenance close"), "{}", r.stdout);
}
