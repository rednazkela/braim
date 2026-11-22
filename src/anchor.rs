//! Symbol anchors: `code:<file>:<symbol>` citations resolved through rctags.
//!
//! A line-number anchor (`code:src/graph.rs:33`) rots every time the file
//! grows above it (braim ID:277/285/333/461). A symbol anchor names the
//! function instead; `rctags query <symbol> --exact` returns its CURRENT lines,
//! so moved code is silently current and only deleted or renamed code is a
//! finding. This module is the single place the shape is defined — the two
//! older parsers that key a source on its artifact path (src/dream.rs
//! `primary_path`, src/graph.rs `import_state`) already tolerate it and point
//! here.
//!
//! Resolution is STRICT: the hit whose `file` string-equals `<file>` wins,
//! never the first hit and never a canonicalized match. The live index carries
//! absolute-path twins of relative entries (63/417 on 2026-10-08, some naming
//! functions that no longer exist), so an absolute citation would string-equal
//! its own stale twin and report "found" for deleted code. Absolute paths are
//! therefore skipped outright rather than resolved.

use std::path::Path;
use std::process::Command;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SkipReason {
    /// Not a `code:` source at all (doc:, test:, transcript:, ...).
    NotACodeSource,
    /// `<file>` starts with `/`: would string-equal a stale absolute twin.
    AbsolutePath,
    /// No `:` after the file, so nothing names a symbol.
    FileOnly,
    /// The part after the last `:` is not an identifier: a line number, a
    /// line range, the old `code:rctags:<file>#<symbol>` shape, or a
    /// composite citation carrying prose.
    NotASymbolAnchor,
}

