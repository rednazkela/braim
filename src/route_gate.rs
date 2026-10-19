//! PreToolUse hook on Bash: deny grep-family commands inside an
//! rctags-indexed tree, log declared bypasses, fail open when rctags is down.
//!
//! Agents defect from rctags to grep because grep has a massive prior and
//! never hard-fails; prose instructions lose to that every time. So this gate
//! removes the cheap path: a bare `grep` under an indexed root is denied with
//! the exact replacement command, and `RCTAGS_BYPASS=1 grep ...` passes but is
//! recorded by the gate itself — the agent never writes the ledger, so the
//! bypass is declared by construction. The Stop gate (src/stop_gate.rs) reads
//! that ledger and files one `tool-bypass` review item per threshold crossing;
//! a dream session then makes the licensed-vs-defection call this script
//! cannot.
//!
//! Exit-code contract (C1): this module ALWAYS exits 0. A deny is signalled
//! only by a `permissionDecision: "deny"` JSON on stdout. Claude Code treats
//! any exit 2 from a PreToolUse hook as a blocking error regardless of what
//! was printed, and a stale binary that does not know this subcommand exits 2
//! from clap — so a nonzero exit can never be allowed to MEAN deny, or the two
//! would be indistinguishable. The settings.json command `braim init` writes
//! (src/bootstrap.rs) turns any nonzero exit into an explicit allow.
//!
//! Dispatch: `Commands::RouteGate` is listed in `is_read_only` (src/main.rs),
//! so the store is opened with `Braim::new` and no `.braim.lock` is taken.
//! The handler exits via `std::process::exit`, which skips Drop; opened as a
//! writer, every grep-family Bash call in every session would strand the lock
//! (braim ID:326).

use crate::bootstrap;
use crate::graph::Braim;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

const FAMILY: [&str; 5] = ["grep", "rg", "egrep", "fgrep", "ag"];

/// Tokens after which the next token is in command position again.
const INTRODUCERS: [&str; 10] = ["|", "||", ";", "&&", "&", "(", "$(", "`", "-exec", "-execdir"];

/// Tokens that forward their remaining arguments to another command, so the
/// token after them (and after any of their flags) is still in command
/// position. `git` is here for `git grep`; `git commit -m grep` is excluded by
/// the command-position rule since `commit` is not a wrapper.
const WRAPPERS: [&str; 13] = [
    "sudo", "env", "time", "nice", "xargs", "command", "exec", "git", "nohup", "bash", "sh",
    "zsh", "dash",
];

const SEPARATOR_GLYPHS: [&str; 9] = ["||", "&&", "$(", "|", ";", "&", "(", ")", "`"];

