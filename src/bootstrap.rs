//! Team bootstrap: give a teammate's agent the working setup in one command.
//!
//! Adoption is single-player first (braim ID:223). Teammates start with no
//! graphs, so day-one value cannot be consuming shared knowledge — there is
//! none. What `init --team` delivers is the setup that already works solo: a
//! local graph plus the policy hooks that automate the evidence discipline, so
//! their agent behaves correctly from the first turn. Sharing layers on once
//! several graphs exist.
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
        "compaction" | "precompact" => Ok(wrap_precompact(COMPACTION)),
        "traits" | "memory" => Ok(TRAITS.to_string()),
        other => Err(format!(
            "Error: unknown policy '{}' (expected perturn, compaction, or traits)",
            other
        )),
    }
}

/// The compaction rule ships as prose; a PreCompact hook wants the same
/// `additionalContext` envelope the per-turn payload already uses.
fn wrap_precompact(body: &str) -> String {
    let payload = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreCompact",
            "additionalContext": body
        }
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| body.to_string())
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

/// True when some hook of `event` already invokes `braim policy`, so re-running
/// the bootstrap is idempotent instead of stacking duplicate injections.
fn has_braim_hook(settings: &Value, event: &str) -> bool {
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
                                .map(|c| c.contains("braim policy"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn hook_entry(event: &str, policy: &str, status: &str) -> Value {
    json!({
        "hooks": [{
            "type": "command",
            // No shell tools, no absolute paths: the binary the teammate already
            // installed emits its own policy, so this line is identical on every
            // platform and every machine.
            "command": format!("braim policy {}", policy),
            "timeout": 60,
            "statusMessage": status
        }],
        // Recorded so a human reading settings.json can tell which tool owns the
        // entry and re-run its bootstrap.
        "_braim": format!("installed by `braim init --team` for {}", event)
    })
}

/// Merge braim's hooks into an existing settings file without disturbing
/// anything else in it. Never rewrites or removes a hook it did not add.
pub fn install_hooks(settings_path: &Path) -> Result<Vec<Change>, String> {
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

    let wanted = [
        ("UserPromptSubmit", "perturn", "Injecting per-turn braim logging discipline"),
        ("PreCompact", "compaction", "Injecting braim compaction discipline"),
    ];

    let mut changes = Vec::new();
    for (event, policy, status) in wanted {
        if has_braim_hook(&settings, event) {
            changes.push(Change::AlreadyPresent(event.to_string()));
            continue;
        }
        let entry = hook_entry(event, policy, status);
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
        changes.push(Change::Added(event.to_string()));
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
    fn install_creates_both_hooks_and_is_idempotent() {
        let dir = temp("install");
        let settings = dir.join("settings.json");

        let first = install_hooks(&settings).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|c| matches!(c, Change::Added(_))));

        let v: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"], "braim policy perturn");
        assert_eq!(v["hooks"]["PreCompact"][0]["hooks"][0]["command"], "braim policy compaction");

        // Re-running must not stack duplicates.
        let second = install_hooks(&settings).unwrap();
        assert!(second.iter().all(|c| matches!(c, Change::AlreadyPresent(_))));
        let v2: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v2["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);
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

        install_hooks(&settings).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();

        assert_eq!(v["model"], "opus", "unrelated settings survive");
        assert_eq!(v["permissions"]["allow"][0], "WebSearch");
        let ups = v["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(ups.len(), 2, "the existing hook is kept and braim's is appended");
        assert_eq!(ups[0]["hooks"][0]["command"], "echo mine", "foreign hook untouched");
        assert_eq!(ups[1]["hooks"][0]["command"], "braim policy perturn");
    }

    #[test]
    fn install_refuses_to_clobber_unparseable_settings() {
        let dir = temp("broken");
        let settings = dir.join("settings.json");
        fs::write(&settings, "{ this is not json").unwrap();
        let err = install_hooks(&settings).unwrap_err();
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
}
