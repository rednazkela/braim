//! Bootstrap: give an agent the working setup in one command.
//!
//! Adoption is single-player first (braim ID:223). Teammates start with no
//! graphs, so day-one value cannot be consuming shared knowledge — there is
//! none. Bare `braim init` delivers the setup that already works solo: a
//! local graph plus the policy hooks that automate the evidence discipline, so
//! their agent behaves correctly from the first turn. `--team --central`
//! layers sharing on top once several graphs exist.
//!
//! The policy payloads are embedded in the binary and emitted by `braim policy`
//! rather than read from a file. The hook wiring this replaces was
//! `cat /home/<user>/.claude/.../braim_perturn_logging.json` — an absolute path
//! into one person's home directory, invoking a tool Windows does not ship.
//! Emitting from the binary is portable by construction (braim ID:225/227) and
//! keeps the policy version-locked to the braim that enforces it.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Per-turn marker-logging discipline, injected by a `UserPromptSubmit` hook.
const PERTURN: &str = include_str!("../policies/perturn_logging.json");
/// Compaction discipline, injected by a `PreCompact` hook.
const COMPACTION: &str = include_str!("../policies/compaction_rule.txt");
/// Evidence-capture traits an operator keeps in agent memory.
const TRAITS: &str = include_str!("../policies/memory_braim_traits.md");

pub fn policy_body(name: &str) -> Result<String, String> {
    match name.trim().to_lowercase().replace('_', "-").as_str() {
        "perturn" | "per-turn" | "logging" => Ok(PERTURN.to_string()),
        "compaction" | "precompact" => Ok(wrap_hook_context("PreCompact", COMPACTION)),
        "traits" | "memory" => Ok(wrap_hook_context("UserPromptSubmit", TRAITS)),
        other => Err(format!(
            "Error: unknown policy '{}' (expected perturn, compaction, or traits)",
            other
        )),
    }
}

/// Compaction and traits both ship as prose; each hook wants the same
/// `additionalContext` envelope the per-turn (perturn) payload already uses,
/// so the harness parses it as structured hookSpecificOutput instead of
/// falling back to treating raw stdout as plain text.
fn wrap_hook_context(event: &str, body: &str) -> String {
    let payload = json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "additionalContext": body
        }
    });
    // Compact, not pretty: this is wire format for the hook harness to parse,
    // never something a human reads directly, and the one-line provenance
    // tail (item 2.2) is sized to the acceptance budget (<160 injected bytes)
    // against the actual additionalContext payload, not an envelope padded
    // with pretty-printing whitespace on top of it.
    serde_json::to_string(&payload).unwrap_or_else(|_| body.to_string())
}

// ─────────────────────────────────────────────────────────────────────────
// Session-aware curriculum (item 2 of the provenance-reframing handoff).
// ─────────────────────────────────────────────────────────────────────────
//
// `perturn` and `traits` are both wired to UserPromptSubmit and used to emit
// their FULL payload on every single prompt — the curriculum the model
// actually needs once per session, repeated in full on every turn after.
// Now each tracks, in the shared stop-gate state file (no new state
// location — house rule), whether IT has already shown its full content
// THIS session:
//
//   - first prompt of a session (or the tracked moment is older than the
//     TTL): the full payload, plus (perturn only) the CLAUDE.md standing
//     instruction text when the project's CLAUDE.md does not already
//     mention braim.
//   - every later prompt: perturn goes silent (its content was already
//     delivered); traits emits just the one-line provenance tail. Since
//     traits is the SECOND UserPromptSubmit hook in `install_hooks`'s
//     wiring order, its output lands last — the recency-weighted position
//     the tail needs.
//
// perturn and traits are tracked under separate keys
// (`policy_full_at_<name>` / `policy_session_key_<name>`) so the two
// commands — separate processes, invoked back to back — don't observe each
// other's write and miscompute "first of session" for themselves.

/// The one-line tail appendix A.4/item 2.2 specifies for every prompt after
/// the first. Must be the LAST line injected — recency-weighted
/// instruction-following forgets the tag exactly when it sits far from the
/// generation point — which is why `traits`, not `perturn`, carries it: see
/// the module-level note above on hook ordering.
const PROVENANCE_TAIL: &str =
    "End your reply with the provenance tag: [braim: logged N,M] for IDs a braim command printed this turn, or [braim: none].";