/// Split on whitespace, then split separator glyphs off as their own tokens,
/// so `cat f|grep x` and `x=$(grep y)` tokenize the same way as their spaced
/// forms. Shell quoting is deliberately NOT parsed (recorded v1 non-goal):
/// `echo "a | grep b"` is a known false positive whose cost is one logged
/// bypass retry, never a silent miss.
fn tokenize(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in cmd.split_whitespace() {
        let mut rest = word;
        let mut cur = String::new();
        'outer: while !rest.is_empty() {
            for g in SEPARATOR_GLYPHS.iter() {
                if rest.starts_with(g) {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                    out.push(g.to_string());
                    rest = &rest[g.len()..];
                    continue 'outer;
                }
            }
            let c = rest.chars().next().unwrap();
            cur.push(c);
            rest = &rest[c.len_utf8()..];
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

fn is_env_assignment(tok: &str) -> bool {
    let mut chars = tok.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    let mut saw_eq = false;
    for c in chars {
        if c == '=' {
            saw_eq = true;
            break;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    saw_eq
}

/// True when a grep-family binary is invoked IN COMMAND POSITION anywhere in
/// `cmd`. A bare regex false-positives on `rctags query grep` — the very
/// replacement the gate steers toward — and on `echo 'use grep later'`, while
/// missing `/usr/bin/grep` and `$(grep ...)`; so this walks tokens and tracks
/// whether the next token names a command.
fn is_grep_family(cmd: &str) -> bool {
    let tokens = tokenize(cmd);
    let mut command_position = true;
    let mut after_wrapper = false;
    for tok in &tokens {
        let t = tok.as_str();
        if INTRODUCERS.contains(&t) {
            command_position = true;
            after_wrapper = false;
            continue;
        }
        if !command_position {
            continue;
        }
        if is_env_assignment(t) {
            continue;
        }
        if after_wrapper && t.starts_with('-') {
            continue;
        }
        if WRAPPERS.contains(&t) {
            after_wrapper = true;
            continue;
        }
        let stripped = t.trim_start_matches(|c| c == '\'' || c == '"');
        let base = stripped.rsplit('/').next().unwrap_or(stripped);
        if FAMILY.contains(&base) {
            return true;
        }
        command_position = false;
        after_wrapper = false;
    }
    false
}

/// The nearest ancestor of `cwd` (itself included) holding
/// `.rctags/index.json`, or None when `cwd` is outside any indexed tree —
/// the v1 cwd-containment scope limit (spec L49).
pub(crate) fn rctags_root_for(cwd: &Path) -> Option<PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        if d.join(".rctags").join("index.json").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// Whitespace-delimited `RCTAGS_BYPASS=1` anywhere in the command — the
/// spec's `*RCTAGS_BYPASS=1*` glob, tightened so `RCTAGS_BYPASS=10` does not
/// qualify.
fn has_bypass_token(cmd: &str) -> bool {
    cmd.split_whitespace().any(|t| t == "RCTAGS_BYPASS=1")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn append_line(path: &Path, line: &str) {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", line);
    }
}

/// One tab-separated line per declared bypass, at the indexed ROOT (not the
/// hook's cwd), so the Stop gate finds a single ledger per repo.
fn log_bypass(root: &Path, cwd: &str, cmd: &str) {
    append_line(
        &root.join(".rctags").join("bypass.log"),
        &format!("{}\t{}\t{}", now_iso(), cwd, cmd),
    );
}

/// Fail-open events go to their own log: bypass.log is read by the Stop gate
/// as agent-declared bypasses only, and a down rctags is not a defection.
fn log_fail_open(root: &Path, cwd: &str, reason: &str, cmd: &str) {
    append_line(
        &root.join(".rctags").join("route_gate_health.log"),
        &format!("{}\t{}\t{}\t{}", now_iso(), cwd, reason, cmd),
    );
}

/// Probe `rctags health` from the indexed root. `rctags health` reads only
/// `./.rctags/index.json` relative to its cwd and does not walk up (measured
/// 2026-10-08: run from a subdirectory it reported `indexed_files: 0 ...
/// verdict: OK` and created a stray empty `.rctags/`), so the probe must run
/// from the root, never from the hook's own cwd. The binary comes from PATH
/// first, then from the pointer `braim init` recorded (src/bootstrap.rs), since
/// a hook may be spawned with a leaner PATH than the shell that ran init.
fn rctags_is_healthy(root: &Path, data_dir: &Path) -> Result<(), String> {
    let bin = bootstrap::resolve_rctags(data_dir).ok_or_else(|| "rctags-not-found".to_string())?;
    let out = std::process::Command::new(&bin)
        .arg("health")
        .current_dir(root)
        .output()
        .map_err(|e| format!("rctags-spawn-failed:{}", e.kind()))?;
    bootstrap::health_verdict(&out)
}

fn deny_json(root: &Path) -> String {
    let reason = format!(
        "rctags-indexed repo ({}). Symbol lookup:\n  rctags query <term>\nSlice read:\n  rctags load <file> <start> <end>\nConceptual search, when you do not know the name:\n  ccc search \"<what it does>\"\nComments / string literals / unindexed files are grep's territory: rerun your exact command prefixed with RCTAGS_BYPASS=1 — the bypass is recorded.",
        root.display()
    );
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason
        }
    })
    .to_string()
}

/// Run the gate against stdin (the PreToolUse hook payload) and return the
/// process exit code, which is always 0. See the module doc for why a deny
/// is never a nonzero exit.
pub fn run(braim: &Braim) -> i32 {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    if let Some(tool) = payload.get("tool_name").and_then(|v| v.as_str()) {
        if tool != "Bash" {
            return 0;
        }
    }
    let cmd = match payload
        .get("tool_input")
        .and_then(|t| t.get("command"))
        .and_then(|c| c.as_str())
    {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => return 0,
    };
    let cwd: PathBuf = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let cwd_str = cwd.to_string_lossy().to_string();

    let bypass = has_bypass_token(&cmd);
    if !is_grep_family(&cmd) {
        return 0;
    }
    let root = match rctags_root_for(&cwd) {
        Some(r) => r,
        None => return 0,
    };
    if bypass {
        log_bypass(&root, &cwd_str, &cmd);
        return 0;
    }
    if let Err(reason) = rctags_is_healthy(&root, &braim.data_dir) {
        log_fail_open(&root, &cwd_str, &reason, &cmd);
        return 0;
    }
    println!("{}", deny_json(&root));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_family_in_command_position_is_detected() {
        let positives = [
            "grep foo src/",
            "rg foo",
            "egrep -i x f",
            "fgrep x f",
            "ag x",
            "/usr/bin/grep foo",
            "/bin/rg foo",
            "./rg foo",
            "x=$(grep -c foo bar)",
            "echo `grep foo bar`",
            "echo $(rg x)",
            "cat f | grep x",
            "ls; grep x",
            "ls && grep x",
            "ls || grep x",
            "(grep x)",
            "find . | xargs grep x",
            "find . -exec grep x {} \\;",
            "sudo grep x f",
            "git grep x",
            "time rg x",
            "env FOO=1 grep x",
            "bash -c 'grep x'",
            "sh -c \"rg x\"",
            "xargs -0 grep x",
            "FOO=1 grep x",
            "grep",
        ];
        for p in positives {
            assert!(is_grep_family(p), "expected true for {:?}", p);
        }
    }

    #[test]
    fn grep_outside_command_position_is_not_detected() {
        let negatives = [
            "cat foo.txt",
            "ls -la",
            "rctags query grep",
            "rctags load src/route_gate.rs 1 9",
            "echo grepping",
            "cargo test regroup",
            "agrep x",
            "--grep x",
            "mygrep x",
            "echo 'use grep later'",
            "git commit -m 'grep'",
            "git log --grep=foo",
            "x=rg",
            "rg=1 ls",
            "pgrep foo",
            "ripgrep x",
            "ls | wc -l",
        ];
        for n in negatives {
            assert!(!is_grep_family(n), "expected false for {:?}", n);
        }
    }

    /// Recorded v1 non-goals, pinned so a change to them is a visible decision.
    #[test]
    fn recorded_non_goals_keep_their_accepted_behaviour() {
        // Quoting is not parsed: an accepted false positive (cost: one logged
        // bypass retry).
        assert!(is_grep_family("echo \"a | grep b\""));
        // Wrapper flags taking a value are not modelled: an accepted miss.
        assert!(!is_grep_family("sudo -u root grep x"));
    }

    #[test]
    fn bypass_token_is_exact_and_position_free() {
        assert!(has_bypass_token("RCTAGS_BYPASS=1 grep x"));
        assert!(has_bypass_token("FOO=1 RCTAGS_BYPASS=1 grep x"));
        assert!(has_bypass_token("cat f | RCTAGS_BYPASS=1 grep x"));
        assert!(!has_bypass_token("RCTAGS_BYPASS=10 grep x"));
        assert!(!has_bypass_token("grep RCTAGS_BYPASS=1x"));
    }

    #[test]
    fn deny_message_names_every_channel_the_agent_may_use_instead_of_grep() {
        // The denial is the only place an agent is told where to go instead.
        // Naming rctags alone sent conceptual searches ("where do we do
        // something like X") back to grep via the bypass, because exact lookup
        // genuinely could not answer them.
        let reason = deny_json(Path::new("/repo"));
        for channel in ["rctags query", "rctags load", "ccc search"] {
            assert!(
                reason.contains(channel),
                "denial must name `{}`; got: {}",
                channel,
                reason
            );
        }
    }

    #[test]
    fn rctags_root_is_the_nearest_indexed_ancestor_or_none() {
        let base = std::env::temp_dir().join(format!("braim_rg_root_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("repo");
        std::fs::create_dir_all(root.join(".rctags")).unwrap();
        std::fs::write(root.join(".rctags").join("index.json"), "[]").unwrap();
        let nested = root.join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(rctags_root_for(&nested).as_deref(), Some(root.as_path()));
        assert_eq!(rctags_root_for(&root).as_deref(), Some(root.as_path()));

        let outside = base.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        assert_eq!(rctags_root_for(&outside), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}
