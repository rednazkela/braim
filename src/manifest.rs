//! Mount manifest — braim's lockfile for federated, cross-domain knowledge.
//!
//! Design provenance (braim_sharing domain): the manifest is the consume-side
//! pin record decided in ID:199 (resolve via pinned snapshot + explicit refresh),
//! modelled on git submodule gitlinks (ID:194) and dependency lockfiles (ID:195).
//! It records, per external domain, WHERE the pack comes from, WHICH snapshot is
//! pinned, and an integrity hash to detect drift. Increments (ID:205) are deltas
//! against the pinned snapshot; advancing a pin is an explicit, human-triggered
//! refresh (ID:192), never live per-query.
//!
//! Integrity computation is DELEGATED, exactly as git computes a commit SHA and
//! the .gitmodules/index only records it: braim stores and compares the hash, it
//! does not hash here. That keeps the base binary dependency-free (no crypto crate)
//! and matches the proven submodule/lockfile split between "record the pin" and
//! "the upstream produced the version".

// Not yet wired to a CLI command — this is the shipped format + validation that
// the mount resolver and server companion (ID:176) will build on.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Current manifest schema version. Bumped only on a breaking format change.
pub const MANIFEST_VERSION: u32 = 1;

/// One mounted external domain: a single pinned reference into another graph.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MountEntry {
    /// Local domain name this mount provides (the alias half of `domain:uuid`, ID:182).
    pub domain: String,
    /// Upstream location the pack is fetched from (git URL or path). Analogous to
    /// the `url` in `.gitmodules`.
    pub source: String,
    /// The pinned snapshot: the braim `version save` number frozen at pin time.
    /// Analogous to a git submodule's pinned commit SHA.
    pub pinned_version: u32,
    /// Content hash of the pinned snapshot, formatted `sha256:<hex>`. Recorded, not
    /// computed here; compared against the actual snapshot hash to detect drift.
    pub integrity: String,
    /// Optional upstream ref a refresh re-pins against (e.g. `main`). Absence means
    /// the pin is only ever advanced by an explicit version bump. Mirrors the
    /// optional `branch` in `.gitmodules`.
    #[serde(default)]
    pub track: Option<String>,
    /// RFC3339 timestamp recording when this pin was set — provenance for the refresh.
    pub pinned_at: String,
}

/// The mount manifest: braim's lockfile. One per consuming graph.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MountManifest {
    pub manifest_version: u32,
    pub mounts: Vec<MountEntry>,
}

/// Result of checking a mount's recorded pin against the snapshot actually present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftStatus {
    /// Recorded integrity matches the actual snapshot — safe to resolve against.
    Pinned,
    /// Recorded integrity differs from the actual snapshot — the upstream moved
    /// without an explicit refresh; resolution must halt until re-pinned.
    Drifted,
    /// No mount declares this domain — a dangling cross-domain reference.
    Unmounted,
}

impl MountManifest {
    /// An empty manifest at the current schema version.
    pub fn empty() -> Self {
        MountManifest { manifest_version: MANIFEST_VERSION, mounts: Vec::new() }
    }

    /// Parse a manifest from JSON. Does not validate semantics — call `validate`.
    pub fn parse(json: &str) -> Result<MountManifest, String> {
        serde_json::from_str(json).map_err(|e| format!("Error parsing mount manifest: {}", e))
    }

    /// Serialize to pretty JSON (the on-disk form).
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("Error serializing mount manifest: {}", e))
    }

    /// Structural + semantic validation. Returns the first violation found.
    pub fn validate(&self) -> Result<(), String> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(format!(
                "Error: unsupported manifest_version {} (this build supports {})",
                self.manifest_version, MANIFEST_VERSION
            ));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for (i, m) in self.mounts.iter().enumerate() {
            if m.domain.trim().is_empty() {
                return Err(format!("Error: mount[{}] has an empty domain", i));
            }
            if m.source.trim().is_empty() {
                return Err(format!("Error: mount[{}] '{}' has an empty source", i, m.domain));
            }
            if m.pinned_at.trim().is_empty() {
                return Err(format!("Error: mount[{}] '{}' is missing pinned_at", i, m.domain));
            }
            validate_integrity(&m.integrity)
                .map_err(|e| format!("Error: mount[{}] '{}' {}", i, m.domain, e))?;
            if !seen.insert(m.domain.as_str()) {
                return Err(format!(
                    "Error: duplicate mount for domain '{}' — a domain is pinned exactly once",
                    m.domain
                ));
            }
        }
        Ok(())
    }

    /// Find the mount entry for a domain, if any.
    pub fn find(&self, domain: &str) -> Option<&MountEntry> {
        self.mounts.iter().find(|m| m.domain == domain)
    }

    /// Compare the recorded pin for `domain` against the integrity of the snapshot
    /// actually present. This is the lockfile-integrity + dangling-reference check.
    pub fn check_drift(&self, domain: &str, actual_integrity: &str) -> DriftStatus {
        match self.find(domain) {
            None => DriftStatus::Unmounted,
            Some(m) if m.integrity == actual_integrity => DriftStatus::Pinned,
            Some(_) => DriftStatus::Drifted,
        }
    }
}

