//! Stop hook: structurally enforces the per-turn braim declaration.
//!
//! Ported from `hooks/braim_stop_gate.sh` (built up over commits edeafbc and
//! 935b6f9) so the Stop hook is wired the SAME WAY `braim init` already wires
//! perturn/traits/compaction: a bare `braim` command with no absolute path
//! and no external interpreter (the bash version needed bash + jq + coreutils
//! and a path to this repo's checkout — exactly the anti-pattern
//! src/bootstrap.rs's own doc comment says emitting from the binary
//! eliminates). This module is that emission for the Stop event: a check
//! instead of a static payload, but the same "no path, no shell tool,
//! version-locked to the binary enforcing it" shape.
//!
//! Design unchanged from the bash version: mandatory-declaration, not
//! detect-and-verify. A dumb check cannot judge whether a turn was
//! substantive enough to need a braim write — that is a call only the model
//! can make. So every reply must end with a small structured tag:
//!
//!   [braim: none]
//!   [braim: logged 12,13]
//!   [braim: logged 12,13 | audit: clean]
//!   [braim: logged 12,13 | audit: 2 flagged 66,71]
//!   [braim: none | audit: skipped: no claim leaned on a stored node]
//!
//! This hook verifies only what is mechanically checkable: the tag is
//! well-formed, logged ids resolve and are new this turn, a turn logging 2+
//! new nodes discloses how they connect in the reply body, and the audit
//! clause's claimed count matches the actual rise in pending audit-rot items
//! (`braim dream review`). See braim-audit-rot-loop.md for the audit-clause
//! rationale.

use crate::dream;
use crate::graph::Braim;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".claude").join("braim_stop_state")
}

struct State {
    last_seen_max_id: u32,
    last_seen_pending: i64,
    last_fired_pending: i64,
    /// Line count of `.rctags/bypass.log` at the last tool-bypass filing.
    /// Advances only when an item is filed, so sub-threshold turns accumulate.
    last_seen_bypass: u64,
}

fn load_state(path: &Path) -> State {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    State {
        last_seen_max_id: v.get("last_seen_max_id").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        last_seen_pending: v.get("last_seen_pending").and_then(|x| x.as_i64()).unwrap_or(0),
        last_fired_pending: v.get("last_fired_pending").and_then(|x| x.as_i64()).unwrap_or(0),
        last_seen_bypass: v.get("last_seen_bypass").and_then(|x| x.as_u64()).unwrap_or(0),
    }
}

fn save_state(path: &Path, s: &State) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!(
        "{{\"last_seen_max_id\": {}, \"last_seen_pending\": {}, \"last_fired_pending\": {}, \"last_seen_bypass\": {}}}",
        s.last_seen_max_id, s.last_seen_pending, s.last_fired_pending, s.last_seen_bypass
    );
    let _ = std::fs::write(path, body);
}

/// Outcome of reading the bypass ledger this turn.
struct BypassSweep {
    /// The watermark to persist.
    last_seen_bypass: u64,
    /// True when one tool-bypass item was filed this turn.
    filed: bool,
}

/// Layer 2 of the route enforcement (spec L51-L59): count bypass.log lines
/// above the persisted watermark and, once the ACCUMULATED delta reaches the
/// shared threshold, file ONE tool-bypass item listing the commands. The
/// watermark moves only on a filing (like `last_fired_pending`), so a steady
/// drip of sub-threshold bypasses still reaches a fire; a shorter log than
/// the watermark (rotated/truncated) resets it without filing. reviews.json
/// is written here without the graph's write lock, as the state file already
/// is: this process is dispatched read-only and the ledger is not current.json.
fn sweep_bypass_log(
    data_dir: &Path,
    hook_cwd: Option<&str>,
    watermark: u64,
    threshold: u64,
) -> Option<BypassSweep> {
    let cwd = hook_cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let root = crate::route_gate::rctags_root_for(&cwd)?;
    let text = std::fs::read_to_string(root.join(".rctags").join("bypass.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let n = lines.len() as u64;
    if n < watermark {
        return Some(BypassSweep { last_seen_bypass: n, filed: false });
    }
    let delta = n - watermark;
    if delta == 0 || delta < threshold {
        return Some(BypassSweep { last_seen_bypass: watermark, filed: false });
    }
    let commands: Vec<String> = lines[watermark as usize..]
        .iter()
        .map(|l| l.splitn(3, '\t').nth(2).unwrap_or(l).to_string())
        .collect();
    let note = format!(
        "{} grep-family bypasses recorded in {} since the last filing (lines {}-{} of .rctags/bypass.log). Classify each as licensed (comment/string/unindexed search — sign off) or defection (a symbol lookup grep'd — wire to the node its output fed): {}",
        delta,
        root.display(),
        watermark + 1,
        n,
        commands.join(" ⏎ ")
    );
    match dream::flag(data_dir, "tool-bypass", &note, Vec::new()) {
        Ok(_) => Some(BypassSweep { last_seen_bypass: n, filed: true }),
        Err(e) => {
            eprintln!("braim stop-gate: could not file the tool-bypass item: {}", e);
            Some(BypassSweep { last_seen_bypass: watermark, filed: false })
        }
    }
}

/// Every maximal run of ASCII digits in `s`, in order. Doubles as the
/// "standalone number" test the bash version did with
/// `(^|[^0-9])$id([^0-9]|$)`: a digit run bounded by non-digits on both sides
/// is exactly what this extracts.
fn extract_ids(s: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            if let Ok(n) = cur.parse() {
                ids.push(n);
            }
            cur.clear();
        }
    }
    if !cur.is_empty() {
        if let Ok(n) = cur.parse() {
            ids.push(n);
        }
    }
    ids
}

