//! The one end-to-end proof for the rctags route enforcement (braim ID:660):
//! install → deny → declared bypass → fail-open → Stop-gate flag → audit, as
//! one system driven through the real entry points Claude Code's hook runner
//! and a human would use. No route_gate/anchor function is called directly.
//!
//! Hermetic: a tmp project with a crafted `.rctags/index.json` (two relative
//! function entries plus an absolute-path twin, mirroring the real 63-twin
//! index), a tmp `.braim` seeded via the real CLI, HOME overridden so the
//! Stop gate's state lands under the tmp dir, and a stub `rctags` first on
//! PATH. The live, mutable /home/magnimus/braim index and ~/.claude are never
//! touched. Harness copied from tests/audit_rot_loop.rs (no [lib] target).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_braim");

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("braim_e2e_{}_{}", test, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Scratch(p)
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

fn install_stub(bin_dir: &Path) -> PathBuf {
    fs::create_dir_all(bin_dir).unwrap();
    let stub = bin_dir.join("rctags");
    fs::write(
        &stub,
        r#"#!/bin/sh
case "$1" in
  health)
    case "${RCTAGS_STUB_MODE:-healthy}" in
      broken) echo "stub: broken" >&2; exit 1;;
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

/// `braim --data-dir <dir> --quiet <args>` with stdin, cwd and env, PATH
/// prefixed by the stub dir and HOME pointed into the scratch tree.
fn braim_with_stdin(
    dir: &Path,
    args: &[&str],
    stdin: &str,
    cwd: &Path,
    home: &Path,
    stub_bin: &Path,
    env: &[(&str, &str)],
) -> (i32, String, String) {
    let base = std::env::var("PATH").unwrap_or_default();
    let mut cmd = Command::new(BIN);
    cmd.arg("--data-dir").arg(dir).arg("--quiet").args(args)
        .current_dir(cwd)
        .env("PATH", format!("{}:{}", stub_bin.display(), base))
        .env("HOME", home)
        .env_remove("BRAIM_AUDIT_THRESHOLD")
        .env_remove("BRAIM_DATA_DIR")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to run braim");
    {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
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

fn decision(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    v["hookSpecificOutput"]["permissionDecision"].as_str().map(|s| s.to_string())
}

fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

fn new_id(stdout: &str) -> u32 {
    let idx = stdout.find("ID:").expect("prints the new id");
    stdout[idx + 3..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()
}

#[test]
fn the_route_enforcement_loop_closes_end_to_end() {
    let s = Scratch::new("loop");
    let stub_bin = install_stub(&s.sub("bin"));
    let tmp = s.sub("project");
    let src = s.sub("project/src");
    let home = s.sub("home");
    let data = tmp.join(".braim");
    let settings = tmp.join(".claude").join("settings.json");

    // Fixture index: two relative function entries, an absolute twin of one,
    // and the trap — removed_fn present ONLY at an absolute path.
    fs::create_dir_all(tmp.join(".rctags")).unwrap();
    fs::write(
        tmp.join(".rctags").join("index.json"),
        concat!(
            "[\n",
            "{\"name\":\"present_fn\",\"file\":\"src/lib.rs\",\"line_start\":3,\"line_end\":9,\"type\":\"function\",\"enriched\":false,\"language\":\"rust\"},\n",
            "{\"name\":\"other_fn\",\"file\":\"src/lib.rs\",\"line_start\":11,\"line_end\":20,\"type\":\"function\",\"enriched\":false,\"language\":\"rust\"},\n",
            "{\"name\":\"present_fn\",\"file\":\"/abs/src/lib.rs\",\"line_start\":3,\"line_end\":9,\"type\":\"function\",\"enriched\":false,\"language\":\"rust\"},\n",
            "{\"name\":\"removed_fn\",\"file\":\"/abs/src/lib.rs\",\"line_start\":30,\"line_end\":40,\"type\":\"function\",\"enriched\":false,\"language\":\"rust\"}\n",
            "]\n"
        ),
    )
    .unwrap();
    let run = |args: &[&str], stdin: &str, cwd: &Path, env: &[(&str, &str)]| {
        braim_with_stdin(&data, args, stdin, cwd, &home, &stub_bin, env)
    };

    // (1) install: the PreToolUse entry and the rctags pointer land.
    let (code, out, err) = run(&["init", "--settings", settings.to_str().unwrap()], "", &tmp, &[]);
    assert_eq!(code, 0, "init: {}{}", out, err);
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    let pre = v["hooks"]["PreToolUse"].as_array().expect("PreToolUse installed");
    assert_eq!(pre.len(), 1);
    assert_eq!(pre[0]["matcher"], "Bash");
    assert!(pre[0]["hooks"][0]["command"].as_str().unwrap().starts_with("braim route-gate ||"));
    assert_eq!(
        PathBuf::from(fs::read_to_string(data.join("rctags_bin")).expect("pointer").trim()),
        stub_bin.join("rctags")
    );

    // (2) a bare grep from inside the tree is denied; no ledger yet.
    let bypass_log = tmp.join(".rctags").join("bypass.log");
    let health_log = tmp.join(".rctags").join("route_gate_health.log");
    let (code, out, err) = run(&["route-gate"], &pretool("grep -rn foo src/", &src), &src, &[]);
    assert_eq!(code, 0, "{}", err);
    assert_eq!(decision(&out).as_deref(), Some("deny"), "{}", out);
    assert!(!bypass_log.exists());

    // (3) the one-shot bypass permit: unearned by default, armed only by a
    // genuine rctags call, consumed by exactly one bypass, then denied again
    // until rctags is used once more.
    let (code, out, _) = run(&["route-gate"], &pretool("RCTAGS_BYPASS=1 grep -rn foo src/", &src), &src, &[]);
    assert_eq!(code, 0);
    assert_eq!(decision(&out).as_deref(), Some("deny"), "no prior rctags use: unearned bypass must be denied, {}", out);
    assert!(!bypass_log.exists(), "a denied bypass writes no ledger line");

    for _ in 0..5 {
        // Arm: a real rctags call, independent of whether it is grep-family.
        let (code, _, err) = run(&["route-gate"], &pretool("rctags query present_fn", &src), &src, &[]);
        assert_eq!(code, 0, "{}", err);

        // Spend: the first bypass after that arm succeeds.
        let (code, out, _) = run(&["route-gate"], &pretool("RCTAGS_BYPASS=1 grep -rn foo src/", &src), &src, &[]);
        assert_eq!(code, 0);
        assert!(decision(&out).is_none(), "the earned bypass is allowed: {}", out);

        // The permit is gone: an immediate second bypass is denied again,
        // with no second rctags call in between.
        let (code, out, _) = run(&["route-gate"], &pretool("RCTAGS_BYPASS=1 grep -rn bar src/", &src), &src, &[]);
        assert_eq!(code, 0);
        assert_eq!(decision(&out).as_deref(), Some("deny"), "a second bypass without a new rctags call must be denied, {}", out);
    }
    let l = lines(&bypass_log);
    assert_eq!(l.len(), 5, "only the 5 EARNED bypasses are logged, not the denied attempts: {:?}", l);
    assert!(l.iter().all(|x| x.ends_with("\tRCTAGS_BYPASS=1 grep -rn foo src/")), "{:?}", l);
    assert!(!tmp.join(".rctags").join("bypass_permit").exists(), "the last permit was consumed, none left armed");

    // (4) rctags down: a bare grep fails open, to the health log only.
    let (code, out, _) = run(&["route-gate"], &pretool("grep foo", &src), &src, &[("RCTAGS_STUB_MODE", "broken")]);
    assert_eq!(code, 0);
    assert!(decision(&out).is_none(), "fail-open: {}", out);
    assert_eq!(lines(&health_log).len(), 1);
    assert_eq!(lines(&bypass_log).len(), 5);

    // (5) the Stop gate files exactly one tool-bypass item and does not
    // block the next turn for the rise it caused.
    let stop = serde_json::json!({
        "hook_event_name": "Stop",
        "last_assistant_message": "done [braim: none | audit: clean]",
        "cwd": src.to_string_lossy()
    })
    .to_string();
    let (code, _, err) = run(&["stop-gate"], &stop, &src, &[]);
    assert_eq!(code, 0, "stop-gate: {}", err);
    let (_, out, _) = run(&["dream", "review", "--count", "--json"], "", &tmp, &[]);
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["tool-bypass"], 1, "{}", out);
    assert_eq!(v["total"], 1, "{}", out);
    let items: Vec<serde_json::Value> = serde_json::from_str(&fs::read_to_string(data.join("reviews.json")).unwrap()).unwrap();
    assert_eq!(items.len(), 1, "one item, not five: {:?}", items);
    let note = items[0]["note"].as_str().unwrap();
    assert_eq!(note.matches("RCTAGS_BYPASS=1 grep -rn foo src/").count(), 5, "{}", note);
    let (code, _, err) = run(&["stop-gate"], &stop, &src, &[]);
    assert_eq!(code, 0, "second stop-gate blocked: {}", err);
    let items: Vec<serde_json::Value> = serde_json::from_str(&fs::read_to_string(data.join("reviews.json")).unwrap()).unwrap();
    assert_eq!(items.len(), 1);

    // (6) audit: a symbol anchor is stale only when rctags cannot find it at
    // exactly that file — not when a line anchor is cited, and not when an
    // absolute twin exists.
    let (code, out, err) = run(&["concept", "add", "Alpha: a concept", "--domains", "t", "--sources", "code:src/lib.rs:present_fn"], "", &tmp, &[]);
    assert_eq!(code, 0, "{}{}", out, err);
    let (code, out, err) = run(&["concept", "add", "Beta: a concept", "--domains", "t", "--sources", "code:src/lib.rs:other_fn"], "", &tmp, &[]);
    assert_eq!(code, 0, "{}{}", out, err);
    let add = |label: &str, source: &str| -> u32 {
        let (code, out, err) = run(
            &["statement", "add", label, "--domains", "t", "--sources", &format!("{},doc:spec.md:1", source), "--depends", "1:0.6,2:0.4"],
            "", &tmp, &[],
        );
        assert_eq!(code, 0, "{}: {}{}", source, out, err);
        new_id(&out)
    };
    let present = add("Alpha and Beta share present_fn", "code:src/lib.rs:present_fn");
    let removed = add("Alpha and Beta share removed_fn", "code:src/lib.rs:removed_fn");
    let line = add("Alpha and Beta share a line anchor", "code:src/lib.rs:12");
    let (code, out, err) = run(&["audit"], "", &tmp, &[]);
    assert_eq!(code, 0, "{}{}", out, err);
    let section: Vec<&str> = out
        .lines()
        .skip_while(|l| !l.contains("Stale symbol anchors"))
        .skip(1)
        .take_while(|l| !l.starts_with("──"))
        .collect();
    let stale: Vec<String> = section.iter().filter(|l| l.contains("✗ ID:")).map(|l| l.trim().to_string()).collect();
    assert_eq!(stale.len(), 1, "exactly one stale anchor: {:?}", section);
    assert!(stale[0].starts_with(&format!("✗ ID:{} ", removed)), "{:?}", stale);
    assert!(!out.contains(&format!("✗ ID:{} ", present)));
    assert!(!out.contains(&format!("✗ ID:{} ", line)));

    // Negative control: outside any indexed tree the gate allows and logs nothing.
    let elsewhere = s.sub("elsewhere/src");
    let (code, out, _) = run(&["route-gate"], &pretool("grep -rn foo src/", &elsewhere), &elsewhere, &[]);
    assert_eq!(code, 0);
    assert!(decision(&out).is_none(), "{}", out);
    assert!(!s.0.join("elsewhere").join(".rctags").exists());
    assert!(!elsewhere.join(".rctags").exists());
    assert_eq!(lines(&bypass_log).len(), 5);
    assert_eq!(lines(&health_log).len(), 1);
}
