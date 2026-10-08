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
/// set and answers per RCTAGS_STUB_MODE (healthy | empty | broken); `query
/// <sym> --exact` answers from `./.rctags/index.json`, one entry per line.
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
      *) printf 'indexed_files: 3\nmissing_files: 0 (0.0%%)\nunparsed_files: 0\nverdict: OK\n'; exit 0;;
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

    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=1 grep foo", &sub)), &sub, Some(&bin), &[]);
    assert_eq!(r.code, 0);
    assert!(deny_reason(&r.stdout).is_none(), "bypass must be allowed: {}", r.stdout);
    let l = lines(&log);
    assert_eq!(l.len(), 1, "exactly one line at the ROOT, not the cwd: {:?}", l);
    assert!(!sub.join(".rctags").exists(), "nothing is written under the cwd");
    let fields: Vec<&str> = l[0].split('\t').collect();
    assert_eq!(fields.len(), 3, "ts\\tcwd\\tcommand: {:?}", fields);
    assert!(fields[0].ends_with('Z') && fields[0].contains('T'), "ISO-8601 timestamp: {}", fields[0]);
    assert_eq!(fields[1], sub.to_string_lossy());
    assert_eq!(fields[2], "RCTAGS_BYPASS=1 grep foo");

    // Mid-pipeline and env-prefixed forms also qualify.
    let r = braim(&data, &["route-gate"], Some(&pretool("cat f | RCTAGS_BYPASS=1 grep x", &sub)), &sub, Some(&bin), &[]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none());
    assert_eq!(lines(&log).len(), 2);

    // Negative controls.
    let r = braim(&data, &["route-gate"], Some(&pretool("grep foo", &sub)), &sub, Some(&bin), &[]);
    assert!(deny_reason(&r.stdout).is_some(), "without the token the same command is denied");
    assert_eq!(lines(&log).len(), 2, "a denial writes no bypass line");

    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=10 grep foo", &sub)), &sub, Some(&bin), &[]);
    assert!(deny_reason(&r.stdout).is_some(), "RCTAGS_BYPASS=10 is not a bypass");
    assert_eq!(lines(&log).len(), 2);

    let r = braim(&data, &["route-gate"], Some(&pretool("RCTAGS_BYPASS=1 cat foo", &sub)), &sub, Some(&bin), &[]);
    assert!(r.code == 0 && deny_reason(&r.stdout).is_none());
    assert_eq!(lines(&log).len(), 2, "a bypass token on a non-grep command is not a defection");
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