#[derive(Debug, PartialEq)]
pub enum Resolution {
    Found(u32, u32),
    NotFound,
    Skipped(SkipReason),
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Pure precondition filter: `code:src/graph.rs:from_verification_status` →
/// `("src/graph.rs", "from_verification_status")`; anything else names why it
/// is not a symbol anchor and is never sent to rctags.
pub fn parse_symbol_anchor(source: &str) -> Result<(String, String), SkipReason> {
    let rest = source.strip_prefix("code:").ok_or(SkipReason::NotACodeSource)?;
    if rest.starts_with('/') {
        return Err(SkipReason::AbsolutePath);
    }
    if rest.chars().any(char::is_whitespace) || rest.contains('(') {
        return Err(SkipReason::NotASymbolAnchor);
    }
    let (file, symbol) = rest.rsplit_once(':').ok_or(SkipReason::FileOnly)?;
    if file.is_empty() || !is_identifier(symbol) {
        return Err(SkipReason::NotASymbolAnchor);
    }
    Ok((file.to_string(), symbol.to_string()))
}

/// Ask rctags (run from `root`, so `file` compares against the root-relative
/// paths it stores) for `symbol` and keep only the hit at exactly `file`.
/// `Err` means the tool itself failed (not installed, bad output); callers
/// treat that as "not checked", never as rot.
pub fn resolve_symbol_anchor(
    rctags: &Path,
    root: &Path,
    file: &str,
    symbol: &str,
) -> Result<Resolution, String> {
    let out = Command::new(rctags)
        .args(["query", symbol, "--exact"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("rctags query failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "rctags query exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let hits: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("rctags query printed non-JSON: {}", e))?;
    for hit in &hits {
        if hit.get("name").and_then(|v| v.as_str()) != Some(symbol) {
            continue;
        }
        if hit.get("file").and_then(|v| v.as_str()) != Some(file) {
            continue;
        }
        let start = hit.get("line_start").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let end = hit.get("line_end").and_then(|v| v.as_u64()).unwrap_or(start as u64) as u32;
        return Ok(Resolution::Found(start, end));
    }
    Ok(Resolution::NotFound)
}

/// Parse then resolve one source string; a precondition miss is `Skipped`.
pub fn resolve_source(rctags: &Path, root: &Path, source: &str) -> Result<Resolution, String> {
    match parse_symbol_anchor(source) {
        Ok((file, symbol)) => resolve_symbol_anchor(rctags, root, &file, &symbol),
        Err(reason) => Ok(Resolution::Skipped(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_symbol_shape_parses_and_every_other_live_shape_is_skipped() {
        assert_eq!(
            parse_symbol_anchor("code:src/graph.rs:from_verification_status"),
            Ok(("src/graph.rs".to_string(), "from_verification_status".to_string()))
        );
        // Real live citations, each a shape the resolver must never touch.
        assert_eq!(parse_symbol_anchor("code:src/graph.rs:1300"), Err(SkipReason::NotASymbolAnchor)); // ID:151
        assert_eq!(parse_symbol_anchor("code:src/graph.rs:2952-2975"), Err(SkipReason::NotASymbolAnchor)); // ID:275
        assert_eq!(parse_symbol_anchor("code:src/main.rs:945-955"), Err(SkipReason::NotASymbolAnchor)); // ID:326
        assert_eq!(
            parse_symbol_anchor("code:/home/magnimus/rctags/src/indexer.rs:reindex_diff_opts"),
            Err(SkipReason::AbsolutePath)
        ); // ID:664
        assert_eq!(
            parse_symbol_anchor("code:/home/magnimus/braim/hooks/braim_stop_gate.sh"),
            Err(SkipReason::AbsolutePath)
        ); // ID:621
        assert_eq!(
            parse_symbol_anchor("code:rctags:src/graph.rs#from_verification_status"),
            Err(SkipReason::NotASymbolAnchor)
        ); // ID:622, old shape
        assert_eq!(parse_symbol_anchor("code:src/dream.rs"), Err(SkipReason::FileOnly)); // ID:257
        assert_eq!(
            parse_symbol_anchor("code:src (grep BRAIM_DATA_DIR returns nothing)"),
            Err(SkipReason::NotASymbolAnchor)
        ); // ID:527
        assert_eq!(parse_symbol_anchor("doc:spec.md:12"), Err(SkipReason::NotACodeSource));
    }

    /// Hermetic fixture reproducing the measured duplicate shape: the same
    /// symbol at a relative path and at a stale absolute twin.
    fn fixture() -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("braim_anchor_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("repo");
        std::fs::create_dir_all(root.join(".rctags")).unwrap();
        std::fs::write(
            root.join(".rctags").join("index.json"),
            concat!(
                "[\n",
                "{\"name\":\"from_verification_status\",\"file\":\"/home/magnimus/braim/src/graph.rs\",\"line_start\":30,\"line_end\":38,\"type\":\"function\"},\n",
                "{\"name\":\"from_verification_status\",\"file\":\"src/graph.rs\",\"line_start\":141,\"line_end\":150,\"type\":\"function\"},\n",
                "{\"name\":\"a_deleted_symbol_name\",\"file\":\"src/other.rs\",\"line_start\":1,\"line_end\":2,\"type\":\"function\"},\n",
                "{\"name\":\"run\",\"file\":\"src/main.rs\",\"line_start\":1,\"line_end\":2,\"type\":\"function\"},\n",
                "{\"name\":\"run\",\"file\":\"src/stop_gate.rs\",\"line_start\":1,\"line_end\":2,\"type\":\"function\"}\n",
                "]\n"
            ),
        )
        .unwrap();
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let stub = bin.join("rctags");
        std::fs::write(
            &stub,
            "#!/bin/sh\n[ \"$1\" = query ] || exit 1\nprintf '['\ngrep -F \"\\\"name\\\":\\\"$2\\\",\" .rctags/index.json | sed 's/,$//' | paste -sd, -\nprintf ']'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (root, stub)
    }

    #[test]
    fn resolution_keeps_only_the_hit_whose_file_matches_exactly() {
        let (root, stub) = fixture();
        assert_eq!(
            resolve_symbol_anchor(&stub, &root, "src/graph.rs", "from_verification_status").unwrap(),
            Resolution::Found(141, 150),
            "the relative hit wins; the absolute twin at 30-38 is never taken"
        );
        assert_eq!(
            resolve_symbol_anchor(&stub, &root, "src/graph.rs", "a_deleted_symbol_name").unwrap(),
            Resolution::NotFound,
            "a same-named symbol in another file does not count"
        );
        assert_eq!(
            resolve_symbol_anchor(&stub, &root, "src/graph.rs", "run").unwrap(),
            Resolution::NotFound,
            "hits in src/main.rs and src/stop_gate.rs do not satisfy src/graph.rs"
        );
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }
}