/// Find the last well-formed `[braim: ...]` tag in `msg`. Well-formed means
/// that right after `braim:` and optional whitespace, the tag reads `none`
/// or `logged` followed by whitespace and a digit — the same grammar the
/// original bash regex `\[braim:\s*(none|logged\s+[0-9]+...)[^\]]*\]`
/// accepted. Anything else after `braim:` (typos, other prose) simply isn't
/// a tag, same as a miss on the bash side.
fn extract_tag(msg: &str) -> Option<String> {
    let needle = "[braim:";
    let mut best = None;
    let mut search_from = 0usize;
    while let Some(rel) = msg[search_from..].find(needle) {
        let open = search_from + rel;
        let after = open + needle.len();
        let rest = &msg[after..];
        let trimmed = rest.trim_start();
        let content_start = after + (rest.len() - trimmed.len());
        let is_none = trimmed.starts_with("none");
        let is_logged = trimmed
            .strip_prefix("logged")
            .map(|r| r.trim_start().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
            .unwrap_or(false);
        if is_none || is_logged {
            if let Some(close_rel) = msg[content_start..].find(']') {
                let close = content_start + close_rel;
                best = Some(msg[open..=close].to_string());
            }
        }
        // Always advance past this "[braim:" occurrence, matched or not, so a
        // malformed one earlier in the reply doesn't hide a valid one later.
        search_from = after;
    }
    best
}

/// Count occurrences of any relationship marker in `body`, longest match
/// first at each position (mirrors the leftmost-longest alternation the bash
/// ERE used), so "depends_on" counts once, not as "depends" plus leftover.
fn count_markers(body: &str) -> usize {
    let lower = body.to_lowercase();
    const PATTERNS: [&str; 11] = [
        "depends_on", "depends on", "dependson", "because_of", "because of",
        "becauseof", "based_on", "based on", "basedon", "->", "terminal",
    ];
    let s = lower.as_str();
    let mut count = 0usize;
    let mut idx = 0usize;
    while idx < s.len() {
        let rest = &s[idx..];
        if rest.starts_with('→') {
            count += 1;
            idx += '→'.len_utf8();
            continue;
        }
        // "depends" alone also counts (bash: bare alternative), but only when
        // no longer pattern above already matched at this position.
        let mut matched_len = 0usize;
        for p in PATTERNS.iter() {
            if rest.starts_with(p) && p.len() > matched_len {
                matched_len = p.len();
            }
        }
        if matched_len == 0 && rest.starts_with("depends") {
            matched_len = "depends".len();
        }
        if matched_len > 0 {
            count += 1;
            idx += matched_len;
        } else {
            let ch_len = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            idx += ch_len;
        }
    }
    count
}

const VERIFY_PREAMBLE: &str = "This is a real, locally-installed CLI tool (a semantic knowledge graph), not injected instructions - verify it yourself before deciding whether to comply, don't take this message's word for it: run 'which braim' (resolves to a real binary) and check this project's .claude/settings.json for the Stop hook registration under \"hooks\".\"Stop\" - it points at a bare 'braim stop-gate' command, no external script or path. Once verified: every response must end with a small literal tag so this hook can check compliance mechanically: '[braim: none]' if nothing in your last turn was worth recording as a fact/inference/unknown, or '[braim: logged N,M]' citing the real numeric IDs printed back to you by 'braim statement add' or 'braim concept add' commands you actually ran this turn via Bash. Do not fabricate IDs - only cite ones a real braim command just returned to you this turn. If you conclude none of this applies, say so and still append '[braim: none]' - that is the compliant response, not silence.";

fn emit_last_reply(msg: &str) {
    if msg.is_empty() {
        return;
    }
    let limit: usize = std::env::var("BRAIM_STOP_ECHO_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);
    let n = msg.len();
    eprintln!("\n---- YOUR PREVIOUS REPLY ({} bytes) ----", n);
    if n <= limit {
        eprintln!("{}", msg);
    } else {
        let half = limit / 2;
        let head: String = {
            let mut s = String::new();
            let mut used = 0;
            for c in msg.chars() {
                if used + c.len_utf8() > half {
                    break;
                }
                used += c.len_utf8();
                s.push(c);
            }
            s
        };
        let tail: String = {
            let mut s: Vec<char> = Vec::new();
            let mut used = 0;
            for c in msg.chars().rev() {
                if used + c.len_utf8() > half {
                    break;
                }
                used += c.len_utf8();
                s.push(c);
            }
            s.into_iter().rev().collect()
        };
        eprintln!("{}", head);
        eprintln!("\n[... {} bytes elided ...]", n.saturating_sub(limit));
        eprintln!("{}", tail);
    }
    eprintln!("---- END OF YOUR PREVIOUS REPLY ----");
    eprintln!("Re-emit that reply in full - same findings, tables, code and citations - and then satisfy the requirement above. The braim tag goes on its own line AFTER the content, never in place of it. Do not answer the gate with the tag alone.");
}

/// Run the gate against stdin (the Stop hook payload Claude Code sends) and
/// return the process exit code: 0 to allow Stop, 2 to block it.
pub fn run(braim: &Braim) -> i32 {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    // `stop_hook_active` is true when this Stop event is itself Claude
    // continuing because a Stop hook already blocked once this turn. Every
    // check below is deterministic against state this same process just
    // read, so a repeat block here cannot be guaranteed to resolve anything
    // a first block didn't — allowing unconditionally caps this gate at ONE
    // block per turn, well under Claude Code's own 8-consecutive-block
    // override, and is what actually fixes a real observed lockup: this
    // check was ABSENT from both this port and the original
    // hooks/braim_stop_gate.sh it replaces (neither ever read the field).
    if payload.get("stop_hook_active").and_then(|v| v.as_bool()).unwrap_or(false) {
        return 0;
    }

    let msg = payload
        .get("last_assistant_message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    macro_rules! block {
        ($($arg:tt)*) => {{
            eprintln!($($arg)*);
            emit_last_reply(&msg);
            return 2;
        }};
    }

    let tag = match extract_tag(&msg) {
        Some(t) => t,
        None => block!("{}", VERIFY_PREAMBLE),
    };

    let (logged_part, mut audit_part) = match tag.find('|') {
        Some(idx) => (tag[..idx].to_string(), tag[idx + 1..].to_string()),
        None => (tag.clone(), String::new()),
    };
    if audit_part.ends_with(']') {
        audit_part.pop();
    }
    let audit_part = audit_part.trim().to_string();

    // ID:888: hashing the raw, uncanonicalized path meant ".braim" (relative,
    // cwd-dependent) and an absolute resolved path to the SAME on-disk store
    // hashed to DIFFERENT digests, so state written by one invocation was
    // invisible to the next — six live checks each saw a fresh/default
    // baseline against a store that already had real pending items.
    // canonicalize() resolves symlinks and relative components so the same
    // real directory always hashes the same way regardless of how it was
    // specified; fall back to the raw string only if that somehow fails (it
    // should not, since Braim::new already loaded successfully from this
    // exact directory).
    let store = braim
        .data_dir
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| braim.data_dir.to_string_lossy().to_string());
    let digest = md5::compute(store.as_bytes());
    let statefile = state_dir().join(format!("{:x}.json", digest));
    let state = load_state(&statefile);
    let mut new_max = state.last_seen_max_id;

    let has_logged = logged_part.to_lowercase().contains("logged");
    let edge_ids = if has_logged { extract_ids(&logged_part) } else { Vec::new() };

    if has_logged {
        let mut bad = String::new();
        for &id in &edge_ids {
            if !braim.state.nodes.contains_key(&id) {
                bad.push_str(&format!(" {}(no-such-node)", id));
                continue;
            }
            if id <= state.last_seen_max_id {
                bad.push_str(&format!(" {}(already-seen-not-new-this-turn)", id));
                continue;
            }
            if id > new_max {
                new_max = id;
            }
        }
        if !bad.is_empty() {
            block!(
                "braim tag claims IDs that failed verification:{}. Cite only node IDs you actually created THIS turn via a real braim command, not ones already in the graph.",
                bad
            );
        }
    }

    // Edge disclosure: a turn logging 2+ NEW nodes must say, in the body
    // (never the tag), how they connect. One node has no internal structure
    // to describe, so this is skipped at exactly one.
    if edge_ids.len() >= 2 {
        let body = msg.replace(&tag, "");
        let body_numbers: std::collections::HashSet<u32> = extract_ids(&body).into_iter().collect();
        let missing_ids: String = edge_ids
            .iter()
            .filter(|id| !body_numbers.contains(id))
            .map(|id| format!(" {}", id))
            .collect();
        if !missing_ids.is_empty() {
            block!(
                "The braim tag cites node(s){} that appear nowhere in the reply body. A tag is a receipt, not a description: every node this turn created must be discussed where the reader can see what it says and how it connects, not only counted on the last line.",
                missing_ids
            );
        }
        let markers = count_markers(&body);
        if markers < edge_ids.len() {
            block!(
                "This turn logged {} nodes but the reply body carries only {} relationship marker(s). A subgraph written and not described is structure thrown away. State, for each node, how it attaches: its depends_on with the weights when the split is doing work, its because_of parent and the direction, or the word 'terminal' when it genuinely has no parent. An edge list ('ID:748 -> ID:747') satisfies this, and so does prose that names the relationship. Do NOT put the edges inside the braim tag — it takes exactly two clauses, 'logged' and 'audit'.",
                edge_ids.len(), markers
            );
        }
    }

    // The audit clause.
    if audit_part.is_empty() {
        block!("The braim tag is missing its audit clause. Every turn declares whether it checked its own assertions against the nodes they lean on, in the same tag: '[braim: logged 12,13 | audit: clean]' when the check found nothing wrong, '[braim: ... | audit: N flagged <node ids>]' when it found defects in nodes that were ALREADY in the graph (file each one first with 'braim dream flag \"<what is wrong>\" --kind anchor|reground|independence|unsupported|tool-bypass --nodes <ids>'), or '[braim: ... | audit: skipped: <reason>]' when nothing this turn rested on a stored node. A finding is a defect in something already stored — a self-correction you made before asserting anything is not a finding and must not be counted.");
    }
    let audit_lower = audit_part.to_lowercase();
    if !audit_lower.starts_with("audit:") {
        block!(
            "The braim tag's second clause must start with 'audit:' — got '{}'. Use '| audit: clean', '| audit: N flagged <ids>', or '| audit: skipped: <reason>'.",
            audit_part
        );
    }
    let audit_body = audit_part[6..].trim_start().to_string();
    let audit_body_lower = audit_body.to_lowercase();

    let claimed: i64;
    if audit_body_lower.starts_with("clean") {
        claimed = 0;
    } else if audit_body_lower.starts_with("skipped") {
        if has_logged {
            block!("'audit: skipped' contradicts 'logged' in the same tag: a turn that wrote nodes to the graph did lean on the graph, so there was something to check. Declare 'audit: clean' if the check found nothing wrong, or 'audit: N flagged <ids>' if it did.");
        }
        claimed = 0;
    } else {
        let digits: String = audit_body.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest_trimmed = audit_body[digits.len()..].trim_start();
        if !digits.is_empty() && rest_trimmed.to_lowercase().starts_with("flagged") {
            let claimed_n: i64 = digits.parse().unwrap_or(-1);
            let after_flagged = &rest_trimmed[7.min(rest_trimmed.len())..];
            let flagged_ids = extract_ids(after_flagged);
            if flagged_ids.len() as i64 != claimed_n {
                block!(
                    "The audit clause claims {} finding(s) but lists {} node id(s). Every finding names the already-stored node it is about.",
                    claimed_n, flagged_ids.len()
                );
            }
            let mut abad = String::new();
            for &id in &flagged_ids {
                if !braim.state.nodes.contains_key(&id) {
                    abad.push_str(&format!(" {}(no-such-node)", id));
                    continue;
                }
                if id > state.last_seen_max_id {
                    abad.push_str(&format!(" {}(created-this-turn-not-a-stored-node)", id));
                }
            }
            if !abad.is_empty() {
                block!(
                    "The audit clause flags ids that failed verification:{}. A finding is a defect in a node that was already in the graph before this turn — a correction you made in flight is not a finding, and must not be counted toward rot.",
                    abad
                );
            }
            claimed = claimed_n;
        } else {
            block!(
                "Unrecognised audit clause '{}'. Use 'clean', 'N flagged <ids>', or 'skipped: <reason>'.",
                audit_body
            );
        }
    }

    let pending_now = dream::audit_tally(&braim.data_dir).1 as i64;
    let rise = (pending_now - state.last_seen_pending).max(0);

    // ID:887/889: persist the OBSERVED baseline (measured pending_now, ground
    // truth — never the model's `claimed` value, so this cannot be gamed by
    // understating a finding) before the mismatch check, not after. The old
    // order let a blocked turn's baseline stay frozen forever: block! returns
    // immediately, so save_state on the line below it never ran, and next
    // turn recomputed the identical stale rise regardless of whether the next
    // claim was correct. Persisting first means a wrong CLAIM about this
    // turn's rise still blocks (unchanged enforcement below), but the rise
    // itself is only ever explained once — it cannot be stuck unexplainable
    // by a baseline that never moves.
    save_state(&statefile, &State {
        last_seen_max_id: new_max,
        last_seen_pending: pending_now,
        last_fired_pending: state.last_fired_pending,
        last_seen_bypass: state.last_seen_bypass,
    });

    if claimed != rise {
        block!(
            "The audit clause claims {} finding(s) but the pending audit-rot queue rose by {} this turn (was {}, now {}). File each finding with 'braim dream flag \"<what is wrong>\" --kind anchor|reground|independence|unsupported|tool-bypass --nodes <ids>' before declaring it, and count only what you actually filed.",
            claimed, rise, state.last_seen_pending, pending_now
        );
    }

    let threshold: i64 = std::env::var("BRAIM_AUDIT_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(5);

    // The bypass ledger, read AFTER the claimed-vs-rise check so a gate-filed
    // item is never charged to the agent's claim, and the baseline bumped by
    // one when it files: the item is observed ground truth the agent could
    // not have declared, and without the bump the next turn's 'audit: clean'
    // would be blocked for a rise of 1 the gate itself caused.
    let mut pending_for_threshold = pending_now;
    let mut last_seen_bypass = state.last_seen_bypass;
    let hook_cwd = payload.get("cwd").and_then(|v| v.as_str());
    if let Some(sweep) = sweep_bypass_log(&braim.data_dir, hook_cwd, state.last_seen_bypass, threshold.max(1) as u64) {
        last_seen_bypass = sweep.last_seen_bypass;
        if sweep.filed {
            pending_for_threshold += 1;
        }
        if sweep.filed || last_seen_bypass != state.last_seen_bypass {
            save_state(&statefile, &State {
                last_seen_max_id: new_max,
                last_seen_pending: pending_for_threshold,
                last_fired_pending: state.last_fired_pending,
                last_seen_bypass,
            });
        }
    }
    let pending_now = pending_for_threshold;

    // The threshold: fires only when the count has grown past the last
    // firing, so a session that cannot clear every finding is not blocked
    // forever by the same backlog.
    if pending_now >= threshold && pending_now > state.last_fired_pending {
        save_state(&statefile, &State {
            last_seen_max_id: new_max,
            last_seen_pending: pending_now,
            last_fired_pending: pending_now,
            last_seen_bypass,
        });
        let items = dream::pending(&braim.data_dir, false);
        let ids: Vec<String> = items
            .iter()
            .filter(|i| i.cleared_at.is_none() && dream::AUDIT_KINDS.contains(&i.kind.trim()))
            .map(|i| i.id.to_string())
            .collect();
        let items_str = if ids.is_empty() { "see 'braim dream review'".to_string() } else { ids.join(", ") };
        block!(
            "Pending audit-rot findings have reached {} (threshold {}). Run a dream session now, scoped to review items: {}. The procedure is in the dream skill under 'Clearing the audit-rot queue' (SIC 536) — read each finding, re-ground the node it names against the SOURCE it cites rather than against its own label, and either fix the node or sign the item off with 'braim dream reviewed <id> --note \"<what you decided>\"'. Gathering the evidence may be delegated to a dream-probe agent; deciding may not. The counter falls as items are cleared. Set BRAIM_AUDIT_THRESHOLD to change when this fires.",
            pending_now, threshold, items_str
        );
    }

    0
}