/// Appendix A.4: the CLAUDE.md standing instruction, injected into the full
/// `perturn` curriculum when the project's own CLAUDE.md doesn't already
/// carry it — teaching the discipline even where the project hasn't (yet)
/// written it down, rather than editing CLAUDE.md on the model's behalf.
const CLAUDE_MD_STANDING_INSTRUCTION: &str = "Per-turn provenance close. This project keeps an engineering decision record in braim (a local CLI knowledge graph; `braim --help`). End every reply with one literal tag reporting what you RECORDED this turn: `[braim: logged N,M]` citing IDs that `braim statement add` / `braim concept add` printed back to you this turn, or `[braim: none]` (optionally `[braim: none: <why>]`) when the turn produced nothing worth recording — an honest and common outcome. Optional second clause when your reply leaned on stored nodes: `| audit: clean`, `| audit: K flagged <ids>`, or `| audit: skipped: <reason>`. The tag certifies actions taken in this session — nothing else. A Stop hook (registered in settings.json by the machine owner) checks the tag mechanically and will return your turn once if it is missing.";

/// Default TTL for a tracked "full curriculum already shown" moment: mostly
/// a safety net for the cwd+day fallback key (which already rotates daily on
/// its own) and for unusually long-lived real session ids. Override with
/// BRAIM_POLICY_TTL_SECS.
fn policy_ttl_secs() -> i64 {
    std::env::var("BRAIM_POLICY_TTL_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(6 * 3600)
}

/// True when `cwd`'s CLAUDE.md is missing or doesn't mention braim at all —
/// the signal item 2.1 uses to decide whether the standing-instruction text
/// is worth including in the full curriculum.
fn claude_md_lacks_braim_section(cwd: &Path) -> bool {
    match fs::read_to_string(cwd.join("CLAUDE.md")) {
        Ok(text) => !text.to_lowercase().contains("braim"),
        Err(_) => true,
    }
}

/// Reads and updates `policy_session_key_<which>` / `policy_full_at_<which>`
/// in the shared stop-gate state file. Returns true exactly when THIS
/// command, for THIS session key, should emit its full content — a new
/// session key, or the last full-content moment is older than the TTL.
/// Each call both decides and records: the field is always rewritten
/// (to the current session key, and to `now` when the decision is "full"),
/// so staleness never carries forward on its own.
fn policy_is_first_turn(data_dir: &Path, which: &str, session_key: &str, now: i64) -> bool {
    let path = crate::stop_gate::statefile_for(data_dir);
    let mut v = crate::stop_gate::load_state_value(&path);
    if !v.is_object() {
        v = json!({});
    }
    let key_field = format!("policy_session_key_{}", which);
    let at_field = format!("policy_full_at_{}", which);
    let stored_key = v.get(&key_field).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let stored_at = v.get(&at_field).and_then(|x| x.as_i64()).unwrap_or(0);
    let first = stored_key != session_key || (now - stored_at) > policy_ttl_secs();

    let obj = v.as_object_mut().expect("just normalized to an object");
    obj.insert(key_field, Value::String(session_key.to_string()));
    if first {
        obj.insert(at_field, json!(now));
    }
    crate::stop_gate::save_state_value(&path, &v);
    first
}

/// `perturn`'s full body: the embedded JSON payload, with the CLAUDE.md
/// standing instruction folded into `additionalContext` when the project
/// lacks one. Falls back to the unmodified payload if the embedded JSON
/// somehow fails to parse (it always has — see
/// `perturn_payload_is_valid_json_for_the_hook`).
fn perturn_full_body(cwd: &Path) -> String {
    let mut v: Value = match serde_json::from_str(PERTURN) {
        Ok(v) => v,
        Err(_) => return PERTURN.to_string(),
    };
    if claude_md_lacks_braim_section(cwd) {
        if let Some(ctx) = v["hookSpecificOutput"]["additionalContext"].as_str() {
            let combined = format!("{}\n\n{}", ctx, CLAUDE_MD_STANDING_INSTRUCTION);
            v["hookSpecificOutput"]["additionalContext"] = Value::String(combined);
        }
    }
    serde_json::to_string(&v).unwrap_or_else(|_| PERTURN.to_string())
}

/// The real entry point the `braim policy` hooks (perturn, traits) call at
/// runtime — session-aware, unlike `policy_body` above (kept as the pure,
/// always-full accessor the unit tests and `compaction` use). `compaction`
/// and any other name fall straight through to `policy_body`: only
/// perturn/traits are subject to the per-session curriculum.
pub fn policy_body_for_turn(
    name: &str,
    data_dir: &Path,
    session_key: &str,
    cwd: &Path,
    now: i64,
) -> Result<String, String> {
    match name.trim().to_lowercase().replace('_', "-").as_str() {
        "perturn" | "per-turn" | "logging" => {
            if policy_is_first_turn(data_dir, "perturn", session_key, now) {
                Ok(perturn_full_body(cwd))
            } else {
                // Already delivered this session; `traits` (registered
                // second) carries the recurring one-line tail instead.
                Ok(String::new())
            }
        }
        "traits" | "memory" => {
            if policy_is_first_turn(data_dir, "traits", session_key, now) {
                Ok(wrap_hook_context("UserPromptSubmit", TRAITS))
            } else {
                Ok(wrap_hook_context("UserPromptSubmit", PROVENANCE_TAIL))
            }
        }
        _ => policy_body(name),
    }
}

#[derive(Debug, PartialEq)]
pub enum Change {
    Added(String),
    AlreadyPresent(String),
}

pub struct BootstrapReport {
    pub settings_path: PathBuf,
    pub changes: Vec<Change>,
    pub graph_dir: PathBuf,
    pub graph_created: bool,
    pub central: Option<String>,
}

/// True when some hook of `event` already invokes this exact `command`, so
/// re-running the bootstrap is idempotent instead of stacking duplicate
/// injections. Matched per (event, command) rather than per event alone,
/// since one event (UserPromptSubmit) can carry more than one braim hook
/// (perturn + traits).
fn has_braim_hook(settings: &Value, event: &str, command: &str) -> bool {
    settings
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|e| e.as_array())
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| c.contains(command))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn hook_entry(event: &str, command: &str, status: &str) -> Value {
    let mut entry = json!({
        "hooks": [{
            "type": "command",
            // No shell tools, no absolute paths: the binary the teammate already
            // installed emits its own policy or runs its own check, so this line
            // is identical on every platform and every machine.
            "command": command,
            "timeout": 60,
            "statusMessage": status
        }],
        // Recorded so a human reading settings.json can tell which tool owns the
        // entry and re-run its bootstrap.
        "_braim": format!("installed by `braim init` for {}", event)
    });
    // Only PreToolUse is per-tool; the other events take no matcher.
    if event == "PreToolUse" {
        entry["matcher"] = json!("Bash");
    }
    entry
}

