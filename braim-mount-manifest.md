# braim Mount Manifest

The mount manifest is braim's **lockfile**: the consume-side record of which external-domain snapshots a graph is pinned to. It is the first concrete artifact of the federated-knowledge design (`braim_sharing` domain).

@[The manifest records, per external domain, the upstream source, the pinned snapshot version, and an integrity hash to detect drift] source: code:src/manifest.rs:MountEntry

## Why it exists

#[A federated graph that resolves cross-domain references live is non-reproducible; pinning a snapshot and refreshing explicitly is the proven choice across git submodules, dependency lockfiles, and Linked Data named-graph preloading] based_on: @[braim ID:199 decision] + @[braim ID:194/195/196 proven patterns]

Design provenance (all in the `braim_sharing` domain):

| Decision | What it fixes here |
|---|---|
| ID:182 — identity = `domain:uuid` | the manifest keys mounts by `domain`; the uuid half identifies nodes within the mounted pack |
| ID:199 — pinned snapshot + explicit refresh | each entry pins one snapshot; advancing it is a deliberate re-pin, never live |
| ID:205 — increment = delta vs snapshot | the pinned snapshot is the base the increments apply against (OSTRICH/lockfile model) |
| ID:192 — human-triggered processing | a refresh is an explicit human action, recorded by `pinned_at` |
| ID:194 — git submodule gitlink | `source` ≈ `.gitmodules` url, `track` ≈ optional branch, `pinned_version` ≈ pinned commit |

## Format

One manifest per consuming graph, JSON:

```json
{
  "manifest_version": 1,
  "mounts": [
    {
      "domain": "billing",
      "source": "git@github.com:org/billing.braim.git",
      "pinned_version": 9,
      "integrity": "sha256:abc123…",
      "track": "main",
      "pinned_at": "2026-06-30T10:00:00Z"
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `manifest_version` | schema version; this build supports `1` |
| `domain` | local domain name the mount provides (alias half of `domain:uuid`) |
| `source` | upstream location the pack is fetched from (git URL or path) |
| `pinned_version` | the braim `version save` number frozen at pin time — the snapshot |
| `integrity` | `sha256:<hex>` content hash of the pinned snapshot; recorded, not computed by braim |
| `track` | optional upstream ref a refresh re-pins against; absent = only explicit version bumps |
| `pinned_at` | RFC3339 timestamp the pin was set — provenance for the refresh |

@[Integrity is recorded and compared, never computed by braim — exactly as git records a commit SHA produced by the upstream] source: code:src/manifest.rs module-doc

## Semantics

- **Resolve**: a cross-domain reference `otherdomain:uuid` is resolved against the snapshot named by that domain's `pinned_version`. Resolution is read-only (ID:163) — a consumer never mutates the upstream, so there is no cross-domain cascade.
- **Drift check** (`check_drift`): compare the recorded `integrity` against the integrity of the snapshot actually present.

  | Result | Meaning | Action |
  |---|---|---|
  | `Pinned` | recorded == actual | safe to resolve |
  | `Drifted` | recorded != actual | halt resolution; upstream moved without a refresh |
  | `Unmounted` | no entry for the domain | dangling cross-domain reference |

- **Refresh**: advancing a pin reads the latest upstream snapshot (optionally following `track`), updates `pinned_version` + `integrity` + `pinned_at`. Explicit and human-triggered (ID:192). This is the only way a mounted domain's content changes from the consumer's view.

## Validation rules

@[validate rejects unsupported manifest_version, empty domain/source/pinned_at, malformed integrity, and duplicate-domain mounts] source: code:src/manifest.rs:validate

- `manifest_version` must equal the supported version.
- `domain`, `source`, `pinned_at` must be non-empty.
- `integrity` must match `sha256:<non-empty lowercase hex>`.
- a `domain` is pinned **exactly once** — duplicates are rejected.

## Tests

@[7 unit tests cover parse+validate, JSON round-trip, version/duplicate/integrity/empty-field rejection, and the three drift states] source: test:src/manifest.rs:tests

Run: `cargo test --release manifest`.

## Not yet built (deliberately out of scope here)

- The mount **resolver** that follows `domain:uuid` into a pinned snapshot — depends on extracting the engine into a lib crate (ID:176).
- Integrity **computation** of a snapshot — delegated; needs a content-hash step at pin/refresh time.
- CLI surface (`braim mount add/refresh/status`) — the module is wired into the binary but no command is exposed yet.