/// An integrity string must be `sha256:<non-empty lowercase hex>`.
fn validate_integrity(s: &str) -> Result<(), String> {
    let hex = s.strip_prefix("sha256:")
        .ok_or_else(|| "integrity must be formatted 'sha256:<hex>'".to_string())?;
    if hex.is_empty() {
        return Err("integrity hash is empty".to_string());
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("integrity hash contains non-hex characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> &'static str {
        r#"{
          "manifest_version": 1,
          "mounts": [
            {
              "domain": "billing",
              "source": "git@github.com:org/billing.braim.git",
              "pinned_version": 9,
              "integrity": "sha256:abc123",
              "track": "main",
              "pinned_at": "2026-06-30T10:00:00Z"
            },
            {
              "domain": "payments",
              "source": "/srv/braim/payments",
              "pinned_version": 3,
              "integrity": "sha256:deadbeef",
              "pinned_at": "2026-06-30T10:05:00Z"
            }
          ]
        }"#
    }

    #[test]
    fn parses_and_validates_a_good_manifest() {
        let m = MountManifest::parse(sample_json()).unwrap();
        m.validate().unwrap();
        assert_eq!(m.mounts.len(), 2);
        assert_eq!(m.find("billing").unwrap().pinned_version, 9);
        assert_eq!(m.find("billing").unwrap().track.as_deref(), Some("main"));
        // track is optional and absent here
        assert!(m.find("payments").unwrap().track.is_none());
    }

    #[test]
    fn round_trips_through_json() {
        let m = MountManifest::parse(sample_json()).unwrap();
        let json = m.to_json().unwrap();
        let back = MountManifest::parse(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut m = MountManifest::parse(sample_json()).unwrap();
        m.manifest_version = 2;
        assert!(m.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_domain() {
        let mut m = MountManifest::parse(sample_json()).unwrap();
        m.mounts[1].domain = "billing".to_string();
        let err = m.validate().unwrap_err();
        assert!(err.contains("duplicate mount for domain 'billing'"), "got: {}", err);
    }

    #[test]
    fn rejects_bad_integrity_format() {
        let mut m = MountManifest::empty();
        m.mounts.push(MountEntry {
            domain: "x".into(),
            source: "/p".into(),
            pinned_version: 1,
            integrity: "md5:abc".into(), // wrong algorithm prefix
            track: None,
            pinned_at: "2026-06-30T00:00:00Z".into(),
        });
        assert!(m.validate().is_err());
        m.mounts[0].integrity = "sha256:xyz".into(); // non-hex
        assert!(m.validate().is_err());
        m.mounts[0].integrity = "sha256:".into(); // empty hash
        assert!(m.validate().is_err());
        m.mounts[0].integrity = "sha256:00ff".into(); // valid
        m.validate().unwrap();
    }

    #[test]
    fn rejects_empty_domain_or_source_or_timestamp() {
        let mut m = MountManifest::empty();
        m.mounts.push(MountEntry {
            domain: "".into(),
            source: "/p".into(),
            pinned_version: 1,
            integrity: "sha256:00".into(),
            track: None,
            pinned_at: "2026-06-30T00:00:00Z".into(),
        });
        assert!(m.validate().is_err());
        m.mounts[0].domain = "d".into();
        m.mounts[0].source = "  ".into();
        assert!(m.validate().is_err());
        m.mounts[0].source = "/p".into();
        m.mounts[0].pinned_at = "".into();
        assert!(m.validate().is_err());
    }

    #[test]
    fn drift_detection_covers_pinned_drifted_and_unmounted() {
        let m = MountManifest::parse(sample_json()).unwrap();
        // recorded integrity matches what's actually present
        assert_eq!(m.check_drift("billing", "sha256:abc123"), DriftStatus::Pinned);
        // upstream moved without an explicit refresh
        assert_eq!(m.check_drift("billing", "sha256:99999"), DriftStatus::Drifted);
        // no mount declares this domain — dangling reference
        assert_eq!(m.check_drift("unknown", "sha256:whatever"), DriftStatus::Unmounted);
    }
}