/// The route gate's hook line. The `||` guard is the only place able to
/// catch an exit this binary's own code never produces: a stale `braim` that
/// does not know the subcommand exits 2 from clap, a panic exits 101, a
/// missing binary 127 — and Claude Code treats any nonzero PreToolUse exit as
/// a block. Without the guard, upgrading braim ahead of the installed binary
/// would deny every Bash call.
pub const ROUTE_GATE_COMMAND: &str = "braim route-gate || echo '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"allow\",\"permissionDecisionReason\":\"route-gate unavailable, failing open\"}}'";

/// Merge braim's hooks into an existing settings file without disturbing
/// anything else in it. Never rewrites or removes a hook it did not add.
/// `with_route_gate` is decided by `check_rctags_health` at init time: a gate
/// that denies grep while its replacement tool is broken teaches agents to
/// hate the gate, so it is only installed when rctags answered healthy.
pub fn install_hooks(settings_path: &Path, with_route_gate: bool) -> Result<Vec<Change>, String> {
    let mut settings: Value = if settings_path.exists() {
        let text = fs::read_to_string(settings_path)
            .map_err(|e| format!("Failed to read {}: {}", settings_path.display(), e))?;
        if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text).map_err(|e| {
                format!(
                    "Failed to parse {} — fix or move it before bootstrapping: {}",
                    settings_path.display(),
                    e
                )
            })?
        }
    } else {
        json!({})
    };

    if !settings.is_object() {
        return Err(format!(
            "Error: {} does not contain a JSON object",
            settings_path.display()
        ));
    }

    let mut wanted = vec![
        ("UserPromptSubmit", "braim policy perturn", "Injecting per-turn braim logging discipline"),
        ("UserPromptSubmit", "braim policy traits", "Injecting braim evidence-capture traits discipline"),
        ("PreCompact", "braim policy compaction", "Injecting braim compaction discipline"),
        ("Stop", "braim stop-gate", "Enforcing the per-turn braim declaration"),
    ];
    if with_route_gate {
        wanted.push(("PreToolUse", ROUTE_GATE_COMMAND, "Routing code search through rctags"));
    }

    let mut changes = Vec::new();
    for (event, command, status) in wanted {
        let label = format!("{} ({})", event, command);
        if has_braim_hook(&settings, event, command) {
            changes.push(Change::AlreadyPresent(label));
            continue;
        }
        let entry = hook_entry(event, command, status);
        settings
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("Error: settings.hooks is not an object")?
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("Error: settings.hooks.{} is not an array", event))?
            .push(entry);
        changes.push(Change::Added(label));
    }

    if changes.iter().any(|c| matches!(c, Change::Added(_))) {
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        let text = serde_json::to_string_pretty(&settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        fs::write(settings_path, text)
            .map_err(|e| format!("Failed to write {}: {}", settings_path.display(), e))?;
    }
    Ok(changes)
}

/// Record where central lives so later `export` calls need no path argument.
/// Written next to the graph rather than into settings.json — it is braim's
/// state, not the agent harness's.
pub fn write_central_pointer(graph_dir: &Path, central: &str) -> Result<(), String> {
    fs::create_dir_all(graph_dir)
        .map_err(|e| format!("Failed to create {}: {}", graph_dir.display(), e))?;
    fs::write(graph_dir.join("central"), format!("{}\n", central.trim()))
        .map_err(|e| format!("Failed to record central pointer: {}", e))
}

pub fn read_central_pointer(graph_dir: &Path) -> Option<String> {
    fs::read_to_string(graph_dir.join("central"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Where the rctags binary was found at init time. Same shape and home as the
/// central pointer: braim's state, next to the graph, not the harness's. The
/// route gate reads it only when `rctags` is absent from the hook process's
/// PATH — a hook spawned from a leaner environment than the shell that ran
/// `braim init` (`env -i sh -c 'command -v rctags'` finds nothing, measured
/// 2026-10-08) would otherwise fail open on every call.
pub fn write_rctags_pointer(graph_dir: &Path, bin: &Path) -> Result<(), String> {
    fs::create_dir_all(graph_dir)
        .map_err(|e| format!("Failed to create {}: {}", graph_dir.display(), e))?;
    fs::write(graph_dir.join("rctags_bin"), format!("{}\n", bin.display()))
        .map_err(|e| format!("Failed to record rctags pointer: {}", e))
}

pub fn read_rctags_pointer(graph_dir: &Path) -> Option<PathBuf> {
    fs::read_to_string(graph_dir.join("rctags_bin"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

pub fn find_rctags_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("rctags"))
        .find(|candidate| candidate.is_file())
}

/// PATH first, the recorded pointer second, and only a pointer that still
/// names an existing file.
pub fn resolve_rctags(graph_dir: &Path) -> Option<PathBuf> {
    find_rctags_on_path().or_else(|| read_rctags_pointer(graph_dir).filter(|p| p.is_file()))
}

/// What "healthy" means for an `rctags health` run, shared by the init-time
/// probe and the per-invocation gate probe: exit 0, `verdict: OK`, and more
/// than zero indexed files. A zero-entry OK is what `rctags health` prints
/// from an unindexed directory (it creates an empty `.rctags/` as a side
/// effect), so it is vacuous and counts as unhealthy.
pub const HEALTH_NO_INDEX: &str = "indexed-files-zero";

pub fn health_verdict(out: &std::process::Output) -> Result<(), String> {
    if !out.status.success() {
        return Err(format!("rctags-exit-{}", out.status.code().unwrap_or(-1)));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("verdict: OK") {
        return Err("verdict-not-ok".to_string());
    }
    let indexed = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("indexed_files:"))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if indexed == 0 {
        return Err(HEALTH_NO_INDEX.to_string());
    }
    Ok(())
}

/// Init-time probe: resolve `rctags` from PATH and run `health` in
/// `project_dir`. Ok carries the absolute binary path to record; Err carries
/// the hint `braim init` prints INSTEAD of installing the route gate.
pub fn check_rctags_health(project_dir: &Path) -> Result<PathBuf, String> {
    let bin = find_rctags_on_path().ok_or_else(|| {
        "rctags not found on PATH — install it (cargo install --path /path/to/rctags) and run `rctags reindex .` before re-running `braim init`; the route gate was NOT installed".to_string()
    })?;
    let out = std::process::Command::new(&bin)
        .arg("health")
        .current_dir(project_dir)
        .output()
        .map_err(|e| format!("rctags could not be run ({}); the route gate was NOT installed", e))?;
    match health_verdict(&out) {
        Ok(()) => Ok(bin),
        Err(reason) if reason == HEALTH_NO_INDEX => Err(format!(
            "rctags reports no indexed files in {} — run `rctags reindex .` first, then re-run `braim init`; the route gate was NOT installed",
            project_dir.display()
        )),
        Err(reason) => Err(format!(
            "rctags health failed ({}) — rebuild or reinstall rctags, then re-run `braim init`; the route gate was NOT installed",
            reason
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("braim_boot_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn policies_are_embedded_and_addressable() {
        assert!(policy_body("perturn").unwrap().contains("UserPromptSubmit"));
        assert!(policy_body("traits").unwrap().len() > 100);
        assert!(policy_body("nonsense").is_err());
        // aliases
        assert_eq!(policy_body("per-turn").unwrap(), policy_body("perturn").unwrap());
    }

    #[test]
    fn perturn_payload_is_valid_json_for_the_hook() {
        let v: Value = serde_json::from_str(&policy_body("perturn").unwrap())
            .expect("the hook consumes this directly; it must parse");
        assert!(v["hookSpecificOutput"]["additionalContext"].is_string());
    }

    #[test]
    fn compaction_prose_is_wrapped_in_a_hook_envelope() {
        let v: Value = serde_json::from_str(&policy_body("compaction").unwrap()).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreCompact");
        assert!(v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("PRECOMPACT"));
    }

    #[test]
    fn traits_prose_is_wrapped_in_a_hook_envelope() {
        // Same envelope as compaction (braim ID:39): unwrapped markdown made
        // the harness fall back to plain-text injection instead of parsing
        // structured hookSpecificOutput.
        let v: Value = serde_json::from_str(&policy_body("traits").unwrap()).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert!(v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("EVIDENCE_CAPTURE_DISCIPLINE"));
    }

    #[test]
    fn install_creates_all_hooks_and_is_idempotent() {
        let dir = temp("install");
        let settings = dir.join("settings.json");

        let first = install_hooks(&settings, false).unwrap();
        assert_eq!(first.len(), 4);
        assert!(first.iter().all(|c| matches!(c, Change::Added(_))));

        let v: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        let ups = v["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(ups.len(), 2, "perturn + traits both live under UserPromptSubmit");
        assert_eq!(ups[0]["hooks"][0]["command"], "braim policy perturn");
        assert_eq!(ups[1]["hooks"][0]["command"], "braim policy traits");
        assert_eq!(v["hooks"]["PreCompact"][0]["hooks"][0]["command"], "braim policy compaction");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "braim stop-gate");

        // Re-running must not stack duplicates.
        let second = install_hooks(&settings, false).unwrap();
        assert!(second.iter().all(|c| matches!(c, Change::AlreadyPresent(_))));
        let v2: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v2["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn install_preserves_unrelated_settings_and_foreign_hooks() {
        let dir = temp("preserve");
        let settings = dir.join("settings.json");
        fs::write(
            &settings,
            serde_json::to_string_pretty(&json!({
                "model": "opus",
                "permissions": {"allow": ["WebSearch"]},
                "hooks": {
                    "UserPromptSubmit": [
                        {"hooks": [{"type": "command", "command": "echo mine"}]}
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        install_hooks(&settings, false).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();

        assert_eq!(v["model"], "opus", "unrelated settings survive");
        assert_eq!(v["permissions"]["allow"][0], "WebSearch");
        let ups = v["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(ups.len(), 3, "the existing hook is kept and both braim hooks are appended");
        assert_eq!(ups[0]["hooks"][0]["command"], "echo mine", "foreign hook untouched");
        assert_eq!(ups[1]["hooks"][0]["command"], "braim policy perturn");
        assert_eq!(ups[2]["hooks"][0]["command"], "braim policy traits");
    }

    #[test]
    fn install_refuses_to_clobber_unparseable_settings() {
        let dir = temp("broken");
        let settings = dir.join("settings.json");
        fs::write(&settings, "{ this is not json").unwrap();
        let err = install_hooks(&settings, false).unwrap_err();
        assert!(err.contains("fix or move it"), "got: {}", err);
        // the file must be left exactly as found
        assert_eq!(fs::read_to_string(&settings).unwrap(), "{ this is not json");
    }

    #[test]
    fn central_pointer_round_trips() {
        let dir = temp("central");
        assert!(read_central_pointer(&dir).is_none());
        write_central_pointer(&dir, "  ~/.braim_central \n").unwrap();
        assert_eq!(read_central_pointer(&dir).as_deref(), Some("~/.braim_central"));
    }

    #[test]
    fn rctags_pointer_round_trips() {
        let dir = temp("rctags_ptr");
        assert!(read_rctags_pointer(&dir).is_none());
        write_rctags_pointer(&dir, Path::new("/opt/bin/rctags")).unwrap();
        assert_eq!(read_rctags_pointer(&dir).as_deref(), Some(Path::new("/opt/bin/rctags")));
    }

    #[test]
    fn the_route_gate_is_installed_guarded_on_bash_and_only_when_asked() {
        let dir = temp("route_gate");
        let settings = dir.join("settings.json");

        let first = install_hooks(&settings, true).unwrap();
        assert_eq!(first.len(), 5);
        let v: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0]["matcher"], "Bash");
        assert_eq!(pre[0]["hooks"][0]["type"], "command");
        assert_eq!(pre[0]["hooks"][0]["command"], ROUTE_GATE_COMMAND);
        assert!(ROUTE_GATE_COMMAND.starts_with("braim route-gate || echo '{\"hookSpecificOutput\""));
        assert!(ROUTE_GATE_COMMAND.contains("\"permissionDecision\":\"allow\""));
        assert_eq!(pre[0]["_braim"], "installed by `braim init` for PreToolUse");
        // The other four take no matcher.
        assert!(v["hooks"]["Stop"][0].get("matcher").is_none());

        // Idempotent on an exact match.
        let second = install_hooks(&settings, true).unwrap();
        assert!(second.iter().all(|c| matches!(c, Change::AlreadyPresent(_))));
        let v2: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v2["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);

        // Negative control: a hand-edited entry missing the guard is not
        // "already present", so the guarded line is added beside it.
        let weak = temp("route_gate_weak");
        let weak_settings = weak.join("settings.json");
        fs::write(
            &weak_settings,
            serde_json::to_string_pretty(&json!({
                "hooks": {"PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "braim route-gate"}]}
                ]}
            }))
            .unwrap(),
        )
        .unwrap();
        let changes = install_hooks(&weak_settings, true).unwrap();
        assert!(changes.iter().any(|c| c == &Change::Added(format!("PreToolUse ({})", ROUTE_GATE_COMMAND))));
        let v3: Value = serde_json::from_str(&fs::read_to_string(&weak_settings).unwrap()).unwrap();
        assert_eq!(v3["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);

        // Without the flag nothing is installed under PreToolUse.
        let off = temp("route_gate_off");
        let off_settings = off.join("settings.json");
        let changes = install_hooks(&off_settings, false).unwrap();
        assert_eq!(changes.len(), 4);
        let v4: Value = serde_json::from_str(&fs::read_to_string(&off_settings).unwrap()).unwrap();
        assert!(v4["hooks"].get("PreToolUse").is_none());
    }

    #[test]
    fn health_verdict_requires_ok_and_a_nonzero_index() {
        use std::process::{Command, Stdio};
        let run = |script: &str| {
            Command::new("sh").arg("-c").arg(script).stdout(Stdio::piped()).stderr(Stdio::piped())
                .output().unwrap()
        };
        assert_eq!(
            health_verdict(&run("printf 'indexed_files: 3\\nmissing_files: 0 (0.0%%)\\nunparsed_files: 0\\nverdict: OK\\n'")),
            Ok(())
        );
        assert_eq!(
            health_verdict(&run("printf 'indexed_files: 0\\nmissing_files: 0 (0.0%%)\\nunparsed_files: 0\\nverdict: OK\\n'")),
            Err(HEALTH_NO_INDEX.to_string())
        );
        assert_eq!(
            health_verdict(&run("printf 'indexed_files: 3\\nverdict: DEGRADED\\n'")),
            Err("verdict-not-ok".to_string())
        );
        assert_eq!(health_verdict(&run("exit 1")), Err("rctags-exit-1".to_string()));
    }
}
